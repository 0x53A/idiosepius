# Japanese handwriting cards — investigation notes

For the current implementation and where to resume after reboot, read
[KANA-HANDOFF.md](KANA-HANDOFF.md).

**Current, 2026-09-12:** both canvases embed the confirmed residual fine-tune.
It recognizes all 2,208 reference-derived positive controls; real beginner
handwriting accuracy and incorrect-writing feedback remain unverified.
See [KANA-BEGINNER.md](KANA-BEGINNER.md) for the current evidence and the
independent captures and reviews needed to continue. The dated updates and
benchmark results below describe earlier models.

Recorded 2026-08-11 for later work. This is an investigation, not a settled
pack format or implementation plan. The first useful scope is hiragana and
katakana; kanji and general-purpose pack plugins can follow independently.

**2026-09-07 update:** the orientation bug in `JAPANESE-REVIEW.md` is fixed,
and a web collection canvas exists. Historical cross-source figures below
predate that fix; see “Web canvas and corrected baseline” for current results.

**2026-09-08 update:** a trained CNN now runs alongside the geometric matcher
in both canvases. The smaller, three-block CNN now includes ETL7 training. ETL4/ETL5/ETL7
stroke-proxy benchmark top-1 is 76.98%/99.39%/99.16%; this is an
experimental identity baseline, not a quality grader. See
[`KANA-VISION.md`](KANA-VISION.md) for model provenance, splits, reproduction,
and current limitations. The older “ETL files still needed” notes below are
historical: the official files have now been obtained into ignored training
storage under their terms.

## Intended exercise

A card asks for a kana from its reading or romanization. The learner answers
by drawing one character with a mouse, pen or finger. The app recognizes what
was drawn and can therefore distinguish three outcomes:

- the requested kana was recognized;
- a different kana was recognized; or
- the ink was not recognizable with enough confidence.

Recognition is the first milestone. Judging stroke order, direction or
penmanship is a separate, later feature.

Most importantly, a correctly shaped kana drawn in an unconventional order
must still be recognized. Stroke-order conformance must never be an input to
the identity verdict. It may produce additional feedback after recognition,
but must not turn `あ` into "unrecognized" merely because its strokes arrived
in the wrong sequence.

## Keep the ink, not just its image

The canvas should record the original input as ordered strokes:

```text
CharacterInk = [
    Stroke = [(x, y, time), ...],
    ...
]
```

Pressure and pointer kind may be retained when available, but recognition
must not require them: a mouse supplies neither useful pressure nor a natural
pen profile. Pointer-down starts a stroke and pointer-up ends it. Preserve the
raw sample and derive normalized/resampled versions from it; do not destructively
replace it during preprocessing.

In handwriting literature this is **online recognition**: the trajectory is
available as the character is written. It does not mean that recognition
needs the Internet. This representation is the useful superset: it can always
be rasterized for an image classifier, while a bitmap cannot reliably recover
stroke boundaries, directions or order.

Capture enough pointer events that the stored line is not limited to egui's
repaint frequency. Touch also needs one active pointer to own a stroke so that
another finger cannot splice points into it. Those are canvas concerns, not
recognizer policy.

## Separate identity from writing feedback

One ink sample should feed two conceptually independent consumers:

```text
ordered vector ink
    -> order-independent recognizer -> ranked identities + confidence
    -> order-aware comparator       -> optional writing feedback
```

The recognizer should return ranked candidates rather than only yes/no. For a
question whose answer is `あ`, a result such as `お, あ, め` is useful both for
grading and for diagnosing recognizer failures. Although the expected answer
is known, recognition should search at least the complete active script—not
only the learned or expected characters. That is what lets the app say that a
learner wrote a valid but different character. Searching the other script as
well is a diagnostic option, not a requirement when the lesson already
provides script context.

There probably should not be a separate binary "is this a kana?" model. A
classifier or template matcher naturally returns a nearest character even for
a scribble, so rejection is a calibration problem: require a sufficiently
good absolute match and a useful margin over the next candidate. The eventual
threshold needs testing against held-out handwriting and deliberately invalid
ink; a nearest candidate's raw score is not automatically a probability.

Possible identity states are therefore:

```text
Recognized { top_k }    // top candidate may be right or wrong
Unrecognized           // too distant or too ambiguous
```

Stroke feedback can later say "recognized as あ; strokes 2 and 3 were
reversed" without changing the identity. Whether such feedback affects the
scheduler grade is a later product decision and should not be baked into the
recognizer.

## How seriously to treat stroke order

Stroke order is real instructional content, not merely a convention invented
for foreign learners. Japanese primary-school material explicitly teaches
attention to stroke length and direction, contacts and crossings, and writing
characters in stroke order; this is visible, for example, in the National
Institute for Educational Policy Research's [elementary assessment material][nier].
It helps produce conventional proportions and makes component-level writing
habits reusable as kanji become more complex.

It is not a sound identity test, however. Adult handwriting joins and splits
strokes, and the order-independent recognizers below were designed partly for
those ordinary variations. The Agency for Cultural Affairs also publishes
broad [guidance on acceptable handwritten jōyō-kanji forms][bunka], warning
against treating one printed glyph as the only correct realization. The app
should therefore teach canonical order as constructive feedback, without
claiming that a legible noncanonical production is a different character.

## Most relevant existing work

### ctegaki / stroke correspondence

[The Stroke Correspondence Problem, Revisited][paper] is unusually close to
this use case. It compares input trajectories directly with one reference
template per character, with matching that is explicitly independent of
stroke order and tolerant of differing stroke counts. The revision adds a
directional distance for hiragana, katakana and low-stroke kanji, where simple
endpoint matching is weak. It needs reference templates rather than a large
corpus of labelled handwriting.

The accompanying [ctegaki C implementation][ctegaki] recognizes hiragana,
katakana and the jōyō kanji. It is old and its binary pattern format is
platform-dependent, so directly linking it is not an
obvious fit for both native Rust and `wasm32`. It is nevertheless the best
starting reference: first reproduce or port only the small recognizer core and
measure it on kana. Its coarse-to-fine matching, normalization, stroke
correspondence and rejection behaviour are more valuable than its packaging.

### Zinnia

[Zinnia][zinnia] is a compact C++ online recognizer. It consumes numbered
coordinate strokes and returns n-best characters ordered by SVM confidence.
It is useful as a baseline. The stroke-correspondence
paper reports that the small open Japanese models do not handle stroke-order
and stroke-count variations particularly well, which is exactly the failure
mode to avoid here.

### Raster recognition

The vector ink can also be normalized and rasterized into a small grayscale
image for a CNN or other offline classifier. This has the attractive property
that stroke order is unknowable and therefore cannot affect identity. The
[ETL Character Database][etl] is the established Japanese handwriting corpus;
among its collections, ETL9 contains 71 hiragana and 2,965 kanji from 4,000
writers. Katakana occurs in other ETL collections. Class coverage and modern
handwritten forms need checking before choosing this route.

A raster model is likely worthwhile eventually, perhaps as an independent
second recognizer, but it brings a training pipeline, model assets and
out-of-distribution calibration. For the initial closed kana inventory,
stroke-correspondence template matching is the smaller experiment.

There is also a useful implementation precedent in [hanzi_lookup][hanzi]: a
Rust/WebAssembly browser recognizer that represents substrokes by direction,
length and normalized location. Its character data is Chinese, so it is not a
drop-in Japanese solution, but it demonstrates that this class of vector
matcher is small enough for Rust/Wasm and can run off the UI thread.

## Reference stroke data

[AnimCJK][animcjk] currently includes SVG paths for 86 hiragana and 91
katakana, with ordered median paths suitable for deriving trajectory
templates. Some looping kana split one
visual stroke into multiple SVG pieces, which its documentation explicitly
warns automatic consumers to handle.

[KanjiVG][kanjivg] provides ordered and directed SVG stroke paths plus
component metadata for kanji. It is a promising later source for kanji
templates and diagrams.

Do not generate recognition templates from an ordinary Japanese display font.
Printed forms can differ from handwritten forms, and a filled glyph outline
does not encode the intended centreline or direction.

## Suggested first experiment

Keep this narrower than a card-kind implementation:

1. Make a standalone Rust recognizer test bed accepting JSON arrays of
   strokes and returning the ten nearest kana with raw distances.
2. Convert one canonical hiragana and katakana template set to a small,
   deterministic application-owned binary or Rust data format.
3. Port or reimplement the ctegaki-style normalization and order-independent
   matching. Build it for native and `wasm32-unknown-unknown` from the start.
4. Collect multiple samples of every basic kana from several writers,
   deliberately including correct shapes in wrong orders and joined/split
   strokes. Keep this evaluation corpus separate from any threshold tuning.
5. Record top-1/top-k confusion, rejection rate, latency and ambiguous pairs.
   Do not begin stroke-order grading until identity detection is credible.

The first UI integration should retain top-k results in a diagnostic mode.
Recognition errors are otherwise hard to distinguish from drawing-canvas
sampling bugs or an overly strict rejection threshold.

## Implemented recognizer experiment

The first experiment now lives in `idiosepius-core::kana`. It contains the 46
basic hiragana and 46 basic katakana, preserves raw `CharacterInk` samples,
and derives an order-, direction- and split-independent geometric
representation for recognition. Every character has one canonical online
trajectory; hiragana additionally has one compact K49 training-split skeleton
variant. Variants are aggregated by character before ranking, so they cannot
duplicate a label or manufacture a zero-margin tie. Because an offline
historical skeleton is weaker evidence than an online trajectory, a
variant-led verdict must also satisfy a 0.05 distance ceiling and 0.02 margin,
stricter than the ordinary 0.065 and 0.012 thresholds. A conspicuously long
segment among otherwise dense pointer samples is treated as an in-air
connector, which also tolerates common pen-down joins without making stroke
count part of identity. Results always retain ranked candidates and raw
distances, including when a threshold rejects the sample. A separately
reported `excessive_ink` guard compares a jitter-simplified path length with
the winning variant and rejects ratios above 1.7; this catches gross retracing
and zigzags without making raw sampling density a verdict.

The standalone test bed accepts either `[x, y]`, `[x, y, time_ms]`, or full
point objects inside a JSON array of strokes:

```sh
cargo run -p idiosepius-core --bin kana-recognize -- ink.json
cargo run -p idiosepius-core --bin kana-recognize -- ~/idiosepius/kana-samples
cargo run --release -p idiosepius-core --bin kana-recognize -- --synthetic 24
cargo run --release -p idiosepius-core --bin kana-recognize -- --synthetic-scripted 24
cargo run --release -p idiosepius-core --bin kana-recognize -- --synthetic-ablation 24
./tools/check-kana.py
cargo run -- --kana-canvas
cargo run -- --kana-canvas --save-dir ~/idiosepius/kana-corpus/writer-a
cargo run -- --kana-canvas path/to/sample.json
```

Without `--save-dir`, samples go to `~/idiosepius/kana-samples`. A dedicated
directory per writer or collection cohort keeps the recursive evaluator's
relative paths meaningful; evaluate their common parent when comparing runs.

To inspect a retained synthetic failure without creating files elsewhere:

```sh
cargo run --release -p idiosepius-core --bin kana-recognize -- \
  --synthetic-scripted 24 > target/kana-report.json
jq '.report.valid_failures[0]' target/kana-report.json \
  > target/kana-failure.json
cargo run -- --kana-canvas target/kana-failure.json
# Or capture it headlessly:
cargo run -- --kana-canvas-shot target/kana-failure.pam \
  target/kana-failure.json
```

An invalid false acceptance is the same workflow with
`.report.invalid_acceptances[0]`; the canvas labels that object as an invalid
probe automatically.

The canvas is a native diagnostic playground, separate from the study flow.
It records every raw pointer event delivered by egui, lets one touch ID own a
stroke, recognizes after each completed stroke, shows the top ten candidates,
and shows whether the expected kana ranked first, elsewhere in the top ten, or
outside it. The expected-result line distinguishes an accepted correct answer,
an accepted wrong identity and a rejection; a correct nearest candidate that
was rejected is deliberately neutral rather than shown as a green verdict. It
saves a self-contained sample under
`~/idiosepius/kana-samples/`. A saved
JSON record contains the lossless strokes (including per-point time, pressure
and pointer kind), an optional expected kana, capture and recognizer metadata,
the active script scope, capture duration, prompt position when applicable,
the algorithm identifier and exact embedded-template fingerprint, rejection
state, and all ten ranked candidates.
Samples are written and synced through a hidden temporary file in the selected
directory, then atomically published without replacing an existing name. A
crash during the write therefore cannot leave a truncated `.json` that looks
like corpus evidence, and same-millisecond filename collisions receive a
numeric suffix without overwriting the earlier sample. Add the expected kana
before saving a failure to turn it into labeled held-out
data; passing the sample directory to `kana-recognize` reports top-1 accuracy,
accepted-correct accuracy, accepted-wrong errors, rejections, matcher time,
result changes and individual failures, with separate cohorts for prompted,
manually labeled and older unspecified samples, plus per-kana accuracy, an
aggregate nearest-candidate confusion matrix and an accepted-only confusion
matrix. The latter excludes harmless abstentions, so it directly exposes
wrong identities that would reach grading. Malformed
files are listed under `errors` without preventing the remaining corpus from
being evaluated; single-file evaluation remains strict. Directory evaluation
is recursive and reports relative paths, so samples can be organized by writer
or collection cohort without flattening colliding filenames. Its
`corpus_fingerprint` hashes the sorted relative paths and exact raw bytes of
every readable JSON file with the versioned `fnv1a64-path-and-bytes-v1`
scheme. Malformed JSON is included too. Reports with different fingerprints
therefore did not evaluate the same corpus, while a repeated run over unchanged
files has the same identity regardless of matcher timing.
Exact duplicate input is reported separately under `duplicate_ink_groups`.
Its versioned structured fingerprint includes stroke boundaries, coordinates,
time, pressure and pointer kind; candidate groups are then compared for exact
ink equality, so even a theoretical fingerprint collision cannot invent a
duplicate. `duplicate_ink_excess` is the number of observations beyond one per
group. This catches copied files and replayed raw ink that would otherwise
overweight corpus rates, including copies whose surrounding saved metadata was
changed. Each group breaks its files down by `kana:<character>`, `invalid` or
`unlabeled`, and `duplicate_ink_label_conflicts` counts groups carrying more
than one such label. The same exact trajectory labeled both valid and invalid,
or as two different kana, is therefore a visible corpus error rather than
contradictory training evidence hidden inside aggregate rates.
Its
`directory_groups` section also reports accuracy, rejection, timing and score
distributions separately for each top-level subdirectory (with `.` for loose
files at the corpus root), so pooled results cannot hide a weak writer or
collection source. Use one top-level directory per writer when collecting a
multi-writer corpus; deeper directories may still separate that writer's runs.
Each group repeats the per-script observation count, unique-label count and
exact missing-kana string, so many samples of a small easy subset cannot be
mistaken for full coverage from that writer.
Every reported binomial rate is accompanied by a two-sided 95% Wilson score
interval. In particular, a perfect result from one or a few samples therefore
still displays broad uncertainty instead of looking equivalent to a complete,
multi-writer held-out corpus. The synthetic summary uses the same intervals.
Here and in synthetic reports, top-1 measures the nearest ranked candidate even
when thresholds reject it; accepted-correct additionally requires a recognized
verdict, accepted-wrong counts harmful non-abstaining errors, and rejection
rate shows the cost of abstaining. Those three verdict counts are reported per
writer, prompt run and character as well as overall, with Wilson intervals for
both accepted rates.
Each prompted sequence has a stable run-start timestamp. Directory reports
group those samples into `prompt_runs`, listing saved, duplicate and missing
positions, as well as exact duplicate and missing kana, and marking a run
complete only when every position has one distinct inventory label with
consistent script and run length. Every run also carries its own top-1,
accepted-correct and rejection counts and rates, so tuning and held-out runs
can be compared without regrouping files by hand; top-level complete and
incomplete run counts make a mixed corpus easy to screen. Prompted samples made
before this field existed remain usable, but are counted and named under
`prompt_run_metadata_missing_files`; this makes it possible to split tuning and
held-out evaluation by collection run instead of accidentally mixing attempts
from one session across both cohorts.
New prompted records also carry the complete shuffled kana sequence. The
evaluator verifies that it is a permutation of the selected inventory and that
the label matches its recorded position, then names the expected kana at every
missing position. It cross-checks each record's `saved_before` against the
files actually present: ordinary skips remain consistent, while a later record
that claims more prior saves than the corpus contains flags a likely missing
file. Legacy prompted records remain valid and are listed separately under
`prompt_sequence_metadata_missing_files`; their older, weaker completeness
evidence is not silently confused with new sequence-aware data, and a legacy
run without that sequence cannot receive the strong `complete` verdict.
Coverage summaries list observations, unique labels and the exact missing
kana for hiragana and katakana separately.
Use `-` in the canvas's expected field to label deliberately invalid ink.
Those records are evaluated separately as rejection probes, including their
rejection rate and every false acceptance; valid-kana and unlabeled rejection
counts remain separate as well. The report also summarizes match
distance, candidate margin and simplified path-length ratio distributions for
valid labels and invalid probes separately, so threshold work can inspect the
overlap rather than tune against one aggregate percentage. A blank expected
field remains an unlabeled exploratory sample.
The top-level and each directory group also include a `capture` summary:
stroke count, point count and finite duration distributions; point-level time,
pressure and pointer-metadata coverage; and sample cohorts for mouse, pen,
touch, unknown, mixed or unspecified input. This makes a device-specific or
sparsely sampled failure pattern visible before it is mistaken for a geometry
problem in the recognizer. Pointer cohort describes the recorded event channel,
not guaranteed physical hardware identity.
Kana labels must agree with an H or K scope; the canvas save and replay paths
and the directory evaluator reject inconsistent records instead of displaying
or counting a guaranteed mismatch as recognizer evidence.
Replay output distinguishes a changed stored result from an algorithm,
template-fingerprint or rejection-threshold change, so a development build
that still says `0.1.0` does not make recognizer provenance ambiguous.
Directory reports count missing legacy provenance separately from confirmed
metadata changes and list the affected files with component-level change
flags. Stored-result presence and absence are counted separately too;
`changed_since_save_rate` is defined only over records that actually contain a
saved verdict, and missing-result files are named rather than treated as
unchanged. Reproduction compares the entire stored candidate list as a prefix, so
opening a top-five synthetic result in the top-ten canvas does not manufacture
a change while any changed stored candidate still does. Likewise, an optional
diagnostic absent from a legacy result is not treated as a claim that the
current value must also be absent.
`B`, `H` and `K` select both scripts, hiragana or katakana; script-aware
recognition models the context a lesson already has and avoids making
equivalent glyphs in the other script compete. `C` clears, `U` removes the
last stroke, `S` saves the sample, `I` labels the ink invalid, and `Ctrl/Cmd+C`
copies just the ink. `N` starts or skips ahead in a shuffled collection set
that prompts every kana in
the selected H or K scope exactly once. Prompted collection requires one of
those script scopes because handwriting alone cannot disambiguate equivalent
glyphs across both scripts; `B` remains useful for free-form diagnostics. A
new run uses a full deterministic permutation seeded from its start time, not
just a rotation of one fixed order, so writers do not repeatedly see the same
neighbor pairs and fatigue is less coupled to particular kana. The exact
resulting sequence is saved with every prompted sample, so evaluation depends
on recorded provenance rather than reproducing the shuffle implementation. A
successful save advances to the next prompt; the record distinguishes
prompted labels from manually entered ones and records how many earlier
prompts in that run were actually saved. Editing the expected field or using
the invalid-label shortcut leaves collection mode, so a later save is manual;
the canvas says so explicitly, and the save boundary also refuses internally
inconsistent prompt metadata. A run therefore cannot silently acquire a manual
or invalid label while retaining prompted provenance. The completion notice
always reports saved versus skipped kana, including zero skipped for a complete
set, so collection coverage is not overstated or inferred from recognition
output.
Passing a saved sample after `--kana-canvas` reopens its lossless strokes,
label and
script scope and recomputes the result with the current recognizer; a raw ink
array is accepted too. Replayed trajectories are fitted into the square with a
reversible view transform, so translated or scaled synthetic cases remain
fully visible without rewriting their raw coordinates; drawing on that replay
maps back through the same view. That makes both individual corpus failures and
the raw invalid acceptances retained by the synthetic report visually
inspectable.

The stored pointer kind names the egui event channel, not necessarily the
physical tool. In egui/winit 0.35 a desktop stylus can arrive through the same
generic pointer events as a mouse, which expose neither stylus identity nor
pressure; explicit touch events retain their touch ID and force when the
platform supplies it. Do not claim mouse-versus-pen corpus results from that
field without separately recording the hardware used.
Point times use egui's monotonic frame clock, not a device event clock:
multiple raw events drained during one frame therefore share a timestamp.
Stroke order and boundaries remain exact; fine-grained speed analysis does
not.

The synthetic evaluator applies whole-character anisotropic scale, rotation,
shear and smooth warping plus independent per-stroke scale and drift, terminal
shortening, point noise, stroke permutation, direction reversal, splitting
and pen-down joining. Its invalid set contains spirals, random walks,
figure-eights and crosshatching. This is deliberately more disruptive than the
initial canonical perturbations. Terminal shortening is measured by arc length
rather than point count, so a sparsely sampled katakana stroke is not shortened
more aggressively than a dense hiragana stroke. The ablation command runs the
whole-character baseline, each hard perturbation in isolation, and all of them
together with the lesson's script supplied.
Each serialized report repeats its synthetic-generator identifier, seed,
scripted-versus-both scope, variants per character and exact perturbation
switches inside `.report`, so a retained failure stays reproducible even when
detached from the CLI wrapper. The same object carries the recognizer algorithm
identifier, embedded-template fingerprint and count, and all three rejection
thresholds plus both stricter variant-led thresholds. Valid kana and invalid probes use independent deterministic random
streams, and each kana/variant pair has its own derived stream. Changing the
valid sample count therefore cannot reshuffle later kana, while every ablation
profile receives the same underlying transform for a given pair; neither can
silently change the invalid rejection corpus being compared.

`tools/check-kana.py` is the repeatable synthetic safety gate. It runs four
fixed 12-variant held-out seeds in the lesson's per-script scope, forces Cargo
artifacts into this workspace's `target/`, and atomically writes the complete
reports plus `summary.json` below
`kana-artifacts/kana-diagnostics/kana-gate-v1/`. It fails on a verdict-partition error,
top-1 below 99 %, top-5 below 100 %, accepted-wrong above 0.5 %, valid
rejection above 6 %, or invalid rejection below 99 %. These are broad
regression rails around the current experiment, not claims about handwriting
accuracy and not permission to tune thresholds until every synthetic sample
passes.
The summary is atomically replaced with `status: running` before Cargo starts
and with a terminal passed or failed state afterward, including execution and
report-shape errors. A broken run therefore cannot leave an older passing
summary looking current. Each seed has a ten-minute execution timeout, and the
summary records matcher and wall-clock latency for trend inspection without a
machine-dependent timing threshold. Wrapper and nested-report generator,
recognizer and seed provenance are cross-checked as part of the gate.
Start/completion timestamps and a versioned SHA-256 fingerprint cover the
exact matcher source, CLI evaluator, embedded template asset, template
generator and gate script.
That ties a copied summary to the uncommitted worktree bytes that produced it,
not merely to a version string or template identifier.

The first v1 gate run passed all four seeds. Top-1 was 99.46–99.91 %, valid
rejection was 2.17–3.99 %, and invalid rejection was 100 %. Three seeds had no
accepted-wrong verdict; one had 1 of 1,104 (0.09 %), a strongly transformed
`フ` accepted as `ワ`. The complete replayable failure remains in that seed's
report. Raising the ambiguity margin just enough to reject it would be
synthetic-only tuning, so the recognizer thresholds remain unchanged pending
real held-out writing.

Across four fresh `kana-synthetic-v3` fixed seeds with 12 generated samples per
character, supplying the lesson's script produced 99.46–99.91 % top-1
accuracy, 100 % top-5 accuracy, 2.17–3.99 % valid rejection and 100 % invalid
rejection, with no false acceptance in any seed. Under low system load,
optimized native recognition with the added hiragana prototypes averaged
4.98–5.01 ms in the lesson's per-script scope. `mean_recognition_ms` times
only calls into the matcher; `mean_evaluation_ms_per_sample` separately
includes synthetic generation and reporting.
In a paired 24-variant v3 ablation, pen-down joining by itself and the baseline
both scored 99.95 % top-1; their valid rejection rates were 1.18 % and 1.09 %.
That is direct evidence that connector handling is not merely hidden by
different random samples. All hard perturbations together scored 99.59 %
top-1, 100 % top-5, 3.94 % valid rejection and 100 % invalid rejection.

The invalid report is split into spiral, random-walk, figure-eight and
crosshatch families and retains the complete ranked result and raw ink of any
false acceptance. The total
number of valid synthetic failures is reported too, with one replayable
representative per affected kana; a wrong-top example takes priority over a
top-correct rejection. Each representative includes its expected label,
variant index, applied transform and structural-operation trace, complete
result and raw ink. A nested provenance block repeats the generator, seed,
scope, switches and recognizer identity, so extracting that one object with
`jq` does not orphan it from the report that produced it. Invalid false
acceptances likewise retain their probe index and provenance. That
breakdown exposed a random-walk zigzag accepted as `へ`: its ordinary shape
distance was 0.0617, but its simplified path was 2.40 times the template.
Across four fresh 24-variant seeds the largest valid ratio was 1.45 and the
1.7 guard added zero valid rejections, while the exact former false-acceptance
seed reached 100 % invalid rejection. The raw-length version of this idea was
discarded first because valid point noise inflated it as high as 2.62.

An independent-source smoke test is intentionally much harsher: against the
46 AnimCJK hiragana median trajectories, only 5 ranked the intended ctegaki
character first and only 1 passed the current rejection thresholds, even with
hiragana scope. Those medians include the split-loop problems that disqualified
them as the hiragana template set, so this is not a real-handwriting accuracy
figure. It is still a useful warning that perturbing the canonical templates
does not measure cross-source or cross-writer generalization.

`tools/check-kana-cross-source.py` now makes that warning reproducible and adds
KanjiVG as a second public canonical source. It requires explicit local
checkouts at pinned commits, adaptively flattens KanjiVG's cubic SVG strokes,
generates ordinary saved-sample documents below
`kana-artifacts/kana-diagnostics/kana-cross-source-v1/`, and runs the same directory
evaluator used for captured samples. No third-party evaluation data enters the
application or the repository. Run its dependency-free parser test, then the
measurement, with:

```sh
./tools/check-kana-cross-source.py --self-test
./tools/check-kana-cross-source.py \
  --kanjivg path/to/kanjivg --animcjk path/to/animCJK
```

The canonical-only baseline ranked 32/46 KanjiVG hiragana and 11/46 KanjiVG
katakana first. With the retained K49 hiragana variants, KanjiVG hiragana rises
to 34/46 while katakana remains 11/46. Fifteen hiragana and one katakana are
accepted correctly, none are accepted as the wrong character, and the rest
are rejected. The AnimCJK hiragana diagnostic remains 5/46 nearest and 1/46
accepted. An early extractor selected each KanjiVG path through both of its
nested ancestor groups and therefore duplicated every stroke; the current
`v1` sample fingerprint and report were regenerated after fixing that error.

Adding every KanjiVG path as a second embedded prototype was tested and
discarded. It made the now-trained-on KanjiVG measurement trivially perfect,
but changed the independent AnimCJK hiragana nearest result only from 5/46 to
6/46 while doubling the asset and comparison work. A direction-free distance
ablation and independent horizontal/vertical normalization were worse on both
KanjiVG scripts; the latter even introduced an accepted wrong answer. Weighting
template coverage more heavily than extra input ink did not change KanjiVG
rankings and weakened the synthetic gate. A bounded ±8° slant search gained one
katakana nearest match, changed no hiragana ranking, worsened rejection and
roughly tripled comparison work. None of these experiments is retained in the
recognizer. Together they are evidence that the source gap is shape diversity,
not a missing cheap invariance or a rejection-threshold adjustment.

Kuzushiji-49 provides a licensed train/test split with 48 hiragana classes plus
an iteration mark. It is historical cursive writing, not a proxy for the
appearance or pointer dynamics of a modern learner, but its 46 overlapping
basic hiragana make a useful hostile geometry corpus. `tools/check-kana-k49.py`
parses the official NPZ/NPY files with the Python standard library, verifies
their fixed SHA-256 hashes, excludes obsolete `ゐ`/`ゑ` and `ゝ`, selects each
class deterministically, and reuses the ETL bitmap skeletonizer. It never
downloads data and writes only below `kana-artifacts/kana-diagnostics/kana-k49-v1/`:

```sh
./tools/check-kana-k49.py --self-test
./tools/check-kana-k49.py --role development \
  --images path/to/k49-train-imgs.npz \
  --labels path/to/k49-train-labels.npz \
  --classmap path/to/k49_classmap.csv
```

The official training split was the only tuning source. A deterministic 24
samples per basic hiragana were reserved for development evaluation. The
canonical-only recognizer scored 13.04 % top-1, accepted 0.72 % as the wrong
character and rejected 98.55 %. `tools/build-kana-k49-variants.py` excludes
those samples, takes a separate deterministic 256-image pool per character,
and embeds the image nearest that pool's pixel centroid after thresholding,
thinning and graph tracing. With one such variant per hiragana, development
top-1 is 31.61 %, accepted-wrong is 0.36 %, and rejection is 96.38 %. Adding
the stricter variant-led acceptance rule restored all four invalid synthetic
sets to 100 % rejection.

Only after that algorithm, selection rule, prototype count and both thresholds
were frozen was the official K49 test split opened. On a deterministic 64
samples for each of the 46 basic hiragana (2,944 total), the canonical-only
comparator ranked 377 first (12.81 %), accepted 18 correctly and 16 wrongly,
and rejected 2,910. The frozen variant recognizer ranked 802 first (27.24 %),
accepted 59 correctly and 12 wrongly, and rejected 2,873. No recognizer value
was changed after observing the holdout. The complete variant summary and
canonical comparator are `holdout/summary.json` and
`holdout/canonical-baseline-report.json` in the ignored diagnostics directory.
This is clean evidence that the compact variants improve cross-style
hiragana ranking; the 97.59 % rejection rate is equally clear evidence that
K49 is not sufficient to enable graded handwriting cards.

The official ETL4 and ETL5 databases are the strongest available next proxy:
6,120 hiragana images from 120 writers and 10,608 katakana images from 104
writers, respectively. They are offline raster images rather than online pen
trajectories, and AIST prohibits redistributing the data outside its own site.
`tools/check-kana-etl.py` therefore does not download or copy the corpus. Given
the uncompressed official C-type files after the user has accepted AIST's
terms, it decodes the fixed records without third-party Python packages,
thresholds each image, applies Zhang-Suen thinning, traces the skeleton graph
into order-free polylines, selects a deterministic number per basic kana, and
passes those generated samples to the ordinary evaluator:

```sh
./tools/check-kana-etl.py --self-test
./tools/check-kana-etl.py --etl4 path/to/etl4c --etl5 path/to/etl5c
```

That result will be real cross-writer identity evidence, but only an offline
skeleton proxy. It cannot validate pointer sampling, stroke joins, order,
direction, timing or pressure. Those claims still require native online ink.
The embedded template collection's BSD, LGPL and CC BY-SA provenance,
modifications, hashes and license links are recorded beside
`kana_templates.json` rather than existing only in generator arguments.

These numbers validate implementation invariants and now demonstrate a real
cross-source ranking improvement; they do not measure modern online
handwriting accuracy. The next evidence needed is ETL4/ETL5 and, ultimately, a
held-out online corpus from several writers. UI, pack, database and scheduler
integration deliberately remain deferred until that measurement exists.

## Web canvas and corrected baseline (2026-09-07)

`web/kana.html` is a standalone diagnostic page alongside the study app:

```sh
nix-shell --run './tools/run-web.sh --release'
# Local desktop: http://localhost:8000/kana.html
```

For iPad use, publish the generated static `web/` directory through the normal
HTTPS host, then open `kana.html` in that directory. No Mac or native iOS build
is involved. This implementation was tested in Chromium with injected pen
input; actual iPad Safari/Pencil testing remains outstanding. This work has
not deployed the page.

The page starts in **Pen only**, so fingers on the square cannot become ink.
Turn that off for mouse/finger input. Choose a script and start a shuffled
46-kana set, or enter an optional expected kana for free drawing. Save commits
one sample to IndexedDB and advances only after the transaction completes;
Skip advances without saving. Editing the label leaves the prompted run.
`-` or `−` labels an invalid probe. Reload preserves saved samples but starts a
new drawing; unsaved ink and an in-progress prompt run are not restored.

Export prepares a ZIP and exposes a download link for an explicit tap into
the browser's download/Files flow. It leaves the saved collection in place.
Extract the archive into its own writer directory for the CLI evaluator;
it contains ordinary sample JSON under `browser/samples/`. The browser
serializes each document once and the Rust ZIP writer preserves those bytes,
including coordinate/time precision, without reserializing the inner JSON.

The HTML canvas records Pointer Events directly, retaining stationary pressure
changes and event timestamps. It uses coalesced samples when supplied, without
duplicating the aggregate event. One pointer owns a stroke until release or
cancellation; captured off-canvas coordinates remain in raw ink. Cancellation
retains the partial stroke and reports it for undo. Only the drawing square
disables touch scrolling. These choices follow the
[Pointer Events model](https://www.w3.org/TR/pointerevents3/); they cannot recover
samples or pressure that a device/browser never delivers.

`crates/kana-web` exposes recognition, inventories, metadata and ZIP export as
a separate core-Wasm package loaded by `kana-worker.js`. Its optimized module
is about 1.6 MiB in this build. The canvas does not instantiate egui, the study
database or audio engine. `build-web.sh` builds both packages and lists both in
the generated offline manifest. The shared service worker still caches the
whole application shell for offline use.

Native capture now preserves stationary events too. Both replay readers compare
variant thresholds and report their absence as incomplete legacy provenance.
Recognition rejects more than 16,384 raw points/strokes or 4,096 derived features
with `input_limit`, while preserving the raw sample for saving. Polyline
simplification uses an explicit stack instead of recursion. These changes are
identified as matcher v3; recognition thresholds are unchanged.

The corrected AnimCJK conversion is `x, 900 - y`, protected by an independent
upright `フ` SVG fixture. Current cross-source measurements:

| Source | Nearest correct | Accepted correctly | Accepted wrongly | Rejected |
| --- | ---: | ---: | ---: | ---: |
| AnimCJK hiragana | 24/46 | 9/46 | 0 | 37/46 |
| KanjiVG hiragana | 34/46 | 15/46 | 0 | 31/46 |
| KanjiVG katakana | 44/46 | 29/46 | 0 | 17/46 |

These are canonical-source measurements, not modern writer accuracy. The earlier
claim that katakana's 11/46 result established a shape-diversity gap was wrong:
its embedded templates had been reflected vertically.

The unchanged four-seed synthetic gate now records 99.46–99.91% top-1,
3.08–3.99% valid rejection, no accepted wrong identities and 100% invalid
rejection. **One seed fails the existing 100% top-five requirement**:
`0x55556666`, `マ`, variant 2 falls outside the top five and is rejected as a
poor match. Top-five is 1103/1104 for that seed. Neither the gate nor matcher
thresholds were weakened. Full reports and the replayable failure remain in
`kana-artifacts/kana-diagnostics/kana-gate-v1/`.

Additional probes kept all 92 canonical identities after eightfold segment
subdivision. Densely joining strokes retained 44/46 hiragana and 45/46 katakana
nearest identities, with four and two rejections respectively and no accepted
wrong identity. Mean optimized native matcher times were 6.46/1.51 ms
(hiragana/katakana) for canonical input and 27.04/1.78 ms after densification
on this run; these are measurements, not timing guarantees.

Repeat automated validation with:

```sh
nix-shell --run './tools/run-all-tests.sh'
nix-shell --run './tools/shot.sh kana'
# After building web/: uses an isolated browser profile and temporary server.
uv run --with playwright python tools/check-kana-web.py
```

The browser check covers pen-only input, stationary pressure, coalescing,
foreign pointers, cancellation, upright katakana, guided save/skip, reload
persistence, exact ZIP round-trip, offline worker startup/recognition, and
portrait/landscape/narrow layouts. Screenshots and the exported test corpus
are under `kana-artifacts/kana-diagnostics/browser-v1/`. The ZIP also passed native
evaluation with no format or provenance errors. All ten candidate identities
and verdicts agreed across Wasm and native; distances differed by at most about
`3.1e-8`. `changed_since_save` compares exact floating results and reports those
numerical differences; it is not by itself an identity-change count.

## Deferred decisions

### Generic shell / plugin boundary (2026-09-07)

Keep ink capture, storage, scheduling and ordinary controls in the host. A
future recognition capability can accept trajectories plus exercise context
and return ranked opaque identities, rejection reasons and optional feedback;
its interface need not mention Japanese. The dedicated `crates/kana-web` worker
is a first packaging boundary, not yet a general plugin ABI or WIT component.

WIT is a possible versioned contract. Native component hosting and browser
hosting should be evaluated separately. The user's existing
`~/src/wasm-in-wasm` experiment is another browser-host candidate: its app uses
`wasm_component_layer`, `wasmi_runtime_layer` and a local `wit-derive`, with
a calculator component and host console import. Reuse deserves investigation;
compatibility with a future recognition world and its interpretation overhead
still need measurement. A `jco` build
adapter is an alternative, not a prerequisite of the architecture.

If exercises later need arbitrary UI, a guest egui context could emit meshes,
clipping and texture deltas into a host-assigned panel on the same surface.
That needs explicit texture namespaces, input/focus routing, repaint requests,
platform-output handling and accessibility policy. It does not share a Rust
`Ui` across the boundary. A host-widget proxy is a different, larger UI API.
Defer both until the data-only capability boundary has a concrete second user.

References: [WIT resources](https://component-model.bytecodealliance.org/using-wit-resources.html),
[egui integration](https://docs.rs/egui/0.35.0/egui/),
[browser component tooling](https://component-model.bytecodealliance.org/language-support/building-a-simple-component/javascript.html).

### Study integration

- The authored question shape and database representation for a drawing
  response. Recorded attempts must preserve meaning independently of future
  recognizer versions; at minimum log the recognized authored answer, and
  decide whether raw ink belongs in the append-only history.
- Whether recognition searches all kana, only learned kana, or eventually the
  full kana-plus-kanji inventory. Restricting candidates improves accuracy but
  can mislabel a genuine out-of-scope character.
- How dakuten, handakuten, small kana and the long vowel mark enter the
  curriculum and candidate inventory.
- Left-handed and cursive variants, stroke joining/splitting, and acceptable
  handwritten glyph variants.
- The confidence calibration and whether an ambiguous correct target can ask
  the learner to retry without counting as a miss.
- Canonical stroke feedback, its tolerance, and whether it is informational or
  affects grading. Japanese schools teach stroke order, and it helps produce
  conventional proportions and fluent joins, but recognition must remain
  tolerant of the variations used by actual writers.
- A future Lua/Wasm pack plugin system. The Japanese deck is a good source of
  requirements for one, but the first recognizer and card kind should not wait
  for or accidentally define that general sandbox and ABI.

[paper]: https://arxiv.org/abs/1909.11995
[ctegaki]: https://github.com/asdfjkl/ctegaki-lib
[zinnia]: https://taku910.github.io/zinnia/
[etl]: https://etlcdb.db.aist.go.jp/the-etl-character-database/
[hanzi]: https://github.com/gugray/hanzi_lookup
[animcjk]: https://github.com/parsimonhi/animCJK
[kanjivg]: https://kanjivg.tagaini.net/
[nier]: https://www.nier.go.jp/12chousakekkahoukoku/03shou-gaiyou/24_shou_houkokusyo_ikkatsu.pdf
[bunka]: https://www.bunka.go.jp/seisaku/kokugo_nihongo/kokugo_shisaku/94336801.html
