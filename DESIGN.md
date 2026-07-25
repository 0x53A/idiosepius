# Idiosepius — design pattern

The look is deep water: a near-black blue-green field with bioluminescent
accents. It should read as an instrument, not as a web app.

## Rules

**Rectangular. Everywhere.** No corner radius on anything — cards, buttons,
rows, panels, meters. `CornerRadius::ZERO` is set on every widget class in
`theme.rs`, and no call site passes anything else. Drop shadows are off too,
because a soft shadow reads as a rounded edge even on a square corner.

**One-pixel borders do the work.** Separation comes from a hairline and a
change of fill, never from a shadow or a gradient. Borders brighten to the
accent on hover and to a semantic colour on feedback.

**Monospace throughout.** Berkeley Mono if the system has it, JetBrains Mono
otherwise, egui's default as a last resort. Nothing is vendored, so no font
licence travels with the source. Monospace is a deliberate choice, not just for
chrome: it keeps formulas like `ω₀²/(s² + 2ζω₀s + ω₀²)` aligned in a prompt.

**Chrome is tracked-out capitals.** `S T A B I L I T Y`, `E X A M  I N  2 D`.
egui has no letter-spacing property, so `theme::tracked()` interleaves spaces.
This is the single strongest signal that the UI was designed rather than
defaulted. Use it for labels; never for prose.

**Prose is sentence case at reading size.** Prompts and explanations are the
content — they get the size and the contrast. Everything else recedes.

## Palette

| role | colour | |
|---|---|---|
| `BG` | `#060A0D` | page |
| `SURFACE` | `#0B1216` | panels, rows |
| `CARD` | `#111B20` | card face |
| `CARD_DEEP` | `#0C1418` | cards further down the deck |
| `LINE` | `#1E3037` | hairlines |
| `LINE_BRIGHT` | `#34525C` | active edges |
| `TEXT` | `#D8E6EA` | prose |
| `TEXT_DIM` | `#74919A` | secondary |
| `TEXT_FAINT` | `#465C64` | labels, hints |
| `ACCENT` | `#2FE0C8` | cyan — accent, and the *true* direction |
| `VIOLET` | `#8C70EC` | the *false* direction |
| `CORRECT` | `#35E0A0` | spring green |
| `WRONG` | `#EC4AA0` | magenta |

Feedback is deliberately **not** red/green. Spring green against magenta stays
distinguishable under red-green colour blindness, and it keeps the whole
palette inside the blue-green family instead of importing a warning colour that
belongs to a different design language.

Direction colours (cyan/violet) and verdict colours (green/magenta) are kept
apart on purpose: while you drag, the colour tells you *what you are about to
answer*; after you commit, it tells you *whether you were right*. Reusing one
palette for both would conflate a choice with a judgement.

## Motion

Motion exists to make the card feel physical, and to cover state changes so
nothing pops into being.

- **Entry** — 0.22 s, scale 0.94 → 1.0 with a cubic ease-out and a fade.
- **Drag** — the card follows the pointer and tilts up to 7° at the commit
  threshold (105 pt). The tilt is a rigid rotation of the whole card, computed
  in `card.rs`, because egui can translate and scale a layer but not rotate one.
- **Stamp** — `TRUE`/`FALSE` fades in proportionally to drag distance, tilted
  a little *further* in the same direction as the card. Tilting it against the
  card cancels the rotation out and reads as a rendering fault.
- **Release below threshold** — exponential ease back to centre, frame-rate
  independent, and it settles exactly at zero rather than drifting.
- **Commit** — the card flies off along the line the hand was moving, fading
  as it goes.
- **Feedback** — grows in over 0.12 s. A correct answer auto-advances after
  0.75 s; a wrong one waits, because you should read why.
- **Brand coin** — the cyan outline coin makes one 0.95 s Y-axis revolution
  on boot, when a deck starts, and after a recorded answer. Clicking the large
  deck-screen coin also spins it. Screenshot mode leaves it settled so captures
  remain reproducible.

Everything is driven by `stable_dt`, clamped to 1/20 s so a stalled frame does
not teleport the card. Animations return whether they still need a repaint, so
a settled screen stops requesting frames.

## Layout

A fixed 56 pt header (deck, running score, exam countdown) over a hairline,
with session accuracy as a 2 pt bar beneath it. A centred stage for the card.
A single line of key hints at the bottom, lowercase and faint — it is a
reminder, not a toolbar.

Cards are sized to their content. A three-line multiple-choice question must
not sit in a half-empty box.

## Input

Every action has a pointer route and a keyboard route, and touch works because
egui treats it as a pointer. True/false additionally maps left-click to *false*
and right-click to *true*, so the whole deck can be answered without moving the
mouse.
