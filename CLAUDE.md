# Idiosepius

Study app. Rust workspace, egui front end, one SQLite file holds everything.

See `README.md` for usage and `DESIGN.md` for the UI rules — **read DESIGN.md
before touching anything visual.** The short version: rectangular everywhere,
no rounded corners, no shadows, monospace, tracked-out capitals for chrome,
cyan/violet for swipe direction, green/magenta (never red) for verdicts.

## Layout

- `crates/core` — library. Schema, content packs, session logging, scheduler,
  stats. Also the `idiodb` CLI for authoring and evaluation. `sql.rs` is the
  database façade; nothing else in the tree touches the driver directly.
- `crates/app` — the `eframe` binary. `theme.rs` (palette/style), `card.rs`
  (swipe motion and rotated painting), `math.rs` + `richtext.rs` (inline
  LaTeX), `explain.rs` (short/deep readings and facts), `app.rs` (screens),
  `import.rs` (decoding picked JSON/ZIP packs), `browser.rs` (the wasm shell:
  OPFS persistence, file picker, download), `audio.rs` + `soundscape.rs` +
  `library.rs` (the background soundscape and its saved scores — see below).
- `content/` — one directory per module, each a separately versioned Git
  repository holding question packs: one JSON file per topic plus shared
  facts, merged on import. Currently `control-systems/` (deck `control-systems`,
  prefix `cs`), `maths-2/` (deck `maths-2`, prefix `ma`, German),
  `automotive-mechatronics/` (deck `automotive-mechatronics`, prefix `atm`) and
  `systemtheorie/` (deck `systemtheorie`, prefix `st`, German).
  The application repository ignores `content/`; do not assume one repository's
  staging or commit operation includes another's. **Each module has its own
  `CLAUDE.md`** with its course conventions — read it before editing a pack,
  along with `AUTHORING.md` here, which is the module-agnostic guide and lives
  in this repository so there is one copy of it rather than one per course.

  One import invocation covers one deck, but one database holds as many decks
  as you like: `./reimport.sh -m cs`, `-m ma`, `-m atm` and `-m st` against the
  same path put all four on the start screen with separate exam dates and
  schedules.

  **Reimporting is a user operation, never an agent validation step.** Do not
  run `reimport.sh` or update the user's study database after editing content.
  Validate packs with `check-packs.py`, `packfmt.py --check`, and the relevant
  tests; leave any reimport to the user.

## Running it

```
cargo run                                                # study ~/idiosepius/study.db
cargo run -- path/to/study.db                            # a different database
cargo run -- --import content/control-systems/cs-*.json  # import, then study
```

`cargo run` means the app because `crates/app` is the workspace default
member. With no path the app opens `~/idiosepius/study.db`; pass one to keep
courses apart — though one database holding several decks is the normal case,
so a path is usually only for experiments. `crates/app/src/main.rs` carries the
full usage, including the `--shot` flags `tools/shot.sh` drives.

`idiodb`, in `crates/core`, is the companion CLI for authoring and evaluation:

```
idiodb study.db import content/control-systems/cs-[0-9][0-9]-*.json  # idempotent, by uid
idiodb study.db decks                       # progress per deck
idiodb study.db stats control-systems       # accuracy and readiness per topic
idiodb study.db weak control-systems        # the cards that keep being missed
idiodb study.db facts control-systems       # shared notes and symbol glossary
idiodb study.db events                      # the full log as JSON lines
```

## The web build

```
./tools/run-web.sh              # build (dev profile) and serve on :8000
./tools/run-web.sh --release    # optimized
```

**There is no Node, npm, bundler or application server anywhere in it.**
`tools/build-web.sh` runs `wasm-pack build crates/app --target web` into
`web/pkg`; `web/index.html` hosts the result as an `<idiosepius-app>` custom
element and `run-web.sh` serves the directory with `python3 -m http.server`.
Any static HTTPS host will do in production. `IDIOSEPIUS_WEB_PORT` picks
another port, and further arguments are forwarded to `wasm-pack`.

`build-web.sh` also writes `web/pkg/asset-manifest.json` after the build,
because wasm-bindgen may emit hashed snippet modules alongside the predictable
loader and `.wasm` names. The service worker needs a complete, deterministic
package list for a *first* visit to be enough for offline use — so a new build
step that adds files to `web/pkg` must keep that manifest generated, not
hand-written.

The hosted app is installable as a PWA; its shell is cached on the first
successful visit and refreshed whenever it is opened online. Course data and
history stay in OPFS, saved automatically — the only browser-specific chrome
left is the word in the bottom corner saying whether storage is up to date, and
there is deliberately no "export before you leave" prompt.

## Things worth knowing

**The database is the whole state.** No config file changes behaviour, and
there is no cache. Copying the `.db` moves the course and the history together.

**`uid` is a question's identity.** Re-importing an edited pack must keep the
attempt history and scheduler state attached. Never key on the prompt text or
the row id. A question that disappears from a pack is *retired* — `active = 0`,
so it stops being scheduled and drops out of lessons — never deleted, because
its attempts are history and history does not get rewritten. Facts are not
retired by an import at all: a pack that mentions none must not empty the
glossary the other packs of the deck depend on.

**Explanations are shared content.** A question has a short reading and a deep
reading; either may contain literal text and `{"fact": "uid"}` references.
Symbol facts supply the glyph, its spoken name, and its meaning. Keep extended
expressions in `$...$` LaTeX so the math renderer can lay out fractions,
radicals and scripts instead of displaying a slash-heavy text approximation.

**A formula fact is the formula sheet.** `FactKind::Formula` holds the equation
in `label` as bare LaTeX — no `$` fences, because it is always set as maths —
and renders as a display line inside the fact block. The set of them *is* the
printed sheet, which is why they are a kind of their own rather than notes:
they have to be enumerable, not merely findable. A calculation question's deep
reading starts by citing one, then substitutes through to the answer.

**Who an option note is addressed to decides where it is shown.**
`explain::NoteView` is the single decision — `Hidden`, `Picked` or `All` — and
`explain::option_notes` is the only place that applies it, so the card, the
review screen and the clipboard transcript cannot drift apart.

- A note is addressed to *whoever picked that option*, so the card shows the
  note of each option the learner actually selected — not all of them. That is
  how they are written: "That is the settling time", "Inverted — check the
  units". Showing every note at once turns a diagnosis into a wall.
- For `multi`, that means a note per wrongly-ticked option. A correct option
  they *missed* has no note to show; the question-level explanation covers it.
- The note sits with its option, not with the explanation block: it says why
  *this* choice was wrong, and the explanation says what is true. It is drawn
  indented under its own row, in that row's verdict colour, and it grows the
  card — it is not appended to the feedback panel.
- The review screen is the exception, and uses `All`. There the card is being
  studied rather than answered, so showing every note is right: the set of them
  is a map of the mistakes the question is built to catch.
- An unanswered card must never leak a note, for the same reason it must not
  leak the answer — a note names which option is wrong. That applies to
  `Ctrl+C` as much as to the screen.

**Options are authored correct-first and shown shuffled.** Putting the right
answer first keeps a pack reviewable and its diffs readable, but drawn in that
order it made the position the answer — every choice question in every deck had
its key in slot 1. `model::option_order` permutes them from a `u64` seed, and
`model::display_options` is the one place that resolves it, for the same reason
`option_notes` is the one place that resolves a note.

- **The seed is drawn once per dealt card**, in `App::place_card` — which is
  why every route that deals one goes through it. A card that comes back after
  a lapse comes back in a new order, so the position never becomes learnable
  the way it would if the order were fixed per question.
- **`App::shuffle` is where determinism is decided.** It is seeded from the
  system in a study session and from `SHOT_SHUFFLE_SEED` under `--shot`, so
  nothing else needs to know whether it is being captured. A card pinned with
  `--card` then takes that constant as its seed *directly*, rather than the
  next value in the sequence: `--card` is the flag that promises a diffable
  screenshot, so its layout must not depend on how many cards were dealt
  before it.
- **The seed is kept, not just used.** `Study`, `Feedback` and `Answered` each
  carry the `order_seed` of the card they hold, so the verdict panel, the
  review screen and the clipboard transcript can lay a card out exactly as it
  was answered. Re-reading a card that was never dealt (the weak-card list)
  gets a fresh one.
- **Indices that leave the UI are authored indices.** A `Response`, a grade and
  the event log only ever carry those, so a lost seed costs the layout of an
  old card and never the meaning of a recorded answer. Only two things are in
  display order: the number in an option's key box — which is therefore the
  number key that picks it — and the numbering in the `Ctrl+C` transcript, so a
  pasted card and the screen agree on what "3." was.

**Files are the shell's job, not a screen's.** Importing a deck and exporting
the database are asked for on the deck screen — the dashed row under the last
deck, and the button at the bottom — but the deck screen only records an
`app::Request`. On the desktop `App` serves it itself, from a thread so a
portal dialog cannot freeze the window; in the browser `BrowserApp` takes it
with `take_request()` and reaches for an `<input type=file>` or a download.
That is why the web build has no toolbar of its own: there is nothing left for
one to hold. Both routes decode packs through `import::decode_packs`, so a ZIP
of packs behaves identically on either.

Import offers three sources on both platforms: local files (one or more flat
JSON packs, a ZIP of any number of them, or a mixture), a short built-in list
of example course repositories, or the URL of any public GitHub repository.
The GitHub route walks the repository for every `.json` file and merges the
packs before the ordinary uid-aware import runs, so it expects the
repository's main URL — `https://github.com/owner/repository` — and not a
file, branch or directory URL.

## The soundscape

Background audio while you study, synthesised live rather than played back
from a file. The engine is **Apteronotus** (`~/src/apteronotus`, a sibling
repository): `apteronotus-lua` is a sandboxed Lua VM producing an owned
`Program`, and `apteronotus-live` schedules it onto a fundsp/cpal output.

**It is behind the default-on `audio` feature.** `--no-default-features`
builds a study app with no audio engine, no host audio stack and no dependency
on another repository at all. Everything in `audio.rs`, `soundscape.rs` and
`library.rs`, and every use of them, is `#[cfg(feature = "audio")]`.

**The dependency is declared against GitHub and patched to the working tree.**
The workspace `Cargo.toml` names `https://github.com/0x53A/apteronotus` and
then `[patch]`es every Apteronotus crate to `../apteronotus/crates/*`, because
that checkout is ahead of its pushed `main`. **Delete only the patch section**
when upstream catches up; the dependency lines are already correct. Apteronotus
is under active development, so an occasional build error inside
`../apteronotus` is its own repository's business and not something to fix
from here.

**The engine must be optimized even in a debug build.** The workspace sets
`opt-level = 3` on `fundsp`, `apteronotus-synth` and `apteronotus-live` for the
dev profile. Without it the audio callback misses its deadline and the console
fills with `buffer underrun`. Do not "simplify" those three profile stanzas
away; `cargo run` is a realtime application now.

**`audio.rs` is deliberately not Apteronotus's player.** That one keeps a live
performance sounding across an edit, reconciling revisions at a monotonic
scheduling frontier. A soundscape is picked once and left for an hour, so every
activation here is a fresh stream at cycle zero and none of that machinery is
reimplemented. What *is* kept is its one real promise: the replacement is
evaluated, lowered, opened and filled completely before the previous stream is
touched, so a score that fails to compile leaves the previous one playing.

**There is no mute, only stop — and the name is the point.** Stopped is the
default, so a session that never asks for sound never opens a device: no ALSA
handle, no DSP running behind a silent output, and in the browser no
AudioContext for the autoplay policy to block. Starting again therefore
restarts the piece rather than resuming it, which for a cyclic ambient loop is
inaudible — but it is why the control must not be called mute. A mute button
that silently closed the device and rewound the piece would be lying. The
browser additionally starts stopped whatever was saved, because restoring
"playing" outside a trusted gesture would restore an error rather than sound.

**In the browser, a frame *is* the audio scheduler's clock.** There is no
worker thread without cross-origin isolation, so `Worker::pump` — which
evaluates and fills lookahead — runs only inside an egui frame. But this app
deliberately stops requesting frames when a screen settles, so audio
scheduling would starve on an idle screen. `Soundscape::repaint_interval`
is what closes that: while anything is playing it asks for a frame every 40 ms
on the web, and every 250 ms natively, where the worker thread is doing the
real work and frames are needed only to notice a status change. **Do not
remove that call thinking the background animation covers it** — that is
exactly the bug it fixed. The ocean requests its own repaints, so before this
existed, whether the music stuttered depended on whether a decoration happened
to be visible.

For the same reason the web build fills **0.60 s** ahead where native fills
0.20 s. Apteronotus cannot buy slack that way — it is an instrument, and
lookahead is the delay before an edit is heard. Nobody is playing a background
soundscape, so nobody can feel a longer horizon, and it is the cheapest
resilience there is against a long main-thread task. The WebAudio *buffer*
size needs no attention here: `apteronotus-live` already keeps cpal's
2048-frame browser default instead of the 512-frame native target, and that
decision is shared code.

**Volume is `apteronotus_live::MasterGain`, a stage between the engine and the
device rather than part of the score.** The obvious alternative — an
Apteronotus `control` the score declares and the fader drives, which is how
that project does its live faders — was tried and removed. **`master` may be
declared only once**, so a score that already has one cannot be given a gain
stage from outside, and every real song ends with a `master(...)`. That version
worked on the shipped presets only because the presets had been edited to
declare a `volume` control, and died the moment a song was pasted in.
`tests::the_presets_declare_no_volume_control_and_do_not_need_to` is the guard
against drifting back to it.

This app then grew its own gain node, and Apteronotus grew a better one, so
**the local copy is gone and the engine's is used**: it is smoothed, so a fader
move is not a step in gain; it is calibrated in decibels over
`MasterGain::MIN_DECIBELS ..= MAX_DECIBELS`, which is a usable taper where a
linear-amplitude fader does its whole audible job in the bottom fifth; and it
is inside *every* `AudioOutput`, so the route with no persistent processor
opens through plain `AudioOutput::open` again. `Player` keeps the *number*, not
the handle, and reapplies it to a new output before that output ever plays —
the handle belongs to one stream, and a replacement must not render a block at
unity. Moving the fader is an atomic store into a node the running graph
already holds, so it never restarts or re-lowers anything, and the fader
consequently has **no disabled state**. An installation that predates the
change is migrated once by `settings::legacy_decibels`, which converts the old
0…1 position through the squared curve it actually used.

`fundsp` is consequently *not* a direct dependency of this crate any more. The
`[profile.dev.package.fundsp]` stanza stays: it still arrives through
`apteronotus-live`, and it still has to be optimized.

**The presets are byte-for-byte copies, and two tests keep them that way.**
They come from Apteronotus's `songs/`, which is that project's specification
corpus rather than a library it exports, so they travel as assets under
`crates/app/assets/soundscapes/` — currently `waves`, `drift` and `neon`, which
is a selection rather than the whole corpus. `every_preset_is_playable`
evaluates each one and builds its arena, catching a copy going stale as the
engine moves next door. Nothing may be *added* to them either — the volume
control that used to be patched in is exactly the kind of edit that made a copy
stop being a copy, and re-syncing one should stay a plain `cp`. `neon`
declares an `audio_input`; the study app opens no capture device, and
`PersistentRuntime::take_processor` feeds those lanes their declared silence
fallback, which is why it plays here unchanged.

**The library is files, and files are the shell's job.** `library.rs` holds the
saved scores and knows nothing about where they are: natively they are `.eod`
documents in `~/idiosepius/soundscapes/`, beside the database and the copied
fonts; in the browser they are in the same origin-private storage, and each row
offers a download, which is the only way out of that sandbox. Both routes go
through an `app::Request` exactly as importing a deck does, and a library write
carries the settings JSON with it rather than queueing a second request behind
itself — there is one request slot, and "which score is open" is part of the
same act. The in-memory list is updated when the request is *made*, not when
the write comes back: a list that lags the file it describes is worse than one
that is briefly optimistic, and a failed write reports itself as an error.

**A template cannot be overwritten, which is the whole of the save/save-as
rule.** The presets are compiled in, so `soundscape::Origin` is `Template`,
`File` or `Unsaved`, and the button reads "save" only for a `File` whose name
has not been retyped. Everything else is a copy under a free name, which
`Library::free_name` finds by stepping `-2`, `-3` past what is taken. A score
reopened from `settings.json` is recognised as its file only if the library
still holds that name *with that content*: an edit made and not saved is not
that file any more, and must not be able to replace it by accident.

**The event log is append-only.** Nothing may `UPDATE` or `DELETE` from
`event`. Undo removes the `attempt` row and rewinds the box, but the original
answer stays in the log — it did happen. Scheduler counters (`seen_count`,
`lapses`) are deliberately not rolled back either, so a misswipe cannot be
laundered out of the statistics.

**Logging never fails a session.** `Session::log` swallows write errors into
`take_errors()` for a status line. Losing a log line is annoying; losing your
place the night before an exam is not acceptable.

**`Session` holds `Rc<Store>`, not `&Store`.** It has to live next to the store
in the UI struct, which a borrow would forbid.

**Latency `-1` means unknown**, not zero — a card answered without a preceding
`show()` must not report a fabricated 0 ms.

## Scheduling

`scheduler.rs` is a pure function over the card's state, and it is tuned for an
exam next week rather than for retention over months. That premise is what
justifies every constant in it, so do not "correct" them towards SM-2.

Leitner boxes with sub-day intervals — 45 s, 3 min, 10 min, 1 h, 6 h, 1 day,
3 days. A correct answer moves a card up one box; a miss sends it to the
bottom, which is why the first interval is short enough that the card returns
inside the same session. When a deck has an `exam_at`, `EXAM_HORIZON` caps any
interval at 40 % of the time remaining, so everything gets another repetition
or two before it counts.

Selection scores urgency (is it due?) against weakness (how badly is it
needed?), samples from the strongest few rather than always taking the maximum
— `JITTER_LOW`/`JITTER_HIGH` decide how far apart two cards must be before the
order stops changing between draws — and penalises staying in one topic.
Interleaving is what separates recognising a card from knowing the answer.

## The database driver

The engine is **turso** (`0.8.0-pre.1`) — the Turso team's Rust rewrite of
SQLite, formerly called Limbo. The file format is ordinary SQLite, so `sqlite3`
on the command line still reads a study database.

It is a pre-release, and that is a deliberate risk taken with eyes open. If it
ever misbehaves on real data, `sql.rs` is the whole blast radius: it is the
only module that names the driver, and its surface (`execute`, `query_row`,
`query_row_opt`, `query_all`, `transaction`) is small enough to reimplement on
`rusqlite` in an afternoon.

**turso is async; core is not.** `sql::Conn` owns a current-thread tokio
runtime and blocks on every call. Do not spread `async` upward — the scheduler
is a pure function, `Session` is shared as `Rc<Store>`, and egui's frame
callback is sync.

**Rows are read eagerly.** `query_all` collects before mapping, rather than
threading the async cursor's lifetime through every caller. Result sets here
are one deck at a time.

**Row closures return `anyhow::Result`.** A corrupt JSON payload is just an
error, not something to smuggle out through the row type.

Nothing in the schema depends on turso: it was checked against the real
`schema.sql` first, upserts (`ON CONFLICT … DO UPDATE`, `excluded.*`),
`COUNT(DISTINCT)`, `LIMIT/OFFSET` subqueries, foreign-key enforcement and
transactions all behave.

## egui version notes

Currently egui/eframe **0.35**, which differs from most examples online:

- `eframe::App` has `fn ui(&mut self, ui, frame)`, *not* `update(ctx, frame)`.
  The root `Ui` has no background; paint one.
- Text layout goes through `Painter::layout*` — inside `ctx.fonts(|f| ...)` the
  `FontsView` is immutable and `layout` needs `&mut`.
- Style is set with `ctx.all_styles_mut(..)`; `ctx.set_style` is gone.
- `Rounding` is `CornerRadius`; `Frame::none()` is `Frame::NONE`.
- egui can translate and scale a layer but **cannot rotate one**. The card tilt
  is done by hand in `card.rs`: place each element's anchor at its rotated
  position, then rotate the element about that anchor by the same angle.

## Development

`.envrc` is `use nix`, so direnv puts you in the shell on `cd` and everything
below just works. The shell is required to *run* the app — eframe dlopens GL
and windowing libraries that must be on `LD_LIBRARY_PATH`. Building and testing
would also work outside it (the dependency tree is pure Rust, no C to link),
but **stay inside**: `mkShell` exports `CC` and `SOURCE_DATE_EPOCH`, build
scripts declare `rerun-if-env-changed` on them, and crossing in or out
recompiles the `x11-dl` → `winit` → `eframe` chain every time.

The workspace `default-members` is `crates/app`, so a bare `cargo build`/`run`/
`test` means the app alone. Add `--workspace` (or use `tools/run-all-tests.sh`)
to include `crates/core` and the `idiodb` binary.

```
./tools/run-all-tests.sh          # core + app, the whole workspace
cargo test                        # app only (default member)
cargo check --no-default-features # the app without the audio engine
nix-shell --run ./tools/shot.sh   # headless UI screenshots -> target/shots/
nix-shell --run "./tools/shot.sh -m ma"   # another module -> target/shots/ma/
cargo test -- --ignored           # needs a real audio device; see below
```

`shell.nix` carries `alsa-lib` for the soundscape: cpal asks pkg-config for it
at compile time and dlopens it at run time, so it is in `buildInputs` *and* on
`LD_LIBRARY_PATH`.

**One test is `#[ignore]`d because it opens the audio device.**
`audio::tests::the_default_soundscape_reaches_a_real_device` runs the whole
path — evaluate, lower, open, fill, sound, move the volume control — and is the
only check that the engine actually reaches hardware. Under Xvfb or in CI there
is no device, and a test that fails for want of one teaches nothing, so it is
opt-in rather than skipped conditionally. A capture never plays either:
`--shot` forces mute, because a screenshot run should not make noise.

**The content tooling lives here, in `tools/`, not beside the packs.** All of
it is generic, so a copy per module repository only guaranteed drift — and did:
there were two `packfmt.py` with different APIs before they were merged. Run it
from this directory, against `content/<module>/`:

```
python3 tools/check-packs.py                 # every $...$ span, every module
python3 tools/packfmt.py --check content/*/*.json
./tools/build-sheet.sh cs                    # formula sheet PDF, beside its pack
```

`check-packs.py` is a mirror of `math.rs`'s command set — **a command added to
the renderer must be added to its `SUPPORTED`**, which is precisely why it
belongs in this repository. `build-sheet.sh` needs LaTeX, which is kept out of
`shell.nix` (direnv puts every `cd` into that shell and tectonic is large); it
re-enters `tools/sheet-shell.nix` on its own.

`shell.nix` also carries `dbus.lib` for a non-obvious reason: `rfd`'s
xdg-portal backend *dlopens* `libdbus-1.so.3`. Without it the native import and
export pickers fail to open, fall through to a zenity that is not installed
either, and return "nothing was picked" — indistinguishable from Cancel, so the
buttons silently do nothing.

Check UI changes with `tools/shot.sh` rather than by eye. `--card <uid>` and
`--drag <px>` pin a capture to a specific question and a frozen mid-swipe, so
screenshots are reproducible and diffable.

**A capture is a still of a moving interface, so `--shot` has to stop the
clock everywhere.** Anything animated is a source of run-to-run pixel noise,
and noise is what makes a diff useless — a screenshot that always differs is
one nobody reads. Three things are silenced, and a fourth kind of motion added
later has to be silenced too:

- The **ocean background** is not painted at all (`OceanBackground::hidden`).
  Freezing its clock was tried first and was not enough: every swimmer is
  positioned from the width of the frame, so the composition stayed coupled to
  whatever size the window had reached by the capture frame.
- The **coin** does not spin — `CoinAnimation::new(false)` makes `spin()` a
  no-op rather than merely pausing it.
- The **card entry animation** is completed in `stage_shot` instead of being
  left to settle. `drive_shot` captures after a fixed *frame* count while
  `ENTRY_TIME` is a *duration*, so on a fast headless frame the card would
  otherwise be caught part-way in, scaled and faded by however long twelve
  frames happened to take.

## Content correctness

Questions are used to revise for real exams. A wrong answer key actively
teaches the wrong thing, so **verify against the course material before
changing a question**, and match the course's conventions rather than a
textbook's — e.g. Control Systems uses `t_se ≈ 3/(ζω₀)` for a 5 % band, and
`ζ ≈ 0.01·φ_m`. Cite the source in the `source` field.
