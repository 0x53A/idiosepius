# Experimental kana vision baseline

For the completed experiments, saved artifacts, and next steps after reboot,
start with [KANA-HANDOFF.md](KANA-HANDOFF.md).

**2026-09-12:** the confirmed stroke-fine-tuned residual model is now embedded
(1,989,236 bytes). See [KANA-BEGINNER.md](KANA-BEGINNER.md) for its training,
validation, reproduction, and error audits. Historical deployed-model figures
below describe the previous weights unless explicitly dated otherwise; the new
model has not been evaluated on test records. The prior weights and full report
are archived under `kana-artifacts/model-releases/pre-stroke-finetune-20260912/`.

Research artifacts were moved from `target/kana-*` to ignored `kana-artifacts/`
on 2026-09-09 so clearing build outputs preserves them. Paths below use the new
location; archived reports retain their original paths. Restore compatibility
links with `python3 tools/kana-artifacts.py` after clearing `target/`.

The native kana canvas and `web/kana.html` now run two classifiers on the same
raw ink: the existing geometric matcher and a trained CNN. Neither uses the
expected answer to select its candidates. The CNN ranks the selected script's
46 characters. Its softmax scores are **uncalibrated**, including scores near
1; it has no invalid-ink rejection policy. The geometric acceptance policy
remains independent. Nothing updates the study scheduler.

A separate canonical-reference comparison follows the CNN's top candidate.
It reports geometric distance and relative path length. This is a first
identity-then-shape pipeline, **not a handwriting-quality grade**: acceptable
style variation, pedagogy, stroke order, and human quality labels have not been
validated. Directed distances are available in exported browser samples;
smaller input-to-reference distance means input lies close to reference ink,
and smaller reference-to-input distance means reference ink has nearby input.
They also include the matcher's tangent-direction term; they are not literal
percentages of missing or extra ink.

## Historical baseline: model and evidence (2026-09-08)

The then-selected 64×64 model has three convolution/pooling blocks (8, 16, and
32 channels), then a linear layer with 92 outputs. It has **194,396 parameters**
and a **777,964-byte** binary, down from 1,513,068 bytes for the previous model.
The portable Rust runtime supports both two- and three-block models at 32px
and 64px, without an external neural runtime or browser GPU.

Training mixes grayscale ETL4/ETL5/ETL7 scans with stroke-derived rasters from
the same training records, with affine and thickness augmentation. All 48,272
eligible records produced nonempty proxies. The shared Rust rasterizer centers
an antialiased union of segments and ignores pressure, timestamps, stroke order,
and direction; saved raw ink retains that information separately.

The selected checkpoint is epoch **24 of 30**, seed `20260908`, using AdamW
and a cosine learning-rate schedule. Checkpoint selection minimizes 92-class
validation proxy cross-entropy; reported recognition scores use the selected
script's 46 classes. The architecture decision used frozen validation scores,
canonical probes, reproduction, and Rust parity **before inspecting test metrics**.
Compared with the two-block ETL7 model, proxy validation top 1 improves from
85.37% to **86.39%** on ETL4, 99.13% to **99.37%** on ETL5, and 97.61% to
**98.24%** on ETL7. Canonical top 1 rises from 86 to 88 of 92.

ETL4/ETL5 test cohorts have been reused across experiments: they are comparison
benchmarks, not fresh final holdouts. ETL7 test records were excluded from
training and model selection. Results below are scan-domain measurements:

| Test input | Source | Samples | Top 1 | Macro top 1 | Top 5 |
| --- | --- | ---: | ---: | ---: | ---: |
| Grayscale scan | ETL4 hiragana | 882 | 83.90% | 84.00% | 91.61% |
| Grayscale scan | ETL5 katakana | 1,960 | 99.74% | 99.76% | 100.00% |
| Grayscale scan | ETL7 hiragana | 4,784 | 99.50% | 99.50% | 99.98% |
| Stroke proxy | ETL4 hiragana | 882 | 76.98% | 77.60% | 85.83% |
| Stroke proxy | ETL5 katakana | 1,960 | 99.39% | 99.43% | 99.95% |
| Stroke proxy | ETL7 hiragana | 4,784 | 99.16% | 99.16% | 99.96% |

A stroke proxy skeletonizes a scan, traces its graph, and renders it through
the canvas rasterizer. It cannot establish accuracy on human-drawn iPad ink.
ETL4 hiragana proxy top 1 improves from the previous model's 69.39% to 76.98%,
but remains well below its 86.39% validation score. Its top-five score also
slightly declines (86.05% to 85.83%). This is not ready to grade study answers.

ETL4 is split by sheet (one sheet per writer). ETL5 and ETL7 conservatively
group records by writer metadata; neither is proven writer-disjoint. The split
and source checksums are retained. See the ETL7 section for counts and caveats.

The report preserves the earlier 32px grayscale, 64px grayscale, 64px mixed,
and two-block ETL7 experiments. The selected deep model reproduced weights,
training history, and fixtures byte-for-byte in a second run on this workstation.
Those deep runs reused frozen prepared arrays; the preceding two-block ETL7
runs independently reproduced preprocessing too. Exact portability to other
software versions or hardware is not promised.

Canonical-vector probes identify **44/46 hiragana and 44/46 katakana**, with
all 92 in the top five. Remaining first-choice confusions are ひ→み, め→ぬ,
ク→タ, and セ→ヤ. These public reference trajectories are not unseen people's
writing. Native inference averages roughly 1.6 ms here. PyTorch/Rust logit
agreement is within `2.4e-5`; synthetic fixtures cover all supported shapes
without distributing ETL pixels.

Browser checks cover Chromium pen events, WebKit and Firefox drawing, saved
provenance, lossless ZIP export, offline Wasm, failed saves, cancelled strokes,
and hiding stale predictions. Five exported Chromium/WebKit/Firefox samples
replay in native Rust with identical candidate order, fingerprint, and scores.
Safari/Apple Pencil hardware remains untested.
Local HTTP works for drawing; offline installation on an iPad requires HTTPS
rather than a workstation's plain HTTP LAN address. See the
[service-worker secure-context requirement](https://developer.mozilla.org/en-US/docs/Web/API/Service_Worker_API#service_worker_concepts_and_usage).

Paired validation replays the same raw proxies through geometry and vision.

| Validation proxy | Samples | Geometry top 1 | Vision top 1 | Only geometry correct |
| --- | ---: | ---: | ---: | ---: |
| ETL4 | 882 | 270 | 762 | 5 |
| ETL5 | 2,058 | 1,275 | 2,045 | 2 |
| ETL7 | 4,600 | 1,973 | 4,519 | 11 |

Geometry remains independently useful, but its accepted ETL4/ETL5 predictions
include 6/38 incorrect identities. Agreement and geometric reference distance
are diagnostic signals, not validated abstention rules or penmanship grades.

The embedded weights, aggregate confusion matrices, source SHA-256s, split
SHA-256, selection epoch, software versions, and limitations live under
`crates/core/assets/kana_vision*`. Full per-record splits, checkpoints, prepared
images, and ETL-derived arithmetic fixtures stay under ignored `kana-artifacts/`.
The FNV fingerprint saved with predictions detects model changes; it is not a
security signature. The report's `binary_model_sha256` identifies the deployed weight artifact;
`model_sha256` identifies the equivalent debug JSON export.

## ETL7 expansion

The trainer also accepts `--etl7`, adding AIST's later hiragana collection.
The official archive contains four M-type files with **33,600 records**, despite
the overview page listing 16,800. Of these, 32,200 belong to the 46 basic kana;
standalone dakuten and handakuten are excluded. The decoder pins every file's
size and SHA-256 and follows the [official M-type format](https://etlcdb.db.aist.go.jp/etlcdb/etln/form_m.htm).

The archive repeats sheet numbers across its large/small files and contains
multiple sheets with identical writer metadata. All records sharing sex, age,
industry, occupation, and collection date are grouped across **all four files**.
This gives 122 conservative groups; it is not a verified writer-ID mapping.
The [AIST collection description](https://etlcdb.db.aist.go.jp/database-development/)
is the source for the collection's writing conditions. Image-quality metadata
is never treated as a penmanship grade.

| Source | Training | Validation | Test benchmark |
| --- | ---: | ---: | ---: |
| ETL4 | 4,116 | 882 | 882 |
| ETL5 | 6,174 | 2,058 | 1,960 |
| ETL7 | 22,816 | 4,600 | 4,784 |

ETL4/ETL5 keep their previous split assignments. ETL7 uses the same seeded
70/15/15 allocation by group count, fixed before training. All 48,272 eligible
records produced nonempty stroke proxies. The test records never enter
training or checkpoint selection.

The optional `--depth 3` model adds a 16→32 convolution and pooling block,
then uses a smaller final linear layer. At 64px it has 194,396 parameters,
versus 378,172 for two blocks. The binary header is `IDKANA02` for three
blocks; `IDKANA01` remains supported for two. Both carry the little-endian
input size, 92 Unicode labels, and ordered f32 parameters. Independent
synthetic arithmetic tests cover both depths at both supported input sizes.

`--prepared-from RUN` reuses a compatible run's frozen arrays and split for
architecture experiments. It verifies the rasterizer, seed, input modes,
reader version, shapes, and labels, and records the preparation's hashes.
Reusing arrays does **not** independently reproduce preprocessing.
`check-kana-reproduction.py FIRST SECOND` checks weights, histories, splits,
configuration, source snapshots, fixtures, and each array inside the NPZ.

## Reproduce and evaluate

Accept AIST's terms and obtain ETL4/ETL5/ETL7 from the [official download
page](https://etlcdb.db.aist.go.jp/download2/). Extract `ETL4/ETL4C` and
`ETL5/ETL5C` and the four `ETL7/ETL7*` files below `kana-artifacts/kana-training/data/`.
The trainer checks the exact unpacked checksums and refuses to overwrite a run. Raw data and prepared
images must not enter the repository or its hosted `web/` directory.

```sh
nix-shell --run 'cargo build --release -p idiosepius-core --bin kana-rasterize --bin kana-vision'
nix-shell --run 'uv run --python 3.12 --with torch==2.14.0+cpu --with numpy==2.5.2 --index https://download.pytorch.org/whl/cpu python tools/train-kana-vision.py --run kana-artifacts/kana-training/my-run --epochs 30 --threads 3 --side 64 --ink-proxy --etl7 --depth 3'
```

This trains only; it never promotes weights. Review validation failures and
retain the experiment history before selecting a replacement. Promotion copies
`model.bin` to `crates/core/assets/kana_vision.bin` and `synthetic-golden.json`
to `kana_vision_golden.json`, and updates `kana_vision_report.json` with the
selected report, attribution, and experiment history. Never copy prepared data,
`golden.json` (which contains ETL pixels), or the per-record split into assets.
After an intentional replacement, rebuild the binaries and run:

```sh
python3 tools/check-kana-vision.py kana-artifacts/kana-training/my-run
python3 tools/check-kana-paired-etl.py kana-artifacts/kana-training/my-run
nix-shell --run './tools/run-all-tests.sh'
nix-shell --run './tools/build-web.sh --release'
uv run --with playwright python tools/check-kana-web.py
nix-shell --run './tools/shot.sh kana'
```

The verification script checks model/split hashes, group leakage, complete
class coverage, PyTorch/Rust arithmetic parity, and canonical-vector transfer.
The ordinary test fixture is synthetic and is exported by the trainer; keep
it with the selected weights. Do not change a tolerance to hide a mismatch. CPU determinism is enabled, but exact
cross-version/cross-platform reproducibility is not promised by
[PyTorch](https://docs.pytorch.org/docs/2.14/notes/randomness.html).

`kana-vision` accepts a JSON array of saved sample documents on stdin. Every
sample needs `script` or `recognizer.script_scope`; expected labels remain
report annotations. It emits both rankings, reference comparisons, and timing.
For example, replay every JSON in an exported ZIP's extracted directory:

```sh
python3 - <<'PY' | target/release/kana-vision > target/kana-replay.json
import json
from pathlib import Path
print(json.dumps([json.loads(p.read_text()) for p in Path('target/my-corpus').rglob('*.json')]))
PY
```

## Captured-corpus and browser checks

`python3 tools/check-kana-corpus.py target/my-corpus --output target/corpus-report.json`

The current report schema is version 2: expected kana annotations measure
intended-identity matching, not valid handwriting. New captures explicitly carry
`writing_validity: "unknown"` unless marked invalid; legacy kana-labelled captures
also default to unknown. See [KANA-BEGINNER.md](KANA-BEGINNER.md) for the annotation
rules and renamed intent metrics. Historical reports are preserved unchanged.
replays original samples through both classifiers. It deduplicates overlapping
exports and rejects conflicting labels for identical ink. The report separates
valid, invalid, and unlabeled input; counts errors unique to either model; and
shows agreement precision and vision score bins. These are descriptive results,
not calibrated thresholds. Do not randomly split one person's repeated samples
into training and test sets and call that writer generalization.

Both replay CLIs accept `--model path/to/model.bin`, so candidate weights can be
checked before installation. `check-kana-vision.py RUN --candidate` checks Rust
arithmetic and canonical probes against that run's weights. In the pinned
training environment, `compare-kana-vision.py COHORT MODEL_RUN...` compares
checkpoints using only COHORT's frozen validation records, including when the
network architectures differ but their rasterizer matches.

The Chromium pen suite now injects failed IndexedDB writes and failed count
refreshes. Failed writes preserve the drawing and prompt position; a successful
commit followed by a failed count refresh remains a successful save. Undo also
removes cancellation metadata for the discarded stroke.

`tools/check-kana-engines.py --engine webkit` and `--engine firefox` exercise
mouse PointerEvents, the real Wasm worker, storage, ZIP download, and offline
reload after stopping the test HTTP server. They use fresh profiles. Install
Playwright's matching browser binaries, or point `PLAYWRIGHT_BROWSERS_PATH` at
the Nix `playwright-driver.browsers` output. The tested combination here is
Python Playwright 1.61.0 with Nix's Playwright browser set 1.61.1.

Linux WebKit 26.5 passes with the server stopped. Playwright's emulated offline
mode instead fails navigation even for a minimal service worker returning a
constant HTML response; `--offline-mode browser` keeps that path reproducible.
This does not establish Safari/Apple Pencil behavior. Playwright documents its
[WebKit platform differences](https://playwright.dev/python/docs/browsers#webkit).

The service worker imports a generated inventory whose hash covers the shell,
Wasm files, and hashed snippet modules. Model-only changes therefore trigger a
new offline install. Each deployment scope owns its cache; an incomplete install
preserves the previous cache. `tools/check-web-update.py` checks these cases.

## Next experiments

1. Address hiragana writer/style generalization and the scan-to-stroke gap.
   Keep the original test split as a fixed benchmark; after looking at its
   outcomes, further tuning cannot be advertised as a fresh test.
2. Add a separate modern hiragana source with documented rights and writer
   grouping, and compare on a genuinely untouched source or writer cohort.
3. Evaluate captured iPad strokes when available, including missing strokes,
   extra strokes, non-kana, and ambiguous pairs. Keep the raw input so all model
   versions can be compared on identical traces.
4. Calibrate identity abstention on independent valid and invalid ink. Only
   then consider lesson integration. Quality feedback needs its own rubric and
   labeled examples, rather than relabeling a distance as a grade.

Training-data attribution: **ETL Character Database / Electrotechnical
Laboratory, Japanese Technical Committee for Optical Character Recognition,
ETL Character Database, 1973–1984.** Source format is documented by AIST's
[C-type record specification](https://etlcdb.db.aist.go.jp/etlcdb/etln/form_c.htm).
Only learned parameters, aggregate metrics, and synthetic test inputs are
included here; raw ETL records and derived image fixtures are not redistributed.

## Quality-first iteration (2026-09-08)

The model budget is approximately **10 MB**. Recognition quality, including
hard characters and source generalization, decides model selection; small size
is a secondary benefit. The initial 76.98% ETL4 proxy result does not measure
actual iPad handwriting accuracy.

`tools/audit-kana-vision.py RUN --output kana-artifacts/kana-training/audit` compares
scan and proxy predictions on validation records and writes local contact sheets.
On the deployed model, ETL4's 882 validation records contain 745 correct in both
representations, 58 correct only as scans, 17 correct only as proxies, and 62
wrong in both. ETL7 has 50 scan-only and 25 proxy-only successes. These counts
identify representation sensitivity, not proven causes of each error.

Inspection found low-contrast ETL4 backgrounds split by Otsu thresholding:
background levels 4 and 5 can become thousands of false foreground pixels while
actual ink is mostly 6–9. Skeletonization then amplifies these speckles into
false strokes. `tools/check-kana-preprocessing.py` tests a border-aware threshold
on the same validation records with the same deployed checkpoint. It preserves
the original prepared benchmark; corrected-input scores must be reported as a
separate preprocessing experiment, not as a new model's accuracy gain.

The trainer supports source-balanced, known-script cross-entropy and warm-start
widening. The comparison holds the new fine-tuning recipe constant at width 1
and width 2, rather than attributing a training-objective change to capacity.
The latter has 16/32/64 convolution channels and 400,220 parameters. Wider
models use `IDKANA03`: side, depth, width, 92 labels, then f32 parameters.
Only sides 32/64, depths 2/3 and widths 1/2/4 are accepted. Synthetic PyTorch/Rust
arithmetic fixtures cover all 12 shapes. `tools/test-kana-training.py` checks
channel replication, symmetry-breaking noise, source weighting, and script masks.

For prepared-data experiments, `tools/reproduce-kana-training.py FIRST SECOND`
runs the archived trainer in an isolated source tree. It checks archived hashes
and the warm-start checkpoint, allowing numerical reproduction while runtime
code evolves without temporarily replacing working files. Prepared arrays are
still shared: this is training reproduction, not independent data preparation.

### Additional data search

The [TUAT Kuchibue collection](https://web.tuat.ac.jp/~nakagawa/database/en/about_kuchibue.html)
contains online character trajectories from 120 writers; its
[application and terms](https://www.univcoop.jp/tuat/order/order_237.html) require
a licensing application and describe a free ten-writer research subset. The
[Nakayosi collection](https://web.tuat.ac.jp/~nakagawa/database/en/about_nakayosi.html)
has 163 writers. Neither was obtained or used in this iteration; no application
or message was sent on the user's behalf.

[7500-unique-kana-images](https://github.com/Orzelius/7500-unique-kana-images)
is generated from handwriting fonts, so it is not independent human trajectory
evidence. [inoueMashuu/hiragana-dataset](https://github.com/inoueMashuu/hiragana-dataset)
contains extracted scans, not online trajectories; the inspected repository
provides neither a clear data license nor a writer mapping. It was not added to
training. Canonical vectors, fonts, and historical scans cannot substitute for
a documented modern-writer test cohort.

The first border-aware threshold experiment improves ETL4 validation proxy
accuracy from **762/882 (86.39%) to 794/882 (90.02%)**, with the same model.
ETL5 stays at 2045/2058 and ETL7 at 4519/4600. Scan accuracy improves from
803/882 to 816/882 on ETL4. This experiment does not replace the original
76.98% test benchmark or establish improved online-ink accuracy.

Confidence diagnostics illustrate the coverage tradeoff. On the deployed
model's original ETL4 validation proxies, a score threshold of 0.99 retains
616/882 samples (69.84% coverage), with 5 errors (99.19% empirical accuracy).
Even 0.999 retains only 500/882, with 2 errors. These are valid-ink validation
observations, not a calibrated policy: they omit scribbles and unfamiliar
characters, reuse validation, and include correlated samples from writers.
The audit reports Wilson intervals as a simple descriptive check, with an
explicit warning that they do not account for writer clustering.

The known-script-loss fine-tuning trials were stopped after 10 epochs (width 1)
and 4 epochs (width 2), rather than completing their configured 12 epochs.
Their inspected best checkpoints failed the quality gates: original ETL4 proxy
validation slipped to 86.28%, ETL7 fell below 97.8%, and canonical Both-scope
accuracy fell from 88/92 to 83/92 and 84/92. Neither was deployed. These results
do not show that larger models are unhelpful; the fine-tuning formulation failed.

`--clean-scans` now enables the corrected training preparation explicitly;
`--prepare-only` freezes that cohort without training. Reuse checks include the
preprocessing version, and paired replay follows the run's recorded preparation.
The follow-up comparison retains all 92 logits in source-balanced training
(`--loss-scope all`) and uses six epochs at each width. Clean-input validation
is primary, with original-input regression checks and both canonical script
scopes. Tests remain reporting benchmarks, never model-selection inputs.

`tools/check-kana-training-pipeline.py --output kana-artifacts/kana-training/smoke-NEW`
runs a synthetic end-to-end experiment, including warm-start widening, archived
source reproduction, binary export, and Rust/PyTorch parity. It requires the
trainer's pinned Python environment but no ETL data. The first such smoke run
reproduced weights, histories, arrays, and provenance byte-for-byte.

### Outcome of the quality-first comparison

The deployed model remains unchanged. Neither completed clean-data warm-start
trial cleared the promotion gates:

| Model | Clean ETL4 proxy | Original ETL4 proxy | ETL5 proxy | ETL7 proxy | Canonical known / Both |
| --- | ---: | ---: | ---: | ---: | ---: |
| Deployed baseline | 90.02% | 86.39% | 99.37% | 98.24% | 88 / 88 |
| Same-width fine-tune | 89.46% | 86.05% | 99.51% | 98.00% | 87 / 86 |
| Twice-width fine-tune | 89.23% | 85.03% | 99.32% | 98.15% | 88 / 87 |

These are validation measurements. Both trials completed six epochs, and their
exports passed Rust/PyTorch parity. Their test metrics were not inspected for
selection or reporting. They were not repeated because neither was eligible
for deployment; the synthetic pipeline did independently test the reproduction
mechanism. This tests warm-start fine-tuning, not training a larger network
from scratch.

`tools/check-kana-tta.py` compares fixed zero-padded translations and averages
logits. On the unchanged model, the five-pass ±2-pixel policy reaches 87.41%
on original ETL4 validation proxies, 90.59% on cleaned ETL4, 99.42% on ETL5, and
98.61% on ETL7. Its mean clean-hiragana gain is 0.468 percentage points, below
the predeclared 0.5-point promotion gate. It remains an experiment; neither
weights nor live recognition policy changed. ±1-pixel averaging helps less
on the scan-derived inputs but reaches 90/92 known-script canonical references.

The next substantive model experiment is a fresh initialization on the corrected
cohort, rather than further fine-tuning weights trained on noisy proxies.
Independent modern pen trajectories remain necessary to establish application
accuracy and valid/invalid-ink rejection. A 99% empirical score among selected
valid samples is not the same target as 99% recognition of all valid drawings.

`tools/check-kana-worker-parity.py` replays all 92 public references through the
actual browser worker and native Rust, comparing candidates, model fingerprint,
and scores. It records desktop worker latency separately from classification
accuracy. No expected label is sent to the browser worker.

Final checks passed for the workspace, audio-free desktop build, all 12 supported
network shapes, synthetic training/reproduction pipeline, and Chromium/WebKit/
Firefox drawing, storage and offline operation. The 92-reference worker replay
matches native candidate order and fingerprint; the maximum score difference
is `5.9e-11`. Desktop worker round-trip timing (both classifiers and reference
comparison, under concurrent build load) was approximately 35 ms median and
80 ms at the 95th percentile; this is not an iPad latency measurement.

### Fresh training on the corrected cohort

The completed controlled comparison used `fresh-clean-plain-v1` and
`fresh-clean-residual-v1`: 30 epochs from fresh initialization on the same frozen
corrected arrays, the original sample-weighted 92-class objective, identical
augmentation, and 50:50 scan/proxy mixing. The plain model retains depth 3 and
width 1; the residual candidate uses width 2 and adds two same-shape 3×3
convolutions with an identity skip after each pooling stage. It has 497,212
parameters (1,988,848 bytes of float32 weights), with no batch-normalization
state. Selection remains proxy validation cross-entropy. All per-source
confusion matrices and accuracies are recorded every epoch. These runs use
`--validation-only`, which omits test evaluation and uses validation examples
for the private arithmetic fixture.

`--seed` continues to freeze cohort membership. `--training-seed` independently
controls initialization, batch order, and augmentation; omitting it preserves
the historical seed behavior. Synthetic checks confirmed that alternate training
seeds change weights while retaining identical split bytes, and that a repeated
residual run reproduces weights and history exactly. Wall-clock timings are
printed to logs rather than included in the deterministic history.

Residual exports use `IDKANA04`: the same bounded side/depth/width descriptor as
`IDKANA03`, followed by labels, plain-network parameters (including the linear
layer), then the residual convolution weights and biases. The research trainer
keeps its original report immutable; `tools/export-kana-residual.py` creates a
separate `portable-export.json` binding the binary to that report and checkpoint.
No export replaces the embedded model automatically.

The Rust runtime now supports both families at all 24 side/depth/width
combinations. `tools/build-kana-arithmetic-fixture.py` regenerates their synthetic
PyTorch fixtures without ETL data. The deliberately amplified width-4 residual
fixtures need a scale-aware floating-point comparison: absolute tolerance
`2e-4` plus `5e-5` times the expected logit vector's infinity norm. Measured
maximum relative infinity error was `1.91e-5`; trained candidate verification
retains its stricter `2e-4` absolute logit limit. This distinction covers scalar
versus vectorized float32 accumulation and cancellation, rather than asserting
bitwise PyTorch/Rust arithmetic.

`KanaEngine.with_model` permits isolated browser candidate evaluation without
changing embedded weights. `tools/check-kana-wasm-candidate.py` replays all 92
public references through native Rust and browser Wasm; expected labels are
excluded from inference inputs. `tools/check-kana-checkpoint-probes.py` records
uncalibrated scores on fixed grids, circles, scribbles, and empty rasters. These
are diagnostic compositions, not an independent invalid-ink benchmark or an
acceptance policy. Empty ink is already rejected before inference in the app.

`tools/check-kana-promotion.py` applies the previously frozen validation gates
to corrected and original comparisons, checking matching cohorts and checkpoint
hashes. Passing establishes eligibility only; size, alternate seeds, portable
parity, and ink diagnostics still require separate evidence.

The fresh plain control completed all 30 epochs and selected epoch 20. It failed
the frozen gates: corrected ETL4 proxy top-1 was 89.57%, original ETL4 was
83.11%, ETL5 was 99.42%, ETL7 was 98.20%, and canonical known/Both were 88/85.
Its native export verified successfully. It is not a replacement candidate.
A second residual run uses training seed 20260909 with the same cohort seed
20260908; it was launched before either full residual result was observed.
Each seed is assessed separately against the gates.

A follow-up visual audit found remaining background texture in some corrected
ETL4 proxies. `tools/check-kana-background-quantiles.py` tried border percentiles
95 and 99 as separate validation-only input diagnostics, using unchanged deployed
weights. Percentile 95 yielded ETL4/5/7 proxy scores of 90.14/99.37/98.22%;
percentile 99 yielded 91.04/99.37/96.52%. The latter produced 13 empty ETL4 and
42 empty ETL7 proxies, so it is unsuitable as a global preprocessing replacement.
An explicitly exploratory ETL4-only follow-up, reverting to the original
corrected raster whenever the aggressive raster was empty, reached 91.84%
ETL4 proxy accuracy and 93.76% scan accuracy. ETL5/7 retained their frozen inputs
and scores. These experiments change neither the frozen training cohort nor
live browser input processing; they suggest source-specific preparation work,
not an increase in measured pen accuracy. Local evidence is under
`kana-artifacts/kana-training/fresh-background-quantiles-v1` and
`fresh-background-fallback.json`.

The fixed non-kana probes expose overconfidence in the deployed CNN: five
concentric circles receive 99.44% all-92 softmax for の. The geometric classifier
rejects all four compositions in each script, including the circles. This is
useful evidence of complementary behavior, but eight synthetic script/probe
combinations do not establish an invalid-ink rejection rate on human drawings.

Two opt-in training controls are now available for subsequent experiments:
`--keep-checkpoints` retains every epoch while preserving loss-based selection;
the reproduction checker also compares those retained files. `--channels-last`
changes PyTorch's CPU convolution storage layout, with the choice recorded and
replayed by the reproduction tool. Portable tensor order and architecture are
unchanged. A synthetic width-2 residual batch benchmark under concurrent load
measured roughly 0.54 s versus 0.30 s median forward/backward time with three
threads, but this is not a full-training speed guarantee. The combined retained-
checkpoint/channels-last synthetic pipeline reproduced exactly and passed native
parity. The three full fresh runs in this comparison use their original layouts
and checkpoint-retention settings.

`tools/check-kana-paired-gain.py` reports fixed versus regressed validation
samples and descriptive bootstrap intervals over the frozen split groups. These
intervals neither correct for repeated model selection nor turn ETL5/7 metadata
groups into verified writer identities; they are not additional promotion gates.
`tools/plot-kana-training.py` generates standalone learning-curve PNG/SVG files
from aggregate histories, with the deployed checkpoint shown on corrected inputs.

For a subsequent data expansion, AIST documents 75 hiragana categories in ETL8
and 71 in ETL9, with 128×127 grayscale images in their G variants. These are
additional historical scan sources, not modern pen trajectories. The published
writer totals cover the entire datasets and must not be presented as writer
coverage for our basic-kana subset. See the [official dataset details](https://etlcdb.db.aist.go.jp/database-development/).
Their record formats need separate adapters: image bytes start at zero-based
60 for [ETL8G](https://etlcdb.db.aist.go.jp/etlcdb/etln/form_e8g.htm) and 64 for
[ETL9G](https://etlcdb.db.aist.go.jp/etlcdb/etln/form_e9g.htm). The ETL8G format
also lists an extra `ETL8G-33` with uncertain dataset numbering. File hashes,
actual metadata/grouping, eligible-label counts, and a frozen split protocol
need auditing before inclusion. Neither archive was downloaded or included in
this comparison.

### Completed fresh comparison — 2026-09-09

All three runs completed 30 epochs. The deployed weights remain unchanged.
Both residual seeds improve corrected-input recognition and public references,
but each fails the frozen original-ETL4 regression limit:

| Model | Selected epoch | Corrected ETL4 proxy | Original ETL4 proxy | ETL5 proxy | ETL7 proxy | Canonical known / Both |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| Deployed baseline | — | 90.02% | 86.39% | 99.37% | 98.24% | 88 / 88 |
| Fresh plain | 20 | 89.57% | 83.11% | 99.42% | 98.20% | 88 / 85 |
| Residual, seed 20260908 | 20 | 90.93% | 84.35% | 99.51% | 98.74% | 91 / 89 |
| Residual, seed 20260909 | 14 | 90.82% | 84.24% | 99.51% | 98.46% | 91 / 90 |

Each residual binary is 1,989,236 bytes. The first residual candidate fixes the
canonical ひ, ク, and セ errors, leaving め→ぬ in known-script evaluation. Its
corrected ETL4 gain comprises 24 fixed and 16 regressed examples. Descriptive
split-group resampling gives an ETL4 gain interval of −0.57 to +2.38 percentage
points; the mean hiragana interval also includes zero. These reused-validation
intervals do not establish a population gain, and no threshold was relaxed.

Later epochs sometimes have higher top-1 accuracy than the loss-selected
checkpoint. Their complete aggregate curves are preserved, but the selected
checkpoints follow the predeclared loss rule. Future accuracy-oriented selection
must be declared before training and checked using retained checkpoints, rather
than retrospectively selecting a favorable curve point with no replayable state.

Both residual candidates pass native/PyTorch checks and all-92-reference browser
Wasm/native parity. Maximum browser score differences are `1.50e-8` and `3.73e-9`.
For the first candidate, desktop Wasm module inference including both classifiers
and reference comparison was about 26 ms median and 54 ms at the 95th percentile
under concurrent load; this is not a worker round-trip or an iPad measurement.
Workspace, audio-free, Wasm app-library, and Chromium/WebKit/Firefox checks pass.
The new 24-shape runtime, alternate-seed smoke checks, and synthetic reproduction
with checkpoint retention and channels-last layout also pass. Exact repeats of
these rejected full training runs were not performed. The existing geometric
synthetic マ top-five miss remains; it was not waived or retested here.

No test records were evaluated by these runs. The deployed model's original
76.98% ETL4 proxy test benchmark remains unchanged. Aggregate evidence is retained
in `kana_vision_report.json` under `fresh_training_2026_09_09`, with detailed local
artifacts in `kana-artifacts/kana-training/fresh-quality-summary.json` and learning curves
in `kana-artifacts/kana-training/fresh-learning-curves.svg`.

### Accuracy-aligned checkpoint selection — 2026-09-12

Future runs can set `--checkpoint-selection source-top1` before training.
It maximizes the arithmetic mean of per-source known-script validation top-1
accuracy, computed from integer confusion counts. Thus ETL4, ETL5, and ETL7
each receive one third of the weight when all three are present, independently
of their sample counts. This is source-balanced accuracy, not per-class macro
accuracy. With `--ink-proxy`, selection uses the frozen validation proxies;
otherwise it uses validation scans.

Ties minimize the existing validation-loss objective, with an exact tie retaining
the earliest epoch. `--source-balanced` and `--loss-scope` continue to control
that loss as before. Non-finite loss stops the run. The default remains `loss`;
old archived recipes replay without receiving a flag their source did not support.
New configurations record the selection mode and every history row records its
minimization key. `best.pt` and exported weights use that same selected epoch.

This implements a selection mechanism, not a new promotion protocol or a model
improvement. The source-specific preparation still needs a full audit, and the
next run must freeze its cohort and promotion gates before training. Existing
model weights, historical reports, and test benchmarks remain unchanged.

Numerical tests cover disagreement between loss and accuracy, unequal source
sizes, replication invariance, both tie-breakers, and non-finite/empty inputs.
A three-epoch synthetic run selected epoch 1 even though loss decreased through
epoch 3, exercising the actual difference in policy. Its archived-source replay
produced byte-identical weights, retained epochs, history, and configuration;
the selected checkpoint matches its retained epoch tensor-for-tensor. The smoke
check uses `--validation-only` and verifies that neither result table includes
test evaluation. Evidence is under `target/kana-selection-smoke-20260912/`;
these synthetic build artifacts are disposable. The loss-mode counterpart in
`target/kana-selection-loss-smoke-20260912/` also reproduced exactly. After
rebuilding the native tools, both selected exports passed Rust/PyTorch parity
(maximum logit differences `6.02e-8` and `1.30e-7`, respectively).
