# Stroke recognition and beginner errors

Started 2026-09-12. This iteration follows the user's request to pursue near-100%
recognition on drawn strokes and to recover likely intended identities from
malformed attempts while reporting that they need correction.

## What is being measured

Identity, validity, and intended identity are distinct outputs. A correct-looking
kana remains recognizable when the writer changes stroke order or lifts the pen.
A malformed attempt can resemble a kana without being a correct instance of it.
An omitted stroke can also yield a completely different, valid kana. Finally,
some identities are visually indistinguishable across scripts (へ/ヘ, ロ/口,
カ/力); no amount of training can recover semantic intent from identical ink.
The lesson's script can narrow recognition, and the requested answer can later
decide whether a recognized kana answers the question. Neither should force the
recognizer to return the requested identity.

The eventual feedback should support:

- Valid recognized kana, which may be a different answer than requested.
- Likely malformed kana, with an independently inferred candidate and a specific
  structural issue when evidence supports one.
- Ambiguous ink or intent; ask for another attempt rather than fabricate certainty.
- Non-kana ink.

Raw softmax, stroke count, or distance to one reference cannot supply these labels
on its own. Rejection must be measured alongside acceptance of valid variation.
Neither scheduler integration nor an automatic handwriting-quality grade is
enabled by this experiment.

## Error hypotheses and sources

[`tools/fixtures/kana-beginner/hypotheses.tsv`](tools/fixtures/kana-beginner/hypotheses.tsv)
contains a separately authored hypothesis for each of the 92 basic kana, including
likely confusion partners, omissions, mirrored components, truncations, and
misplaced contacts. These are engineering hypotheses, not a measured distribution
of beginner mistakes and not 92 teacher-certified invalid-shape labels.

The source check used the Japan Foundation's [hiragana](https://a1.marugotoweb.jp/en/hiragana.php)
and [katakana](https://a1.marugotoweb.jp/en/katakana.php) teaching references,
Kyoiku Shuppan's [first-year handwriting materials](https://www.kyoiku-shuppan.co.jp/m-link24/shosha/1nen/unit2/index.html),
which explicitly cover line endings, turns, order, and similar hiragana, and the
[JYL katakana practice book](https://www.kodomo-kotoba.info/booklet/pdf/wb_shido_katakana_50onjun_V1.pdf)
(PDF page 10 groups シ/ツ/ン/ソ among its discrimination exercises).
These support the teaching categories and reference checks; the proposed mutation
operators and individual error hypotheses are our own inferences.

Reference trajectories come from the existing pinned embedded sources and pinned
KanjiVG samples, retaining source hashes and license provenance in local manifests.
No teaching-site images or animations have been copied into the app or training.

## Frozen first corpus

`tools/build-kana-beginner-corpus.py` produces deterministic raw strokes and a
manifest under `kana-artifacts/kana-training/beginner-corpus-v1/`:

| Source | Positive controls | Corruption probes |
|---|---:|---:|
| Embedded references, development | 1,104 | 1,238 |
| KanjiVG, evaluation only | 1,104 | 1,251 |

There are also 26 procedural non-kana cases across the two script scopes: empty
ink, dense grids, concentric circles, and dense scribbles. Each character has 12
positive controls: its reference, reversed stroke order, reversed point direction,
split strokes, resampling, complete retracing, and six mild shape variations.
The mutation operators omit each stroke in turn, mirror components, truncate
either end, and displace a component. Single-stroke kana use partial deletions.
The per-character prose is an audit guide; it is not a claim that every generic
mutation realizes that prose or is invalid.

All corruption probes have `validity: unknown`. Their `parent_character` records
how they were generated and is never passed to inference. Only positive controls
carry an expected identity. The evaluator passes strokes and script alone to the
actual native CLI. Parent recall on corrupted samples is reported separately
from positive-control accuracy. KanjiVG derivatives are excluded from training;
they remain a repeatedly used public evaluation source, not fresh human holdout.

The frozen payload SHA-256 is
`78f87f1c3a5e89e17d783702b6569afe7db35fd536d7a5154a2b0bab8b048c0a`.
The generator self-test checks deterministic variation, arc-length resampling,
input immutability, and the absence of asserted invalid labels on corruptions.

## Deployed baseline

`tools/check-kana-beginner.py` evaluated all 4,723 samples using the deployed model
without expected-answer inputs. Detailed predictions and per-character aggregates
are in `kana-artifacts/kana-training/beginner-baseline-v1/`.

| Positive controls | CNN top-1 | Geometry top-1 | Geometry accepted |
|---|---:|---:|---:|
| Embedded | 96.20% | 100% | 91.67% |
| KanjiVG | 100% | 85.33% | 51.45% |

The CNN's embedded-reference misses concentrate on セ, ク, め, and ひ, plus
one そ variation. Geometry rejects all 92 full embedded retraces because its
path-length guard counts the repeated strokes twice. This is a discovered
positive-control failure, not grounds for loosening all rejection thresholds.

Geometry rejected all 26 explicit non-kana probes. The CNN produced a candidate
above 99% softmax on one. Many corruption probes also receive very high CNN
scores, but they cannot be counted as false accepts until their validity is
labelled. A threshold sweep is diagnostic evidence, not a calibrated policy.

## Training hosts

Both machines ran the same hashed trainer and `tools/benchmark-kana-training.py`
with Python 3.12, PyTorch 2.14.0+cpu, NumPy 2.5.2, batch 128, 64-pixel inputs,
three warmup steps, and eight measured steps per configuration. The code exercises
the actual models, augmentation, backward pass and AdamW. It excludes preparation,
validation, file IO and transfer; it is not a full-job or GPU benchmark.

| Best configuration (6 threads, channels-last) | Laptop i5-1345U | nixos-server Ryzen 5 7640HS |
|---|---:|---:|
| Plain depth 3, width 2 | 73.7 ms/batch | 46.9 ms/batch |
| Residual depth 3, width 2 | 129.8 ms/batch | 74.9 ms/batch |

The server was nearly idle, with about 10 GiB available memory and 59 GiB free
disk. It is about 1.6×/1.7× faster in this small benchmark. No NVIDIA/ROCm runtime
was exposed by the initial command-path probe; CPU is the measured route.
Aggregate benchmark records are in
`kana-artifacts/kana-training/host-benchmark-20260912/`.

`frost-8000` needs its full shared-tailnet hostname
`frost-8000.tail5cae6b.ts.net`. Tailscale reported it offline for 11 days, and SSH
to that hostname timed out. Its hardware and training speed remain unverified.

## Predeclared fine-tuning trial

The immutable recipe is
[`tools/fixtures/kana-beginner/trial-v1.json`](tools/fixtures/kana-beginner/trial-v1.json).
It was written before training either candidate. It compares an eight-epoch
fine-tune of the deployed plain CNN and the previously rejected residual model.
Both mix the original and corrected ETL training proxies, plus the 1,104
embedded-source positive controls. A frozen initialization teacher constrains
changes on ETL inputs. No corruption, KanjiVG, or test sample is trained on.

`tools/prepare-kana-stroke-trial.py` exports 33,106 training records and 7,540
validation records, each with the two existing proxy preparations. Test records
are excluded from the exported trial data entirely. The source cohorts are not
rewritten. Source-specific threshold redesign is deliberately a later experiment,
so it cannot obscure the effect of adding stroke examples here.

`tools/train-kana-stroke-trial.py` selects by equal-weight accuracy across the six
source/preparation validation groups, then mean loss, then earliest epoch; every
epoch is retained. Existing ETL regression gates still apply, alongside positive
stroke controls, negative diagnostics, runtime parity, size, and a second seed.
This script has its own research report format, distinct from the original trainer.
It exports candidate binaries but never installs them.

Synthetic tests exercised both architectures through two training epochs, exact
replay, selected-epoch checks, and Rust numerical parity (maximum logit differences
8.4e-8 and 1.81e-7). Full trials run sequentially on nixos-server under
`~/kana-research/stroke-trial-20260912/`, with `plain.log`, `residual.log`, and
`coordinator.log`. Only private train/validation derivatives and code are transferred.

### First trial results

Both eight-epoch runs finished. Both candidates scored 100% on each set of
1,104 positive controls and 92/92 canonical identities in both script scopes.
The plain model selected epoch 5 and stayed at 777,964 bytes; the residual selected
epoch 3 and stayed at 1,989,236 bytes. Their native export logit differences were
7.71e-6 and 2.99e-6, respectively, below the 2e-4 parity limit.

| Validation proxy score | Deployed | Plain fine-tune | Residual fine-tune |
|---|---:|---:|---:|
| Original ETL4 | 86.39% | 87.64% | 85.94% |
| Corrected ETL4 | 90.02% | 90.93% | 91.50% |
| ETL5 (both preparations) | 99.37% | 99.42% | 99.61% |
| ETL7 (both preparations) | 98.24% | 98.33% | 98.96% |

The plain model's mean corrected hiragana gain was **0.49699** percentage points,
just below the frozen 0.5-point requirement; the gate was not rounded or waived.
The residual gained **1.09566** points and passed all first-stage gates, including
the original ETL4 regression limit and the non-kana confidence-count check.
Neither candidate is deployed. Results and manifests are under
`stroke-finetune-plain-v1/`, `stroke-finetune-residual-v1/`, `beginner-plain-v1/`,
and `beginner-residual-v1/` in the training archive.

`confirmation-v1.json` freezes a second fine-tuning seed (20260913) and the
independently trained second residual initialization. The server now runs an exact
repeat of the first residual trial, followed by this confirmation. Check
`reproduction.log`, `confirmation.log`, and `confirmation-coordinator.log`.
Both cases keep the same performance gates; results are assessed separately.

`tools/plot-kana-beginner.py` also produced 92 SVG audit sheets and an index in
`kana-artifacts/kana-review/beginner-v1/`. These expose the actual mutated ink and
independent predictions for review. The お sheet was inspected: several harmless
short-stroke distortions remain legible while omissions remove essential reference
parts, illustrating why a generic mutation label must not be treated as a universal
invalidity label.

### Confirmation and promotion

The exact full residual repeat matched 15 files byte-for-byte, including all eight
epochs, the selected checkpoint, history, and report. The alternate initialization
and seed selected epoch 5, also passed all gates, and reached 100% on both positive
control sets and 92/92 canonical forms. Its corrected hiragana mean gain was
1.09181 points; original ETL4 fell 0.45352 points, within the frozen limit.
Both candidates passed browser/native parity (maximum score deltas 3.64e-12 and
5.83e-11). Desktop module replay, including both classifiers, took about 19 ms
median and 25.5 ms p95; this is not measured iPad or worker round-trip latency.

The first confirmed residual trial is now embedded, with binary SHA-256
`adfe3ccbc3de26e15d8cfb7e1ba132c0a1aab5eb77f6696fe661d4c92a101d2c`.
The prior weights, synthetic golden fixture and complete historical report were
copied unchanged to `kana-artifacts/model-releases/pre-stroke-finetune-20260912/`
with hashes. The shipped report now describes the current model and explicitly
states that its test metrics have not been evaluated. Earlier test numbers must
not be attributed to these weights. No study database was touched.

The embedded-model core inference tests, native CLI build and release web build
pass. Browser canvas, worker parity and offline-update checks also pass; logs
are `stroke-model-*.log` in the training archive. The worker replay over 92
canonical references had maximum score delta 3.64e-12, with desktop round trips
of 21.85 ms median and 38.80 ms p95. This is not measured iPad latency.
All four server training runs are finished; there is no unattended training job
remaining.

### Captured intent and writing validity

**Current stopping condition:** independent validity judgments and real beginner
strokes are still absent. This was checked again after three continuations:
there is no native capture directory and no completed review export in the
review artifacts. The five recent remote training runs have all exited with 0;
their selected checkpoints failed the frozen small-loop requirement. There is
no pending training process to wait for. The goal remains unachieved: synthetic
recognition/control results do not prove reliable malformed-writing feedback or
near-100% detection in the actual beginner-input setting. Further grading-policy
validation needs independent evidence, rather than more tuning against the same
observed examples. The capture and review workflow is ready to receive it.

Completed review downloads can now be validated without touching the corpus:

```sh
python3 tools/check-kana-reviews.py \
  --queue kana-artifacts/kana-review/annotation-queue-v3 \
  --reviews /path/to/kana-review.json \
  --output kana-artifacts/kana-review/validated-review-v1
```

The validator checks queue contents and hashes, every task's exact ink hash,
review binding, duplicate/unknown ids, reviewer aliases, reveal provenance,
single-character identity fields and reasons for invalid judgments. Unknown
and ambiguous judgments stay outside definite-validity counts; summaries split
blind versus hypothesis-seen supplied judgments. Original downloaded bytes,
normalized annotations, source aliases and validator code are archived in a new
directory. Nothing is automatically imported into training or a study database.
Self-tests cover binding failures, conflicting entries, missing provenance and
ambiguity exclusion. The actual 44-task queue binding passes; no completed
review was supplied and no correctness result is inferred from that absence.

The current review queue is now `annotation-queue-v3/` (**44 unique inks**).
Direct visual inspection of the five flagged disagreements found that the two
truncated フ records are identical ink from overlapping sources. The generator
now deduplicates exact script-plus-stroke coordinates before sampling and retains
all `source_alias_ids`. There are four unique disagreements, 20 other flagged
cases and 20 controls; different point samplings/equivalent rasters are not yet
merged. Checks verify uniqueness and retained フ aliases, and the HTML opens
directly offline with hidden hypotheses and unknown validity.

`engineering-observations.json` records the assistant's informed visual reading
of the four disagreements. These are not adjudicated reviews: validity remains
unknown and inferred intent is not asserted. The short フ-like path can plausibly
remain a shortened kana rather than a missing-stroke マ; the single curve from ゆ
cannot uniquely establish intended り; the mirrored ア is テ-like but not decisive;
and the fragmented む supplies weak support for the proposed か repair. These
examples motivate ambiguity-aware feedback rather than confident defect labels.
The observations were made after seeing source/proposal labels and must not be
treated as blind or independent confirmation.

`tools/build-kana-review-queue.py` builds an offline annotation page; the current
historical queue is `kana-artifacts/kana-review/annotation-queue-v2/index.html`. It contains
45 deterministically shuffled cases: all five flagged parent disagreements,
20 other external-source flagged cases and 20 external canonical controls.
Source identities, mutation metadata and repair hypotheses are initially hidden.
The reviewer records writing form (`valid`, `invalid`, `ambiguous`, or unreviewed
`unknown`), drawn kana, possible intended kana, reason and an alias. Revealing the
hypothesis is recorded to distinguish blinded from informed judgments.

Reviews are separate downloadable JSON bound to the queue and exact ink hashes.
They do not edit source corpora, train a model, or certify correctness. Ambiguous
and unknown judgments must not become positive or negative training labels.
Progress uses browser-local storage when available; the exported JSON is the
portable record. Serve this directory with a local static server or open its
HTML directly. This is a deliberately selected diagnostic queue, not a
representative population sample. Actual review is still outstanding.

Browser checks passed for unknown/hidden defaults, edits, navigation, reload,
reviewer preservation, hypothesis reveal tracking and hash-bound download. The
test used a fresh browser context and discarded its artificial judgments; its
marked screenshot (`browser-check.png`) is a UI test, not an annotated dataset.

Native and browser sample exports now include `writing_validity`: ordinary
captures (prompted, manually labelled or unlabelled) use `unknown`; an explicit
invalid-ink capture uses `invalid`. The additive field preserves format version
1 compatibility. `expected` and `expected_source` still describe the annotated
intended kana, not a certification of the strokes. Capturing a correct classifier
match never changes unknown validity into valid.

`tools/check-kana-corpus.py` report version 2 corrects the old accounting that
called every kana-labelled capture “valid.” It reports intended-identity match
rates and groups those matches by supplied writing validity. Legacy kana-labelled
samples default to unknown. A reviewed sample may supply `writing_validity` as
`valid` or `invalid` while retaining its intended `expected` kana; these are
annotations, not inferred truth. The old `expected_invalid` non-kana annotation
remains distinct and cannot be combined with an expected kana. Duplicate ink with
conflicting validity annotations is rejected rather than silently merged.

Thus a malformed attempt labelled with its intended kana can yield a successful
intent match and still have invalid writing. No quality grader is implied.
Self-tests cover this case, legacy unknown defaults, contradictory annotations
and duplicate conflicts. Browser checks confirm prompted unknown validity
survives saving, export and offline reload. Replaying that browser fixture
(`capture-validity-replay-v1.json`) gives two unknown-validity intended samples
and one explicitly invalid sample. Old report files are not rewritten; consumers
must use the version-2 names such as `vision_intended_top1_rate` and
`intent_label_score_bins` rather than treating former “valid” counts as reviewed
handwriting. This change prepares capture/evaluation data; automated writing-error
feedback is still research-only. All 27 native canvas tests pass, including
prompted unknown-validity and explicit-invalid serialization checks.

The native reload/re-export path now preserves supplied `valid` or `invalid`
writing validity while both the raw ink and intended label remain unchanged.
Previously it ignored the new field and re-exported unknown. A loaded review is
bound to the original ink and label; editing either discards that review for
the changed sample, and clearing removes it. Explicitly marking ink invalid
still supplies a fresh invalid annotation. Import rejects contradictory
non-kana/validity labels and unsupported validity values. All **29 native canvas
tests** pass, including actual in-memory save/re-export for reviewed valid and
malformed intended-kana samples, edit invalidation and legacy unknown defaults
(`review-validity-native-tests-v2.log`). This preserves annotations rather than
creating automated correctness judgments.

### Small-loop experiments

`style-affine-trial-v1.json` freezes the synthetic-morphology comparison against
the four-epoch weighted trial. `augment(..., morphology=False)` skips only the
3×3 max/min-pooling operations for synthetic input, retaining the affine stage.
The Python random draw is still consumed, preserving subsequent draws; ETL
augmentation calls retain their original default. Across 30 seeds, the default
output exactly matches the archived trainer, disabled morphology preserves both
Python and Torch RNG states, and applying the selected pooling operation to the
affine-only result exactly reconstructs the old output. The data, class balance,
initialization, four epochs, loss weights and all gates remain unchanged.
The remote runner archives updated source files and the prior remote scripts
were preserved under `*-pre-affine.py` before updating them.

**Affine-only synthetic result: rejected.** Four epochs completed, selecting
epoch 4 by the frozen ETL rule. The observed loop result remains **5/6**;
removing synthetic morphology did not resolve the last case. Original ETL4
gains 0.68027pp and clean ETL4 gains 0.45351pp; ETL5 is unchanged and ETL7 loses
0.08695pp in both views. Native logit delta is 3.12e-6. The archived run is
`style-affine-finetune-residual-v1/`, with model SHA
`f9e4d18272d66549596bb96558a0ef1f09ee3e881b4b620d024694151ee14b3f`;
`style-affine-v1-remote.log` records completion. No candidate was promoted or
confirmed. Several controlled trials now plateau at 5/6, so further tuning
against the same six inspected probes is deferred. The next priority returns
to intended-identity versus writing-validity annotations and error-feedback
evaluation; these observed failures remain development evidence.

`tools/check-kana-loop-morphology.py` isolates the existing trainer's 3×3
thickness augmentation from its affine transformations. The NumPy operation was
checked exactly against PyTorch max-pooling/erosion, including padding, on random,
empty and filled images. `loop-morphology-v1/` replays 18 original る/ろ reference
and mild-shape inputs through the deployed model: parent recall is 15/18 before
thickness changes, 11/18 after dilation and 14/18 after erosion. Dilation changes
the clipped AnimCJK canonical る plus mild variants 0, 2 and 4 from る to ろ;
erosion additionally changes mild variant 4. This is evidence of sensitivity,
not proof of invalid transformed writing or that augmentation caused the model
failure. The running eight-epoch trial retains its frozen augmentation. A later
comparison can isolate morphology on synthetic strokes, retaining scan-domain
augmentation and every existing validation gate.

**Style-adaptation trial:** `tools/fixtures/kana-beginner/style-trial-v1.json`
keeps the rejected shape trial's seed, optimizer, epoch count, transformations
and class balance, but replaces half of each hiragana's added variants with
clipped AnimCJK canonical-source variants. This explicitly changes AnimCJK
hiragana from evaluation-source data to development/training-source data.
All earlier source results remain archived; future gains there must not be
described as independent source generalization. KanjiVG and ETL validation/test
remain excluded from training. The already inspected failures are development
checks and are not copied as training rows.

`style-trial-prepared-v1/` has 6,992 synthetic training examples, of which 1,472
come from the newly adapted source. Separate-seed evaluation transforms remain
excluded. Array comparisons against the shape trial verify unchanged ETL arrays,
all labels, original controls and per-class counts; only intended reference
images change. The training script accepts additional sources only through an
explicit protocol allow-list, retaining embedded-only defaults for prior trials.
The data hash is
`30c400b5255c498ae1488277eff7e780ca407458216935dba7df577287b2ce75`.
All previous gates remain, and this trial additionally requires all six observed
small-loop controls correct as a development check. Check it with
`check-kana-shape-screen.py --required-loop-correct 6` before broader gates.

**Style trial result: rejected.** Four epochs completed and ETL selection chose
epoch 4. The six observed loop controls improve from 3/6 to **4/6**, below the
predeclared 6/6. Original ETL4 gains 0.90703pp, clean ETL4 0.34013pp; ETL7 loses
0.08695pp in both views, ETL5 is unchanged. Native logit delta is 4.50e-6.
`style-finetune-residual-v1/shape-screen.json` records the rejection; its model
SHA is `fce4118c322fa5c36b416b7e317186fe2ae48d374719abbe09a123cb22e3138f`.
All run artifacts and `style-v1-remote.log` are local. No deployment or
confirmation is claimed. `style-weight-trial-v1.json` freezes the next one-factor
comparison: increase supervised synthetic weight from 0.25 to 0.50, leaving
prepared data, seed, four epochs, initialization and all gates unchanged.

**Weight trial result: rejected.** `style-weight-finetune-residual-v1/` selected
epoch 4 and reached **5/6**, still below 6/6. The remaining `mild_shape_3` predicts
ろ at 74.75% versus る at 24.75%. Original ETL4 gains 0.90703pp, clean ETL4 gains
0.45351pp; ETL5 loses 0.04859pp and ETL7 loses 0.08695pp in both views, within
the specified limits. Native logit delta is 2.69e-6. Candidate SHA:
`6a424fd612f4b16920a04c7354c5fae2680da478f839dbf24522b588cf83ea16`.
Artifacts and `style-weight-v1-remote.log` are local; no promotion or independent
confirmation was run. `style-eight-trial-v1.json` freezes the next comparison:
eight epochs and an eight-epoch cosine decay, with the same weight-trial data,
initialization, seed and loss mixture. It is a new run, not a continuation of
the rejected checkpoint. The 6/6 development requirement and all other gates
are unchanged.

**Eight-epoch result: rejected.** The run finished with exit 0; frozen ETL
selection chose epoch 4. `style-eight-finetune-residual-v1/shape-screen.json`
still reports **5/6**, failing 6/6. Original ETL4 improves 1.02041pp, clean ETL4
0.45351pp; ETL5 loses 0.04859pp and ETL7 loses 0.08695pp in both views. Native
logit delta is 3.96e-6. Model SHA:
`cd12730bcc77e5fe1495e11d690c213bf6fb7a13cd2b21902538a32a971677a1`.
All eight checkpoints and `style-eight-v1-remote.log` are local. No later
checkpoint was substituted based on loop scores. No model was deployed and
no remote training remains. The next justified comparison is affine-only
synthetic augmentation, retaining thickness augmentation for scans, because
the separate morphology diagnostic exposes small-loop sensitivity.

`tools/check-kana-style-gates.py` combines the original and additional style
gates after a candidate passes its early screen. It checks original positive
controls, adapted controls, and new individual >=0.99 non-kana cases, rather
than allowing a new false positive to be hidden by removing an old one. Gate
reports now include candidate model hashes to prevent mixing evidence from
different models. Passing remains eligibility for confirmation, not deployment.

`tools/project-kana-audit.py` allows one expanded-corpus inference run to supply
the original-corpus checks too. It verifies corpus hashes, exact subset strokes
and metadata, audit coverage and prediction metadata before projection. On the
existing deployed-model audits, projected aggregate and per-character metrics
exactly match the separate original audit (`audit-projection-check-v1/`).

`tools/check-kana-terminal-loop.py` varies the embedded る terminal suffix around
inspected source point 53 (`[152,163]`), keeping the preceding stroke and anchor
fixed. It tests seven horizontal loop scales, three vertical aspect ratios and
three rotations, plus three rotated ろ references. The 66-row `terminal-loop-v1/`
diagnostic includes duplicate zero-scale cases across aspect ratios; these are
not independent examples. All modified forms have unknown validity and are
excluded from labelled training. Zero-scale cases are explicit omission probes.
Unit checks cover identity, collapse, fixed prefix and immutable inputs.

At zero and 20% horizontal loop scale all nine cases per scale become ろ. At 35%,
six become る and three become ろ (the −9° rotations). At 50%, 75%, 100% and 125%,
all nine cases per scale become る. This is a decision-boundary probe, not an
accuracy score or justified grading cutoff. Visual inspection of
`kana-artifacts/kana-review/terminal-loop-v1/size-sweep.png` confirms that at 20%
the raster loop becomes a small terminal blob, while larger scales preserve a
visible loop. The outer body still has the embedded reference's angular style;
shrinking its terminal loop alone does not recreate the rounded AnimCJK body
that failed earlier. Future training must cover shape/style as well as size,
and must not label collapsed loops as correctly written る.

The subsequent predeclared experiment is
`tools/fixtures/kana-beginner/shape-trial-v1.json`: four epochs from the deployed
residual, learning rate 5e-5, otherwise the same ETL/synthetic distillation recipe
and ETL-only checkpoint selection. `tools/prepare-kana-shape-trial.py` adds 64
shape variants per class from embedded references only, keeping all 92 classes
balanced and the previous 1,104 controls (6,992 synthetic training examples total).
Another 16 variants per class use a separate seed and never enter training.
`shape-trial-prepared-v1/` archives them with parameters and hashes; array-by-array
checks confirm every ETL input/label and previous synthetic example is unchanged.
Neither AnimCJK hiragana failures nor KanjiVG enter the new training data.

Original promotion gates remain required, with additional preservation checks
against the deployed residual: at most 0.25 percentage-point loss in each ETL
validation group, all original 2,208 controls still correct, no reduction on the
clipped controls, and improvement on the six observed small-loop variants. No
additional >=0.99 non-kana case is allowed. Independent confirmation and runtime
parity remain required before any deployment. Broader transforms are engineering
training assumptions, not teacher-certified examples or human accuracy evidence.

**Result: rejected.** The four epochs finished on nixos-server (about 31 seconds
each), and ETL selection chose epoch 4. `shape-finetune-residual-v1/` and
`shape-v1-remote.log` are copied locally. The early checker
`tools/check-kana-shape-screen.py` reports original ETL4 +1.02041 percentage
points, clean ETL4 +0.45351, ETL5 unchanged, and ETL7 −0.02174 in both views
against the deployed residual. Native synthetic-logit delta is 2.58e-6.
However, the required observed-loop improvement failed: **2/6 versus 3/6** for
the deployed model. `shape-screen.json` records this rejection and input/model
hashes. The candidate SHA is
`0d10a621baa45319a02ece113675996495bfa0d8ae1fea227d44615815a20eba`.
No broader promotion gate pass is claimed, no confirmation was run, and the
model was not deployed. Broader embedded-reference transforms alone did not
solve the small-loop problem; the next data experiment should change loop
geometry/scale while retaining valid confusable controls.

`tools/check-kana-loop-robustness.py` separates rotation from path sampling for
る/ろ across the embedded, KanjiVG and clipped AnimCJK references. It is a targeted
diagnostic after observing failures, not a new accuracy benchmark. Archived
`loop-robustness-v1/` and `loop-robustness-v2/` preserve inputs, predictions and
source snapshots. The visual comparison is
`kana-artifacts/kana-review/loop-robustness-v1/strokes-and-raster.png` (SVG alongside).
Visual inspection confirms that the small loop remains in the failed controls
and their actual native 64-pixel rasters.

All six canonical source/character combinations recognize correctly under nine
rotations from −12° to +12° when original path sampling is retained. Resampling
AnimCJK る to only 16 points produces eight failures in nine rotations; 32, 48 and
96 points all pass those pure-rotation probes. The other five source/character
combinations pass every sampling density and rotation in this sweep.

For the six original combined-transform controls, keeping identical rotation,
scale, shear and arc-length wobble parameters but increasing sampling density
changes る recognition from 3/6 (32 or 48 points) to 4/6 (96 or 192 points).
Variants 3 and 5 still become ろ. Thus sparse sampling contributes to sensitivity
but does not fully explain the model's small-loop failure. The replay verifies
the 32-point reconstruction against archived coordinates within 2e-6, allowing
their six-decimal serialization. No deployed preprocessing, thresholds or model
weights changed. A subsequent training trial should develop broader small-loop
variations and retain ろ/confusable negatives, while treating these already-seen
failures as development diagnostics and preserving all promotion gates.

### Omission experiments

`tools/build-kana-whole-omissions.py` now preserves the mapping from animation
trajectories to visible fragments. `whole-omissions-corpus-v1/` contains the
unchanged 289-entry geometric bank, 92 clipped canonical controls, and 228
trajectory-omission probes (120 hiragana, 108 katakana). A deletion removes every
fragment for the selected trajectory, rather than just one clipped segment.
Fourteen hiragana have at least one trajectory split into multiple fragments.
`trajectory-groups.json` preserves outline and fragment indices. Identical shared
medians are grouped together; none occur in the pinned basic-kana records, though
a synthetic fixture verifies this case. Nonidentical animation paths can still
describe pieces of one handwritten stroke, so this is a trajectory-level proxy,
not a certified human stroke-count dataset.

The replay (`whole-omissions-residual-v1/`) recognizes all 92 canonical controls.
All 13 empty one-trajectory omissions produce no CNN candidates, no geometry
acceptance, and no repair flag. The unchanged geometric policy
(`geometric-whole-omissions-v1/`) flags 6/120 hiragana omissions, with five matching
parent, and 15/108 overlapping katakana omissions, all matching parent. No
canonical control is flagged. These are source-parent retrieval counts, not
verified intent or correctness. Programmatic checks verify that every probe
removes exactly its recorded fragment group and retains unknown validity.

`tools/build-kana-clipped-corpus.py` extends the frozen corpus without rewriting
its original bytes. `beginner-clipped-corpus-v1/` contains 7,259 cases, adding
552 controls per script plus 1,432 corruption probes from the pinned AnimCJK
outlines and clipped medians. All appended samples are evaluation-only.
Embedded katakana already comes from that same AnimCJK commit, so its new group
is explicitly named `animcjk_clipped_katakana_overlap`. Only hiragana adds a
different reference source to the repair bank; earlier source diagnostics have
already examined it, so this is not a pristine holdout or independent writers.

Clipping can split one authored animation stroke into several visible fragments.
The existing corruption generator operates on those fragments, so these probes
include partial omissions rather than exclusively omitted whole strokes. Their
validity and intended identity remain unknown. A subsequent whole-stroke corpus
must preserve the mapping from original strokes to clipped fragments rather than
guessing a stroke count from the extracted paths. The corpus archives generator,
clipper and SVG parser snapshots with source hashes.

The deployed-model replay (`beginner-clipped-residual-v1/`) recognizes **549/552
clipped hiragana controls (99.4565%)** and **552/552 overlapping katakana controls**.
All three vision misses are る read as ろ under `mild_shape_1`, `_3`, and `_5`;
one wrong top score is 99.12%, reinforcing that softmax is not writing validity.
The unmodified canonical る is recognized. These samples expose a local shape
robustness gap without changing the deployed model or preprocessing.

The frozen geometric policy (`geometric-repairs-clipped-v1/`) flags zero added
positive controls. On clipped hiragana it flags **6/125 omission probes**, five
matching their source parent, plus one other corruption probe with matching
parent. Katakana overlap yields 15/104 omission flags, all with matching parent;
this is not third-source generalization. Original flags and candidate ordering
remain unchanged after adding the evaluation rows; float32 matrix calculations
differ by at most 2.98e-8 in reported repair distance. Coverage on the
additional hiragana source remains too low for general correction feedback.

An additional experiment, `tools/check-kana-geometric-repairs.py`, compares
symmetric raster Chamfer distance to embedded canonical forms and their
single-stroke deletions. It measures the average distance between ink sets in
both directions, so reversed order, split strokes and full retracing do not
inherently change the metric. It uses actual native rasterization. Its numerical
policy was fixed before the first output: maximum distance 0.05, edit/valid ratio
0.65, parent margin 0.003, and CNN top-1 agreement. KanjiVG remains evaluation-only.

`geometric-repairs-v1/` flags **19/197 KanjiVG omission probes**, with **18 matching
their source parent**, versus 12 flagged / 7 matching for the feature-space v2
below. Both flag zero of the 2,208 positive controls and zero of 26 procedural
non-kana probes. Geometry alone proposes defects on 16 valid KanjiVG controls,
including valid ニ interpreted as an incomplete エ; CNN identity agreement blocks
these flags. The remaining wrong-parent omission is `kanjivg:3086:omit_0`: a
fragment derived from ゆ looks like an incomplete り to both methods. That is
ambiguity, not proof that either intended character is known.

The geometric method also flags five other KanjiVG corruption probes, four with
matching parent identity. This does not verify the stated defect: a displaced or
truncated stroke can resemble an omission. Embedded deletion scores are bank
self-retrieval, not generalization. No method here yet warrants definitive
“incorrect stroke” feedback. Next validation should use a third reference source
and new transformations before integrating tentative feedback. Distance tests
cover identical ink, symmetry, duplication and a known one-pixel translation.

`tools/check-kana-repair-hypotheses.py` explores a bank of valid embedded references
and single-stroke deletions in the CNN's feature space. It searches the full script
without a requested answer. Deleted prototypes supply possible parent identities
and missing-stroke indices; they are not proof that the query is malformed.

The first policy found 48 public-source omission hypotheses, 42 sharing the
generator's parent label, but falsely flagged 16 positive controls: valid ナ and
ニ looked like incomplete チ and エ. Requiring the proposed parent to agree with
the CNN's top identity removed those positive-control false flags. That conservative
v2 flags only 12/197 public-source omissions, with seven sharing their generator's
parent. Both figures are lineage retrieval, not verified intent accuracy: an
omitted stroke can make a different valid kana, and some remaining shapes are
intrinsically ambiguous. This is not ready for definitive correction feedback.
The v1/v2 artifacts remain separate; the public controls are now regression cases,
not fresh holdout. Exact embedded deletion retrieval is explicitly not a
generalization result, because those prototypes are in the search bank.

### Valid alternatives and voiced marks

`tools/build-kana-answer-confusions.py` adds 652 challenges. In 328 cases the
writer supplies a complete confusable kana instead of the requested one; the
new CNN identifies the drawn character in all 328, without receiving the prompt.
Another 324 cases add procedural dakuten or handakuten to base references.
The 92-class CNN returns the unmarked requested base in 299/324 cases, including
224 at >=99% softmax. Geometry even accepts the requested base in 33 of these.
These are outside-basic-inventory challenges, not non-kana negatives; the result
shows why a future grader must not silently ignore voiced marks.

`tools/kana_marks.py` proposes small upper-right mark components directly from
strokes. The body is recognized separately, then Unicode composition checks
whether the mark belongs with that body. Pen order/direction do not matter.
The first version used longest-side normalization for both positional axes,
incorrectly excluding marks on narrow glyphs. V2 uses each axis's actual extent
for position while keeping size/length checks relative to the longest side.
Tests cover this defect, similarity transforms, order/direction, circles, and
non-finite coordinates.

The v2 adapter composes **324/324 procedural marked forms correctly**, with no
wrong compositions on 2,208 unmarked controls. Raw pinned AnimCJK marked references
score 51/54; all marks are detected, but three bodies are uncertain or wrong.
The base CNN's score threshold is not relaxed to hide these failures.

The source audit confirmed an additional extraction problem already hinted at in
the older AnimCJK split-loop notes: animation medians can extend outside the visible
glyph. For example, the pinned [は SVG](https://raw.githubusercontent.com/parsimonhi/animCJK/ec5e17cca76c87587790bcbce5ea0b4d4fb753d6/svgsJaKana/12399.svg)
clips two shared-animation pieces to different filled outlines; replaying their
unclipped medians draws an invisible extra stem far below the character.
`tools/kana_animcjk_clip.py` now derives visible median fragments using the actual
outlines and nonzero fill, retaining separate runs instead of connecting gaps.
Synthetic tests cover clipping boundaries, holes, separated regions, and screen
orientation. This is a new geometry proxy, not recovered human stroke timing.

With clipped source geometry, the mark adapter composes **52/54 public marked
forms correctly**, with no wrong compositions; the remaining abstentions are the
underlying は body in ば and ぱ. A separate tentative-reading diagnostic
(`marks-clipped-tentative-v1/`) identifies all 54 correctly: those two bodies rank
は first at 87.69%, below the unchanged 90% acceptance threshold. Thus 54/54 is
tentative identity, while accepted composition remains 52/54; neither measures
stroke correctness. No tentative or accepted wrong composition appeared on the
2,208 unmarked controls. Earlier diagnostics are under `marks-v1/`, `marks-v2/`,
and `marks-clipped-v1/`; raw source bytes and old results are preserved.
Neither mark composition nor missing-stroke feedback is deployed yet.
