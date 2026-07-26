# Encrypted database sync

Status: design note, deferred until after the current exams.

This records the sync investigation from 2026-07-26. It is not an
implementation plan for the exam period, and none of the choices below should
make the local app or database depend on a network service.

## Desired product

- No email address, password, profile or conventional server-side account.
- A Mullvad-like numeric account id.
- Account authority and encryption roots are created locally.
- A recovery phrase and QR code add a device or recover an account.
- One remotely provisioned database per paid account.
- All user data is encrypted before it leaves the device.
- The service operator can provision, bill, suspend and delete storage without
  being able to read study data.
- Studying remains fully local and offline. Sync is opportunistic and failure
  never loses the current session.

The numeric id is a locator, not an encryption key. The secret recovery root
must have substantially more entropy and must not be sent to the control
plane.

## Upstream Turso status

The old blocker has changed: Turso now publishes
[`@tursodatabase/sync-wasm`](https://www.npmjs.com/package/@tursodatabase/sync-wasm)
for browser-local databases with push/pull sync. Turso describes the browser
implementation in
[Introducing Turso in the Browser](https://turso.tech/blog/introducing-turso-in-the-browser)
and the sync API in
[Turso Sync usage](https://docs.turso.tech/sync/usage).

That package is not a small switch for this application:

- It is a second database engine beside Idiosepius's Rust-compiled
  `turso_core`. Version 0.7.1 contains a roughly 12.8 MB raw Wasm module.
- It uses a threaded `wasm32-wasip1-threads` build, a worker and
  `SharedArrayBuffer`, which requires COOP/COEP headers.
- Its JavaScript API is asynchronous. Idiosepius deliberately exposes a
  synchronous database façade and currently runs `turso_core` over its own
  in-memory browser VFS.
- The official Rust `turso::sync` implementation is still native-oriented;
  there is no equivalent supported Cargo feature that drops into the current
  `wasm32` build.

Turso Sync also uses
[last-push-wins conflict resolution](https://docs.turso.tech/sync/conflict-resolution).
The current database cannot safely be treated as an opaque multi-writer
replica:

- `session`, `event` and `attempt` use locally allocated integer primary keys.
  Two devices can independently allocate the same ids.
- `review_state` is mutable derived state. Last-push-wins can discard one
  device's learning progress.
- Undo deletes an `attempt` locally. A replicated undo needs an append-only
  meaning rather than a cross-device row deletion.

Direct whole-database Turso Sync is therefore possible in the browser now, but
is not the recommended Idiosepius design.

## Recommended split

Keep the existing SQLite database as the local working database. Add a
separate encrypted synchronization journal:

```text
local account root
    ├── authentication key
    ├── envelope encryption key
    └── optional local-at-rest key
              │
              ▼
local SQLite ── encrypted envelopes ──► per-account Turso database
      ▲                                      ciphertext only
      └──────── decrypt and materialise ─────┘
```

Turso is an opaque mailbox and durable ordering point, not the authority on
how study records merge. The application owns those semantics.

The remote database can begin with one table:

```sql
CREATE TABLE envelope (
    id          TEXT PRIMARY KEY,
    uploaded_at INTEGER NOT NULL,
    ciphertext  BLOB NOT NULL
);
```

The local database holds a durable outbox, downloaded envelope ids and a
remote cursor. Upload uses idempotent inserts. Download pages through remote
rows in server insertion order. No remote deletion or compaction is needed for
an initial version; study histories are small and append-only storage is much
easier to make correct.

Because Idiosepius already has a local database and an offline queue, it may
only need Turso's HTTP data API rather than the full browser replica engine.
A spike must verify direct browser CORS and token refresh. If direct access is
not suitable, a small ciphertext-only relay can live beside the control plane
without becoming an application data server.

## Identity and keys

On first account creation, the client generates a random 256-bit account root.
Use a versioned KDF such as HKDF to derive independent material for:

- an Ed25519 authentication key;
- an XChaCha20-Poly1305 envelope key;
- an optional key for local database-at-rest encryption;
- future device/key-wrapping uses.

Do not reuse one raw key for these jobs.

The server allocates a unique numeric account id, for example 24 digits shown
in groups. It stores the id and authentication public key, never the root or
encryption keys.

The recovery kit contains:

- the numeric account id;
- a 24-word encoding of the random account root;
- a format/version marker.

The QR code carries the same information in a compact versioned payload. It
must not use a normal web URL that can leak through navigation, history or
referrer logs.

### Authentication

1. A device requests a nonce for the numeric account id.
2. It signs the nonce and request context with the derived signing key.
3. The control plane verifies the stored public key.
4. It returns the database URL and a short-lived, database-scoped Turso token.

Turso supports scoped and expiring database tokens:
[Authorization](https://docs.turso.tech/sdk/authorization).
The organization-level Platform API token stays only in the control plane.

Copying one root to every device is acceptable for the first version, but
revoking a stolen device then requires rotating the root/content key. A later
design can give each device its own signing and wrapping keys, with the account
root used only to authorize devices and recovery.

## Control plane

This is intentionally much smaller than a conventional account system. Its
durable account row needs only:

```text
numeric account id
authentication public key
Turso database name/hostname
billing state and paid-through time
created/last-token timestamps
optional deleted/suspended state
```

Required operations:

- create a pending account;
- associate payment or prepaid time;
- provision one Turso database through the Platform API;
- issue a nonce;
- verify a signed nonce and mint a short-lived database token;
- suspend token issuance after a grace period;
- export/delete according to the retention policy.

Turso explicitly supports database-per-user provisioning through its
[Platform API](https://docs.turso.tech/api-reference/introduction).

The control plane must rate-limit account creation, nonce requests and token
minting. A stolen database token can only address one account and should
expire quickly, but an attacker could still upload garbage or consume quota.

## Envelope format

The authenticated plaintext should contain stable application identities, not
SQLite row ids:

```text
format version
operation UUID
device id and monotonic device sequence
hybrid logical timestamp
operation kind
deck slug
question or lesson uid, when applicable
operation payload
optional previous-envelope hash
```

Use the format version, numeric account id and envelope id as AEAD associated
data. A random nonce is stored with each ciphertext. Optional size buckets can
pad envelopes if leaking exact record size matters.

A per-device hash chain can detect missing or reordered history relative to a
head a device has already observed. It cannot make an untrusted storage
provider available, and a brand-new device has no independent knowledge of
the latest global head. Manual encrypted exports remain important.

## Merge semantics

### Authored content

Questions, lessons and facts continue to use authored `uid` values. Public
packs do not need to be copied into private storage if a source URL and
revision can recreate them. Locally imported/private packs need either an
encrypted pack blob or an encrypted snapshot path.

Content imports and user history remain distinct operations. A newer authored
question may replace an older version by `uid`; it must not detach history.

### Sessions, events and attempts

User-generated records need globally unique ids. A migration can add UUID
columns while retaining integer ids as local implementation details.

Append-only records union across devices. They refer to deck slugs and
question/lesson UIDs, which are resolved to local integer ids during
materialisation.

### Undo

Undo is an append-only tombstone referring to the attempt UUID. The local
materialised `attempt` table may still omit undone attempts, but sync must
retain both the original answer and its undo, matching the existing event-log
rule.

### Scheduler

Do not synchronize `review_state` as last-writer-wins mutable state. Rebuild or
incrementally fold it from the merged attempt/undo stream in a deterministic
order:

```text
(hybrid logical timestamp, device id, device sequence)
```

The fold must preserve the current undo policy: the attempt and box transition
are undone, while deliberately non-rollback counters such as `seen_count` and
`lapses` remain historical.

Clock skew must not be allowed to make merge results nondeterministic. A
hybrid logical clock plus a stable tie-breaker is preferable to wall time
alone.

### Other state

Lesson reads naturally union as append-only events. User-owned settings use an
explicit versioned last-writer rule based on the logical clock. Do not
implicitly apply this rule to attempts or scheduler state.

## Privacy boundary

Application-level envelope encryption is the E2EE boundary. The service
operator and Turso receive ciphertext, ids, sizes and timing metadata, but no
questions, responses, history or scheduler state in plaintext.

Turso BYOK is a useful additional server-side at-rest layer, but it is not the
same boundary. Turso receives the encryption key for operations even though it
says the key is not stored. It is also currently a Pro/Enterprise feature:
[Turso BYOK](https://docs.turso.tech/cloud/encryption).

The web application itself remains part of the trusted computing base. An
operator who can deploy arbitrary new JavaScript could ship code that steals a
loaded recovery root. Strong claims against a malicious future operator would
need reproducible/pinned clients, signed native releases, strict CSP with no
third-party scripts, or a similarly verifiable delivery mechanism.

Local OPFS encryption is a separate question:

- Keeping the root beside ciphertext in browser storage protects the cloud
  boundary but not a compromised browser profile.
- Meaningful local-at-rest protection needs a passphrase, WebAuthn-backed key
  wrapping, or OS credential storage.
- The existing browser snapshot boundary makes an encrypted container
  practical, but it changes the current property that the stored file is
  directly readable as ordinary SQLite. Explicit export can still decrypt to
  an ordinary database.

## Alternatives considered

### Direct whole-database Turso Sync

Now browser-capable, but it requires replacing or deeply integrating the
current browser database layer, adds a large threaded Wasm runtime, and still
requires schema/merge changes. It does not make conflicts correct by itself.

### Encrypted whole-database blob

Uploading a checkpointed encrypted database to WebDAV, R2, S3 or Turso is a
small first backup feature. With an ETag/generation check it can safely detect
"both copies changed", but it cannot seamlessly merge concurrent devices.
Never synchronize a live SQLite file through Dropbox/Syncthing and assume
that file-level conflict handling is database synchronization.

### Peer-to-peer

QR-assisted WebRTC or local-network transfer is a useful later supplement. It
avoids durable hosted storage but requires devices to meet and usually still
needs signalling/relay infrastructure.

### General sync platforms

PowerSync, Electric, Couch-style replication or a CRDT store bring their own
server/database model and would replace much more of this application than the
problem warrants.

## Delivery sequence after the exams

1. **Transport spike**
   - Provision a development Turso database.
   - Mint an expiring database token.
   - Upload and download opaque envelopes from native and Wasm builds.
   - Verify CORS, batching, token refresh and failure behaviour.

2. **Sync-safe local model**
   - Add stable UUIDs for user-generated records.
   - Add a durable encrypted outbox and downloaded-envelope set.
   - Define attempt, undo, lesson-read and setting envelopes.
   - Test deterministic scheduler rebuilding from histories produced on two
     devices.

3. **Local account root**
   - Implement versioned key derivation, AEAD and signed challenges.
   - Implement recovery phrase and QR import/export.
   - Recover a clean second device from only the recovery kit and remote
     ciphertext.

4. **Provisioning and billing**
   - Create the minimal control plane.
   - Provision database-per-account through Turso's Platform API.
   - Add short-lived credential issuance, grace periods and deletion policy.

5. **Completeness and hardening**
   - Decide how private packs and full disaster-recovery snapshots travel.
   - Add device-specific keys/revocation if required.
   - Consider encrypted local storage.
   - Add metadata padding, hash chains and abuse quotas as justified by the
     threat model.

## Questions deliberately left open

- Direct browser-to-Turso HTTP access versus a ciphertext-only relay.
- The exact numeric id length and display grouping.
- Recovery root encoding and whether a user passphrase wraps exported kits.
- Shared account root versus per-device keys in the first release.
- Whether the initial release syncs public pack provenance only, encrypted
  private packs, or periodic full snapshots as well.
- Retention and export behaviour after payment lapses.
- Whether sync is prepaid yearly rather than monthly. Turso's paid plans
  currently allow unlimited databases, so payment processor fixed fees are
  likely more significant than storage for a `$1/month` product:
  [Turso pricing](https://turso.tech/pricing).

