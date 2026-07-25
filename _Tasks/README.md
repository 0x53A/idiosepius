# Idiosepius — working notes

## State: usable

Backend and UI are done and working. The separately versioned `content/`
checkout contains 136 Control Systems questions and 44 shared facts. Every
card has a short reading and a deep explanation. Exam: **Monday 2026-07-27**.

```
nix-shell --run "cargo run -p idiosepius-app -- study.db --import content/cs-0*.json"
nix-shell --run "cargo run -p idiosepius-app -- study.db"
```

## Open question for Lukas

`content/cs-01-modeling.json` sets `exam_at` to **2026-07-27T09:00+02:00**.
That time is a guess — the announcement thread never states it. It only
affects the countdown and the scheduler's horizon (nothing is scheduled
further out than 40 % of the time remaining), so it is worth correcting.

## Content coverage

| topic | cards | source |
|---|---|---|
| Modeling | 28 | Course Book + slides, linearization, Laplace, poles |
| Identification | 26 | step-response characteristics, ζ/ω₀ recovery |
| Stability | 36 | BIBO, Routh-Hurwitz, Nyquist, Bode, margins |
| Accuracy | 22 | steady-state error, integrators, disturbance |
| Linear Feedback Control | 24 | P/PI/PD/PID, design trade-offs |

Every mock-exam question from `K - CS-SS25.pdf` is represented, tagged `exam`.
Course conventions are followed deliberately: `t_se ≈ 3/(ζω₀)` (5 % band, not
the 2 % `4/(ζω₀)` many textbooks use) and `ζ ≈ 0.01·φ_m`.

Extended expressions are authored as `$...$` LaTeX and rendered by the app's
box-layout math engine. Shared derivations and symbol definitions live in
`content/cs-00-facts.json`; question explanations refer to them by `uid`.

Not covered: anything needing a figure (Bode/Nyquist plot reading, "which plot
matches H(s)"), and the multi-step Section III problems. Both need question
kinds that do not exist yet.

## Next, roughly in order of value

1. **Image questions** — the exam has "read this Bode diagram" items. Needs an
   `image` field on the pack format and texture loading in the card.
2. **Numeric answer kind** — type a number, graded with a tolerance. Covers
   the short-answer section, which is currently unrepresented.
3. **Topic filter in the UI** — the scheduler already takes one
   (`next_card(.., topic_filter)`); there is no way to set it from the app.
4. **Cram and exam modes** — `Mode::Cram` and `Mode::Exam` exist in core and
   are honoured by the scheduler, but the deck screen only ever starts
   `Practice`.
5. **Resume** — a session is per-launch; nothing reopens an unfinished one.

## Asked for, not built yet

Raised 2026-07-25, deliberately deferred so the explanation work could land
first:

- **Translations.** Everything authored — prompts, options, explanations,
  facts — wants a language variant, with `l` cycling: toggle between two,
  loop through three or more. Content shape: probably a `lang` map per
  authored string, or a parallel pack keyed by uid. Note that the *choice*
  belongs in the database like everything else, not in a config file.
## Decisions worth remembering

- **JSON, not YAML**, for packs: prompts are full of `0x`-ish tokens, colons
  and `no`, all of which YAML reinterprets.
- **Packs are per-topic files** merged on import, because a 136-question
  single file is miserable to edit.
- **App and content have separate Git histories.** `content/` is a nested
  repository ignored by the application repository; importing a pack is the
  boundary between authored data and the self-contained study database.
- **Feedback colours avoid red** — green/magenta, per DESIGN.md.
- **`e` is a skip with a pause.** It writes the same `skipped` event as `s`,
  creates no attempt, and holds the answer and explanation on screen until
  the user continues.
- **`--shot` is a permanent feature, not scaffolding.** Checking a GUI by
  running it is slow and not reproducible; `tools/shot.sh` renders every
  screen under Xvfb to PNGs.
