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
  LaTeX), `explain.rs` (short/deep readings and facts), `app.rs` (screens).
- `content/` — a separately versioned nested Git repository containing
  question packs, one JSON file per topic plus shared facts, merged on import.
  The application repository ignores it; do not assume one repository's
  staging or commit operation includes the other.

## Things worth knowing

**The database is the whole state.** No config file changes behaviour, and
there is no cache. Copying the `.db` moves the course and the history together.

**`uid` is a question's identity.** Re-importing an edited pack must keep the
attempt history and scheduler state attached. Never key on the prompt text or
the row id.

**Explanations are shared content.** A question has a short reading and a deep
reading; either may contain literal text and `{"fact": "uid"}` references.
Symbol facts supply the glyph, its spoken name, and its meaning. Keep extended
expressions in `$...$` LaTeX so the math renderer can lay out fractions,
radicals and scripts instead of displaying a slash-heavy text approximation.

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
```

Check UI changes with `tools/shot.sh` rather than by eye. `--card <uid>` and
`--drag <px>` pin a capture to a specific question and a frozen mid-swipe, so
screenshots are reproducible and diffable.

## Content correctness

Questions are used to revise for real exams. A wrong answer key actively
teaches the wrong thing, so **verify against the course material before
changing a question**, and match the course's conventions rather than a
textbook's — e.g. Control Systems uses `t_se ≈ 3/(ζω₀)` for a 5 % band, and
`ζ ≈ 0.01·φ_m`. Cite the source in the `source` field.
