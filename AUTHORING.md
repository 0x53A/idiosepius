# Authoring content packs

How to write a module for Idiosepius. **Nothing here is specific to any one
course** — this is the shared guide, and it applies to every module unchanged.
The per-course conventions live in that module's own `CLAUDE.md`; see § 12.

Examples below are drawn from whichever module illustrates the point best.
They are illustrations, not a description of any particular deck.

A module is a **deck**: one subject, one exam date, one set of topics. Its
questions are split across several JSON files purely for editing comfort — they
are merged on import and the app never sees the seam.

A database holds **as many decks as you like.** Each carries its own `exam_at`,
topics, facts and scheduling state, and the start screen lists them all. The
only rule is that *one import invocation covers one deck*, because packs are
merged by deck slug and a set describing two decks is rejected — so importing
several modules into one database means running the import once per module:

```
./reimport.sh -m cs
./reimport.sh -m ma     # same database, second deck
```

Separate databases are a choice, not a requirement. One file per course keeps
histories independently backupable; one file for everything gives a single
start screen and a single thing to copy between machines.

---

## 1. File layout

**One Git repository per module**, checked out side by side under the
application's `content/`, one directory each. The application repository
ignores `content/`, so no repository's commits include another's — staging or
committing in one does nothing for the rest.

This guide, and all the tooling, live in the **application** repository. A
content checkout holds content only; see § 10.

Within a module, files are flat and prefixed with a short module code:

```
<mod>-00-facts.json       shared facts: symbols and notes
<mod>-00-formulas.json    shared facts: the formula sheet          (optional)
<mod>-01-<topic>.json     questions, one file per topic
<mod>-02-<topic>.json
<mod>-formula-sheet.pdf   generated — never hand-edited
```

The prefix is what the tooling globs on, so it must be short, lowercase and
unique across modules. Every file of a module carries the **same `deck`
block**, because packs are merged by deck slug and the importer rejects a set
that describes more than one deck.

```json
{
  "deck": {
    "slug": "<module-slug>",
    "title": "<Module Title>",
    "description": "<Course, lecturer, institution. What it covers.>",
    "exam_at": "2026-07-27T11:00:00+02:00"
  },
  "topics": [
    { "slug": "<topic>", "title": "<Topic Title>", "ord": 1 }
  ],
  "questions": [ ... ]
}
```

**Set `exam_at` as soon as it is known, and not before.** The scheduler
compresses its intervals as the date approaches, so a guessed date quietly
distorts the revision plan — no date is better than a wrong one. It is a
timestamp, not a day: the hour matters on the last morning.

`description` and `exam_at` need appear in only one file. `topics` is declared
by the file that owns the topic; a question referencing an undeclared topic is
rejected at import.

### Adding a module

Everything below is run from the **application checkout's root**.

1. `git init content/<module-name>` — its own repository, ignored by this one.
2. Pick the prefix. Create `<mod>-00-facts.json` with the `deck` block and an
   empty `questions: []`.
3. Write the topic files. Keep a file to one topic and roughly 20–35 questions
   — beyond that it stops being editable by hand.
4. Write `content/<module-name>/CLAUDE.md` — see § 12.
5. `python3 tools/check-packs.py content/<module-name>`
6. `./reimport.sh -m <mod>` to build the database.
7. If the subject has formulas, add `<mod>-00-formulas.json` and run
   `./tools/build-sheet.sh <mod>`.

Nothing else needs changing. The scripts discover modules by prefix, so no
script has a list of modules to keep in step.

**Glob two digits.** Any script that finds packs must use
`<mod>-[0-9][0-9]-*.json`, never `<mod>-0*.json`. The latter silently stops at
the ninth topic file, which is a bug that looks exactly like success: the
import reports a plausible count and nobody notices the missing topics. Check
the reported topic count against what you expect.

### Why a repository per module

Modules are separate bodies of work with separate lifetimes — a course ends,
its deck stops changing, and it should stop appearing in another course's
history.

One question from that split is still open: **whether the file prefixes
survive.** Inside `content/<module-name>/`, `cs-01-modeling.json` says "cs"
twice, and `01-modeling.json` would be cleaner. It stays as it is for now:
renaming should go through `git mv` so the history follows, and that is the
author's call, not a script's. The prefix is also still what the tooling globs
on, so dropping it is not a pure rename.

---

## 2. Identity and revision

**`uid` is the question.** Attempt history, box position and scheduling all
hang off it. Rewrite the prompt, swap the options, fix the answer — the history
follows. **Never renumber**, and never reuse a retired uid for a different
question: doing so silently attaches someone's past answers to something they
never saw.

Use `<mod>-<topic3>-<nnn>`: `cs-sta-014`, `ma-int-007`. The number is an
arbitrary serial, not an ordering — leave gaps and append.

Deleting a question from a pack **retires** it (`active = 0`); it is never
dropped from the database, so its history stays readable.

---

## 3. Correctness

This is the rule that matters more than every formatting rule below.

**Verify against the course material before writing or changing an answer
key.** A wrong key does not merely fail to teach — it actively teaches the
wrong thing, and spaced repetition will drill it in. Where a subject has
several defensible conventions, **match the course's**, not a textbook's, and
record which one in `source`.

`source` is free text naming where the item came from: `"Stability, Bode"`,
`"Lecture Slides — Integration II, p. 14"`. It is shown in small print under a
fact and is what you will want when a question turns out to be wrong.

State the assumptions the answer depends on **in the prompt**, not only in the
explanation. A true/false statement that is true under the course's usual
assumptions and false in general is a bad question; either qualify it or drop
it.

---

## 4. Question kinds

```json
{
  "uid": "cs-sta-014",
  "topic": "stability",
  "prompt": "For $P(s) = s^3 + 2s^2 + 3s + K$, the system is stable for which range of $K$?",
  "kind": "multiple_choice",
  "options": [
    { "text": "$0 < K < 6$", "correct": true },
    { "text": "$K > 0$", "correct": false,
      "note": "Necessary but not sufficient — the Routh entry also constrains $K$ from above." }
  ],
  "explanation": "...",
  "explain": { "deep": [ ... ] },
  "difficulty": 4,
  "source": "Stability, Routh",
  "tags": ["routh", "gain"]
}
```

**`true_false`** takes `answer`. Answered by swiping, so it should read as a
claim someone might plausibly believe. Avoid negations — "an open loop cannot
not correct a disturbance" is a reading test, not a test of the subject.

**`multiple_choice`** takes `options`, and `multi: true` when more than one is
correct. Four or five options. Import rejects a question with fewer than two
options, none marked correct, or several correct without `multi`.

**Distractors must be wrong for a reason.** The best ones are the results of
specific, common mistakes: the inverted fraction, the missing factor of 2, the
formula for the neighbouring quantity, the right method applied to the wrong
variable. A distractor nobody would pick teaches nothing and just makes the
question faster to guess. Do not pad to five options with filler.

Keep option lengths comparable — the longest option being the answer is the
oldest tell in multiple choice.

### Figures

A question's `prompt` and a fact's `body` are ordered content. A plain string
is shorthand for one text block. Use an array when text and figures need to be
interspersed; it may contain any number of either:

```json
"prompt": [
  "First inspect the open-loop response.",
  { "figure": { "kind": "bode", "num": [1], "den": [1, 10, 0], "phase": true } },
  "Which closed-loop response is consistent with it?",
  { "figure": { "kind": "step", "num": [4], "den": [1, 0.4, 4], "t": [0, 20] } }
]
```

The same array shape is valid for a fact's `body`. Transfer functions use
coefficient arrays in descending powers of $s$, exactly like MATLAB's
`tf(num, den)`. Do not write an expression string: arrays have no parsing
ambiguity and represent complex poles without special syntax.

Frequency bounds and ticks are chosen from the polynomial coefficients. A
Bode figure draws magnitude and, when `phase` is true, a second phase panel.
A step response needs a proper transfer function and a finite time interval
with `0 <= start < end`.

For a block diagram or another figure that is not a transfer-function plot,
store the complete SVG inline:

```json
{ "figure": {
    "kind": "svg",
    "src": "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 400 120\">...</svg>"
} }
```

Keep it self-contained: no file paths or web URLs. Inline paths, shapes, text
and data URLs travel with the pack; external image references are deliberately
disabled. SVG is rasterised once and cached by content hash, then its textured
rectangle tilts rigidly with a swiped card.

`tools/check-packs.py` validates figure kinds, finite coefficients, the
denominator, step ranges and SVG XML along with the usual maths spans.

---

## 5. `note`: per-option feedback

An option's `note` is addressed **to someone who picked that option**, and
explains why it is wrong or what they confused it with:

```json
{ "text": "$t_p = \\frac{3}{\\zeta\\omega_0}$", "correct": false,
  "note": "That is the settling time." }
```

Write it as a correction, not as a general remark: "That is integration, not
differentiation", "Inverted — check the units", "That would need $K_p = 4$;
here $K_p = H_O(0) = 2$". One sentence. It carries the diagnosis; the
question-level `explanation` carries the reasoning.

A `note` on the *correct* option is usually redundant with `explanation` — skip
it unless it is a genuine caveat.

**Where it shows up.** After answering, the card shows the note of each option
the learner actually *selected*, indented under that option's row and in the
verdict colour — a diagnosis of their mistake, not a wall of every note at
once. The review screen shows them all, because there the card is being
studied rather than answered. An unanswered card never shows one: a note names
a wrong option, so it would give the answer away.

That is also why a note has to stand on its own. It is read directly under the
option, often without the explanation having been opened.

---

## 6. Notation

**Maths goes in `$...$`, and only maths does.** An expression — anything with a
variable, an operator between quantities, or a relation — is LaTeX inside
fences. Bare numbers, units and quantities in running prose stay plain:

| in fences | left as prose |
|---|---|
| `$\zeta \approx 0.01\varphi_m$` | `about 70°` |
| `$K < -2$` | `−20 dB/decade` |
| `$e(\infty) = \frac{1}{1 + K_p}$` | `0 dB`, `16 %` |

This applies everywhere: prompts, option `text`, option `note`, `explanation`,
`explain` segments and fact `body`. Mixed notation inside a single question —
a LaTeX prompt above plain-text options — is the specific failure to avoid.

**Never use LaTeX in a fact `title`.** Titles are drawn through
`theme::tracked()`, which interleaves spaces for letter-spacing and never
reaches the maths renderer; `$...$` in a title renders as literal dollar signs.

**Only what the app can draw.** The renderer is `crates/app/src/math.rs` — a
hand-written subset, not LaTeX. It shows an unknown command as its own source
text rather than swallowing it, which is a visible defect on a card. It covers
fractions, radicals, scripts, sized fences, sums and integrals (`\int`,
`\iint`, `\iiint`, `\oint`), matrices, accents, blackboard bold (`\mathbb`),
Greek and the usual relations. It does **not** cover the long tail:
`\iff`, `\substack`, `\xrightarrow`, most of `amssymb`. Run
`tools/check-packs.py` after editing; it validates every span against the
renderer's actual command set.

Prefer real structure over lookalikes: `$\frac{a}{b}$`, not `a/b`;
`$\omega_0$`, not `ω₀`. The renderer exists so formulas are laid out rather
than approximated.

**Emphasis is `*…*`, over a span.** `*ein Wort*` and `*mehrere Wörter*` both
work, and `**so**` means the same thing; the markers are removed and the words
are set a shade brighter. An asterisk with no closing partner stays ink, so a
stray one cannot emphasise the rest of a card, and `\*` is always literal.
Asterisks inside `$...$` are multiplication and are left alone. There is no
italic, no bold, and no other Markdown: prose is prose.

---

## 7. Facts

Facts are shared explanation fragments, authored in a pack-level `facts` array
and referenced from any question by `{"fact": "uid"}`. Ten variants of one idea
should not carry ten copies of its wording.

| kind | carries | rendered as |
|---|---|---|
| `symbol` | `label` = the glyph, `name` = what to call it | violet rail, "ζ ZETA" |
| `note` | `title` + `body` | cyan rail, tracked-capital title |
| `formula` | `label` = the equation, `title`, `body` | cyan rail, equation on a display line |

**`symbol`** — one glyph, its spoken name, its meaning. The deep view
automatically appends definitions for every symbol that actually occurs on the
card, so a symbol fact pays for itself across the whole module. `name` must be
the LaTeX spelling without the backslash (`zeta`, `varphi`), which is how a
prompt writing `\zeta` is matched to the glyph `ζ`.

**`note`** — a rule, a caveat, a piece of reasoning. Prose, in `body`.

**`formula`** — one entry of the formula sheet. `label` is the equation as
**bare LaTeX without `$` fences**, because it is always set as maths. Import
refuses a formula fact with no label.

Give facts stable, descriptive uids: `sym-zeta`, `note-dominant-poles`,
`f-peak-time`. The `f-` prefix for formulas is a convention, not enforced.

**Do not duplicate a formula fact as a note.** If both exist, a question citing
both shows the same content twice in one panel. Formula facts state the
equation; note facts state the surrounding argument.

---

## 8. Explanations

Two readings, and they answer different questions.

**`explanation`** is the short one, shown the moment the card flips. One or two
sentences: what the answer is and the single reason it is that. It should
settle the matter for someone who merely slipped.

**`explain.deep`** is for someone who did not know. It is a list of literal
strings and fact references, and it should read as a continuous argument with
the shared facts quoted into it.

### Derivations

A question that requires a **calculation** gets a deep reading that starts from
the formula sheet and carries the arithmetic through:

1. Say what is given and what is wanted.
2. Cite the formula fact it starts from.
3. Substitute, writing out the intermediate values.
4. State the result with its units.
5. Close with what the number means, or the trap the distractors set.

```json
"deep": [
  "Peak time is expressed in the damped frequency, so that is the first step.",
  {"fact": "f-damped-frequency"},
  "$\\omega_d = 2\\sqrt{1 - 0.1^2} = 2\\sqrt{0.99} = 1.990$ rad/s.",
  {"fact": "f-peak-time"},
  "$t_p = \\frac{\\pi}{\\omega_d} = \\frac{3.1416}{1.990} \\approx 1.58$ s.",
  "At $\\zeta = 0.1$ the correction is only half a percent, which is exactly why the wrong answer sits so close to the right one."
]
```

The point is that **no step begins somewhere the reader has not been shown**.
Do not write "it follows that" across an algebraic gap, and do not quote a
number that was not computed in view.

**Recall and trivia questions do not get this treatment.** A derivation chain
for "what is the Nyquist critical point" is padding, and padding trains people
to skip the deep view.

---

## 9. Difficulty and tags

`difficulty` is 1–5 and feeds the scheduler's weakness score, so it is a real
input and not decoration:

| | |
|---|---|
| 1 | recall — a definition, a name |
| 2 | one step from a stated fact |
| 3 | a short derivation, or recall under a twist |
| 4 | several steps, or a result that must be inverted |
| 5 | a multi-step derivation combining separate topics |

Default is 2 if omitted. Be honest: inflating difficulty starves easier cards
of the repetition they need.

`tags` are lowercase, hyphenated, and free-form. Use them for the cross-cutting
threads a topic split cannot express — `laplace`, `routh`, `bode`, `design`.

---

## 10. Tooling

**The tooling lives in the application repository, and there is one copy of
it.** Every script is generic — none of them knows about any particular course
— so a copy per module repository would only guarantee they drift, which is
exactly what happened during the period when there were two `packfmt.py`. Run
them **from the application checkout's root**:

```
python3 tools/check-packs.py                  # every module
python3 tools/check-packs.py content/<module-name>
python3 tools/packfmt.py --check content/<module-name>/*.json
./tools/build-sheet.sh <mod>                  # <mod>-formula-sheet.pdf, with glosses
./tools/build-sheet.sh <mod> --terse          # formulas only, for the exam
./tools/build-sheet.sh                        # every module with a formulas pack
```

`check-packs.py` and `packfmt.py` need only python3, which the development
shell provides. `build-sheet.sh` needs LaTeX, which is deliberately *not* in
that shell — direnv puts every `cd` into it and tectonic is large — so the
script re-enters `tools/sheet-shell.nix` by itself. There is nothing to set up,
and a content checkout needs no `shell.nix` of its own.

**Any script that rewrites a pack must go through `tools/packfmt.py`.** It
serialises in the house style — options and `{"fact": …}` references on one
line, so a question reads as a block — and asserts that its output parses back
to exactly what went in. A plain `json.dump` reformats everything and buries
the real change in a thousand-line diff.

```python
import json, sys
sys.path.insert(0, "tools")
import packfmt

doc = json.load(open(path))
...
open(path, "w").write(packfmt.format_pack(doc))
```

`packfmt.py --check` exits non-zero if anything would be rewritten, and so does
`check-packs.py` if it finds a problem — both can gate a commit.

`check-packs.py` mirrors the command set implemented in
`crates/app/src/math.rs`. If the renderer gains a command, add it to the
checker's `SUPPORTED` too, or the checker will reject content that would in
fact display correctly. **That coupling is the reason the checker belongs in
the application repository**: it is a mirror of the renderer, and a mirror kept
in a different repository goes stale silently.

---

## 11. Before committing

- `python3 tools/check-packs.py content/<module-name>` is clean (it exits
  non-zero if it is not).
- `./reimport.sh -m <mod>` imports without error, and the reported topic and
  question counts are what you expect.
- Answer keys were checked against the course material, not from memory.
- New formulas are in the formulas pack, and the sheet was rebuilt.
- No LaTeX in fact titles; no plain-text maths outside prose quantities.
- The module's own `CLAUDE.md` still describes what is actually there.

Remember that a module is its **own repository**: committing in the application
checkout does not commit content, and vice versa.

---

## 12. The module's `CLAUDE.md`

Every module carries a `CLAUDE.md` at the root of its own repository. This
guide holds everything general; that file holds **only what a person or an
agent could get wrong about this particular course**, and nothing that is
already true of every module.

Worth writing down:

- **Deck slug, exam date and time, and the file list** with what each topic
  covers, so the shape of the module is visible without opening nine files.
- **Course conventions that override a textbook.** The single most valuable
  section. Where the course marks against a convention a standard reference
  contradicts — a different settling-time band, a gradient written as a row
  vector, a method deliberately not taught — say so, and say that the course
  wins. A question written from a textbook instead teaches the wrong thing.
- **The sources**, named precisely enough to check a key against: which script,
  which edition, which problem sheet. If one source is closer to the real exam
  than the others, say which and say why.
- **Errata in the source material.** Once someone has verified that a worked
  example in the script is wrong, that finding should never have to be made
  twice.
- **The language of the deck**, if it is not English, and why.
- **Anything deliberately absent** — no formula sheet, no `exam_at` yet, a
  topic that is out of scope — with the reason. An absence with no reason
  recorded reads as an oversight and gets "fixed".

Not worth writing down: anything in this guide, and anything the packs already
say. A module `CLAUDE.md` that restates the file format has buried its real
content.
