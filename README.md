# Idiosepius

A study app you swipe through. Named after *Idiosepius paradoxus*, the pygmy
squid — small, and it sticks to things.

Everything lives in **one SQLite file**: the questions, the complete record of
what you did with them, and the scheduler state. The app is a single binary
that opens that file directly. No server, no sync, no network.

The engine is [turso](https://github.com/tursodatabase/turso), the Rust rewrite
of SQLite, so the build is pure Rust with no C to link. The file it writes is
an ordinary SQLite database — `sqlite3 study.db` still works.

```
cargo run -- --import content/cs-0*.json    # ~/idiosepius/study.db
cargo run                                    # study it
```

With no path it uses `~/idiosepius/study.db`; pass one to keep several courses
apart. `cargo run` means the app because it is the workspace default member.

On NixOS, `nix-shell` first — eframe dlopens the GL and windowing libraries at
runtime and they must be on `LD_LIBRARY_PATH`.

## Studying

| | |
|---|---|
| `←` `→` or drag | answer false / true |
| left / right click | answer false / true |
| `1`–`5` | pick a multiple-choice option |
| `enter` | confirm a multi-select |
| `s` | skip (recorded, not graded) |
| `u` | undo the last answer |
| `esc` | end the session, show the summary |

True/false cards are swiped like a card game: drag past the threshold and the
card tilts, stamps itself, and flies off. Multiple-choice cards are clicked or
numbered.

## Layout

```
crates/core     the study database: schema, content, logging, scheduling
crates/app      the egui front end (binary: idiosepius-app)
content/        question packs, one JSON file per topic
tools/shot.sh   headless UI screenshots
```

`idiodb` is a companion CLI in `crates/core` for authoring and evaluation:

```
idiodb study.db import content/cs-0*.json   # idempotent, matched by uid
idiodb study.db decks                       # progress per deck
idiodb study.db stats control-systems       # accuracy and readiness per topic
idiodb study.db weak control-systems        # the cards you keep missing
idiodb study.db events                      # the full log as JSON lines
```

## Writing questions

A pack is JSON — deck metadata, topics, and questions. Several files may
describe the same deck; they are merged on import, which is why the Control
Systems deck is one file per topic.

```json
{
  "uid": "cs-sta-004",
  "topic": "stability",
  "prompt": "An LTI system is stable if all its poles ...",
  "kind": "multiple_choice",
  "options": [
    { "text": "have a strictly negative real part", "correct": true },
    { "text": "have a negative or zero real part", "correct": false,
      "note": "A pole on the imaginary axis is marginally stable." }
  ],
  "explanation": "Every pole must satisfy Re(p) < 0.",
  "difficulty": 2,
  "source": "Stability, summary"
}
```

`kind` is `true_false` (with `answer`) or `multiple_choice` (with `options`,
and `multi: true` when several are correct). Prompts use Unicode maths — `ζ`,
`ω`, `∞`, `≈`, `²` — rather than LaTeX, because there is no formula renderer
and a monospace font aligns them well enough.

The `uid` is the identity of a question. Re-importing an edited pack updates
the text in place and keeps the attempt history and scheduling attached to it.
Questions dropped from a pack are retired, never deleted.

## Scheduling

A Leitner box system with sub-day intervals (45 s, 3 min, 10 min, 1 h, 6 h,
1 day, 3 days), because this is built for an exam next week rather than for
retention over months. A correct answer moves a card up one box; a miss sends
it back to the bottom, where it returns inside the same session.

If a deck has an `exam_at`, no card is ever scheduled further out than 40 % of
the time remaining, so everything gets at least another repetition or two
before it counts.

Card selection scores urgency (is it due?) times weakness (how badly is it
needed?), samples from the strongest few rather than always taking the maximum,
and penalises staying in one topic — interleaving is what separates recognising
a card from knowing the answer.

## Screenshots

```
nix-shell --run ./tools/shot.sh      # writes target/shots/*.png
```

Renders the UI under Xvfb and captures each screen to a PNG, so the look can be
reviewed and diffed without a desktop session. `--card <uid>` and `--drag <px>`
make a capture reproducible, including a swipe frozen mid-gesture.
