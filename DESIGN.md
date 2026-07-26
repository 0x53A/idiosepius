# Idiosepius — design pattern

The look is deep water: a near-black blue-green field with bioluminescent
accents. It should read as an instrument, not as a web app.

## Rules

**Rectangular. Everywhere.** No corner radius on anything — cards, buttons,
rows, panels, meters. `CornerRadius::ZERO` is set on every widget class in
`theme.rs`, and no call site passes anything else. Drop shadows are off too,
because a soft shadow reads as a rounded edge even on a square corner.

**Hairline borders do the ordinary work.** Separation comes from a one-pixel
line and a change of fill, never from a shadow or a gradient. Active surfaces
brighten on hover; the large Boolean card additionally grows a stepped,
rectangular outline because a single brighter pixel is too easy to miss.
Feedback borders use their semantic colour.

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

**A wrong choice gets its own diagnosis.** An option's authored `note` says why
*that* option was the wrong one, so it belongs with the option — indented under
the row that was picked, in the verdict colour — and not in the explanation
block, which says what is true rather than what went wrong. Only the options
the learner actually selected show theirs while answering; the review screen
shows all of them, because there the card is being studied rather than
answered. An unanswered card shows none: a note names a wrong option, so it
leaks the answer exactly as surely as marking the option would.

**A cited fact is quoted, not woven in.** Facts are inset behind a two-pixel
rail — violet for a symbol, cyan for a note or a formula — because the point of
a shared fact is that it is the same wherever it appears, and it has to be
recognisable as the same thing on the fifth card that cites it. A formula fact
additionally sets its equation on a display line of its own, above the prose: a
formula cited mid-derivation has to be readable at a glance, which it is not
when it is buried in a sentence. No third rail colour was introduced for it —
the palette says "shared fact", the display line says "formula".

**Notation is named where it was used.** A deep explanation ends with the
symbols the card actually contains, and a lesson ends with the same section:
the glyphs its prose, its display maths or the formulas it quotes use, minus
the ones it stopped to define itself. A reading uses more notation than any one
card does, not less, so leaving it out is exactly where it would be missed. It
sits under a faint `symbols` rule at the foot, before the source line — a place
to look down at, never something interrupting the argument.

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
- **Feedback** — grows in over 0.12 s, then waits whether the answer was right
  or wrong. A graded answer puts square Undo and Next controls to the left and
  right of the card; a revealed answer has Next only. The answered question,
  verdict and explanation form one vertically scrollable card, with the
  explanation attached below the question rather than covering it. The card
  itself is not a giant implicit button, so the release that completes a swipe
  cannot dismiss the explanation it just opened.
- **Hover** — Boolean cards gain a strong, direction-neutral stepped outline;
  options and feedback panels brighten their border. These transitions are
  animated. The Boolean halo is made from crisp rectangular lines, not a
  blurred shadow, and stays neutral until a drag establishes TRUE or FALSE.
- **Brand coin** — the cyan outline coin makes one 0.95 s Y-axis revolution
  on boot, when a deck starts, and after a recorded answer. It remains visible
  and clickable on every user-facing screen. Screenshot mode leaves it settled
  so captures remain reproducible.

Everything is driven by `stable_dt`, clamped to 1/20 s so a stalled frame does
not teleport the card. Animations return whether they still need a repaint, so
a settled screen stops requesting frames.

## Layout

A fixed 56 pt header (deck, running score, exam countdown) over a hairline,
with session accuracy as a 2 pt bar beneath it. A centred stage for the card.
While a card is asking, square Explain and Skip controls sit below it;
multi-select adds Confirm as the rightmost control. Each has a symbol on its
face and a hotkey-labelled verb below.

Cards are sized to their content. A three-line multiple-choice question must
not sit in a half-empty box.

## Navigation

A square Back control sits at the top right of every screen below the deck
list. It is the pointer and touch route for the same operation as `Esc`: from a
lesson it returns to the lesson list, from a one-card question it returns to
the bank, and from scheduled study it ends the session and opens the summary.
In the browser, each forward screen adds a History API entry and `popstate`
calls that same operation, so the browser Back button and edge/back gesture do
not invent a second navigation model. At the deck list, browser Back is left to
the containing page.

A deck row on the home screen is a split action. The broad left segment opens
the deck: lessons, the complete question bank and progress live behind it. The
square segment on the right is the quick route into shuffled study and is
marked with two dice. These are independent hit targets with independent hover
states; the shared border must not make the whole row look like one ambiguous
button.

The dice are drawn from square outlines and square pips rather than taken from
a font. That keeps the symbol available on every platform and obeys the
rectangular visual language. The icon means random order, not a separate
scheduler or a different set of questions.

Lessons organise teaching but do not gate practice. A newly imported deck has
its entire active question bank available immediately, whether or not the deck
contains any authored lessons.

The home screen's deck list scrolls independently of its title and database
actions. It uses the available vertical room before introducing a scrollbar,
and its final import row must be fully revealable rather than clipped against
the scroll boundary. A short window or a database with many decks must not
push import or export beyond the viewport.

The question bank is grouped in authored topic order. Its current topic header
sticks to the top of the scrolling list and remains the collapse control, so a
long topic can be closed without scrolling back to its first row. Correct,
incorrect and not-yet-attempted filters are independent checkboxes; correct
and incorrect describe the latest recorded attempt. Opening a row asks that
one question through the ordinary graded study surface, then returns to the
bank and refreshes the filters. Revealing or skipping returns without turning
the question into an attempt.

## Input

Every action has a pointer route and a keyboard route, and touch works because
egui treats it as a pointer. A true/false card can be swiped, or answered with
the explicit violet `FALSE` and cyan `TRUE` controls in a row directly below
the card. Those controls are large enough to read as buttons and to hit with a
thumb; tapping the prompt itself does not answer.

Only the visible card, option, panel or button is clickable. Empty margin is a
safe focus target: clicking it must never answer a question or advance the
session. Clickable study surfaces give animated hover feedback.

The whole interface scales with `Ctrl/Cmd` + `+` or `−`; `Ctrl/Cmd` + `0`
returns to 100 %. Scaling applies equally to prose, formulas, cards and chrome.

`Ctrl/Cmd` + `C` copies a human-readable transcript of the current screen.
It preserves authored `$...$` LaTeX rather than copying JSON or flattening
formulas to screen glyphs. An unanswered card must never leak its answer into
that transcript.
