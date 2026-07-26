# TODO

Cross-cutting work that content authoring has run into. Each entry names what
is missing, what it currently does instead, and where the fix goes.

## Reformat the packs through the surviving `packfmt.py`

There were two `packfmt.py` — one here, one in the content repository — with
different APIs and different output. They have been merged onto this
repository's version, which is the better one (fixed key order, `--check`, and
an assertion that the output parses back to exactly what went in).

18 of the 32 packs were last written by the *other* formatter, so
`python3 tools/packfmt.py --check content/*/*.json` currently reports them as
needing a rewrite. The rewrite is safe — it is asserted faithful — but it
touches two content repositories at once and should be **its own commit**,
made when no exam is imminent, so a formatting diff never sits on top of a
content change.

## Whether module file prefixes survive

Inside `content/control-systems/`, `cs-01-modeling.json` says "cs" twice.
`01-modeling.json` reads better, but the prefix is what every script globs on
(`content/*/<mod>-[0-9][0-9]-*.json`), so dropping it is a tooling change as
well as a `git mv`. Left alone deliberately; recorded in `AUTHORING.md` § 1.

## Done

- **Plot questions were unreadable, and now they exist.** Reading a criterion
  off a figure needs the line the criterion is stated against: Bode panels
  carry 0 dB and $-180°$ rules, Nyquist marks $-1 + j0$ and the direction of
  increasing $\omega$, and frequency padding is one decade rather than two.
  A forced reference tick now displaces the regular tick that would print on
  top of its label, and a magnitude curve that never rises above 0 dB gets no
  second tick above it — both were making an axis unreadable. `\begin{array}`
  with column specs, `|` and `\hline` came in at the same time, so a Routh
  table can be shown rather than described. Five cards that had been drafted
  and dropped are in: `cs-sta-043` (Bode, which line the curve crosses first),
  `cs-sta-044` and `cs-sta-045` (Nyquist verdict and gain margin), `cs-sta-046`
  (Routh in $\lambda$, exam 2023 problem 3) and `cs-ctl-028` (placing
  $\omega_c$ for a required phase margin, exam 2023 problem 4.1).
  What *coefficients* make a reading legible is now in `AUTHORING.md` § 4,
  because it is not obvious and cost a full round of drafting to learn.
- **Lessons are a complete vertical slice.** `Lesson` and `LessonBlock` live
  beside the other authored content, schema v4 stores UID-stable readings,
  merged-pack import validates topics, facts and practice uids, and removed
  lessons retire without losing progress. The UI groups readings by authored
  topic order, renders prose, quoted facts, figures, tracked headings and
  centred display maths, and runs each practice list exactly in authored
  order. Explicit read marks are append-only `lesson_read` events. Lessons
  still do not gate the scheduler or question bank.
- **Where the shared content tooling lives** — *settled*: this repository's
  `tools/`. `check-packs.py` is a mirror of `math.rs`'s `SUPPORTED` set and a
  mirror in another repository goes stale silently; the duplicate `packfmt.py`
  proved the drift was real rather than hypothetical. `build-sheet.sh` writes
  the sheet beside its pack and re-enters `tools/sheet-shell.nix` for LaTeX,
  so `shell.nix` stays lean and the content repositories carry no `shell.nix`
  of their own.
- **Option notes are shown.** `explain::NoteView` (`Hidden`/`Picked`/`All`)
  and `explain::option_notes` are the one place the rule lives, so the card,
  the review screen and the `Ctrl+C` transcript cannot drift. A note is drawn
  indented under its own option row in that row's verdict colour.
- **The scheduler served the same handful of cards.** Scores were ranked with
  a stable sort and sampled from a fixed top five, so exactly-equal fresh
  cards came out in row-id order and a coarse term like `difficulty` acted as
  a gate rather than a preference: a real deck of 136 questions had shown 6,
  all of them the deck's only difficulty-4 cards. Scores now get multiplicative
  jitter before ranking, and the cooldown window grows with the deck. Two
  regression tests cover it.
- **Native import and export did nothing.** `rfd`'s xdg-portal backend dlopens
  `libdbus-1.so.3`, which was not on `LD_LIBRARY_PATH`; the picker then fell
  through to an uninstalled zenity and returned "nothing picked", which is
  indistinguishable from Cancel. `dbus.lib` is in `shell.nix` now. Worth
  knowing that this failure mode is silent by construction — rfd cannot report
  "no dialog available" through an `Option`.
- **`\iint`, `\iiint`, `\mathbb`** — `Big::Int(n)` now draws one, two or three
  integral signs with a tight kern, and `Node::Bb` draws double-struck letters
  as the letter plus the extra stem stroke. Not the Unicode codepoints: `ℝ` is
  missing from every monospaced face the app loads, and a tofu box in the
  domain of a function is worse than no `\mathbb` at all. Both are in the
  checker's `SUPPORTED` set and on the `--screen math` sheet.
- **`check-packs.py` matrix false positive** — `\\` is stripped before the
  command scan, so `\begin{pmatrix}x-1\\y-1\end{pmatrix}` no longer reports an
  unknown `\y`. It also exits non-zero now, so it can gate a commit.
- **Emphasis in `richtext.rs`** — `*…*` is now a span, so `*mehrere Wörter*`
  works and `**fett**` leaves no asterisks. An unmatched `*` stays ink, `\*` is
  literal, and asterisks inside `$…$` are multiplication. Documented in
  `AUTHORING.md` § 6.
- **Stale globs** — `reimport.sh` and `tools/shot.sh` both glob
  `content/*/<mod>-[0-9][0-9]-*.json` and take `-m <module>`. `shot.sh` no
  longer hardcodes `cs-*` uids: the `cs` set stays curated, any other module
  picks cards by kind and uid order and writes to `target/shots/<module>/`.
