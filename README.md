# Idiosepius

_*Idiosepius paradoxus*, the pygmy squid — small, and it sticks to things._

A local study app with a Rust/egui front end. One SQLite-compatible file holds
content, progress, and history.

Run the desktop app with `nix-shell --run 'cargo run'`. The default database is
`~/idiosepius/study.db`; pass a different path after `--` for an experiment.
For a browser build, use `nix-shell --run './tools/run-web.sh --release'` and
open `http://localhost:8000/`. The web build uses static files, without Node or
an application server.

The experimental kana handwriting lab is at `/kana.html` on that same server,
or `nix-shell --run 'cargo run -- --kana-canvas'` on desktop. It compares a
trained vision model with a geometric matcher, preserves original pen input,
and exports samples for evaluation. It does not change study progress.
For continuing development, start with the [kana reboot handoff](KANA-HANDOFF.md).
Local datasets and experiment records live in ignored `kana-artifacts/`, outside
the disposable Cargo `target/` directory.
See [the vision model and evaluation notes](KANA-VISION.md) and
[the handwriting investigation](JAPANESE.md) for evidence and limitations.
The current [stroke and beginner-error experiments](KANA-BEGINNER.md) document
the confirmed model, synthetic controls, and remaining rejection/feedback work.

[DESIGN.md](DESIGN.md) describes the interface rules;
[AUTHORING.md](AUTHORING.md) describes content packs. Run the complete test suite
with `nix-shell --run './tools/run-all-tests.sh'`.

The offline cache is content-versioned, including the kana model. The web build
regenerates both package inventories; after editing only web shell files, run
`python3 tools/write-web-manifest.py` before serving a distributable build.
`tools/check-web-update.py` tests complete upgrades, incomplete deployments,
and separate deployment paths without touching an existing browser profile.
