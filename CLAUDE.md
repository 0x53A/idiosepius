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
  OPFS persistence, file picker, download).
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
review screen and the clipboard transcript cannot drift apart. There are ~1,230
authored notes across the three decks.

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
nix-shell --run ./tools/shot.sh   # headless UI screenshots -> target/shots/
nix-shell --run "./tools/shot.sh -m ma"   # another module -> target/shots/ma/
```

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
takes `tectonic`, else a system `xelatex`/`lualatex`, and re-enters
`tools/sheet-shell.nix` only when neither is installed. Never `pdflatex` — the
sheet carries no `fontenc`, so it breaks umlauts silently.

`shell.nix` also carries `dbus.lib` for a non-obvious reason: `rfd`'s
xdg-portal backend *dlopens* `libdbus-1.so.3`. Without it the native import and
export pickers fail to open, fall through to a zenity that is not installed
either, and return "nothing was picked" — indistinguishable from Cancel, so the
buttons silently do nothing.

Check UI changes with `tools/shot.sh` rather than by eye. `--card <uid>` and
`--drag <px>` pin a capture to a specific question and a frozen mid-swipe, so
screenshots are reproducible and diffable.

## Content correctness

Questions are used to revise for real exams. A wrong answer key actively
teaches the wrong thing, so **verify against the course material before
changing a question**, and match the course's conventions rather than a
textbook's — e.g. Control Systems uses `t_se ≈ 3/(ζω₀)` for a 5 % band, and
`ζ ≈ 0.01·φ_m`. Cite the source in the `source` field.
