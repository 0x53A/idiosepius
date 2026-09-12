# Kana handwriting — reboot handoff

Updated **2026-09-12**. Start here when continuing this work; detailed evidence
and experiment history are in [KANA-VISION.md](KANA-VISION.md).

**Blocked pending independent evidence:** the absence of completed reviews and
modern user strokes persisted across three continuations and was rechecked.
`~/idiosepius/kana-samples` does not exist; no completed `idiosepius-kana-review`
export exists under the review artifacts. All five recent remote run exit files
are 0, and their selected checkpoints were evaluated and rejected. No remote
training wait remains. The goal is not complete: reliable malformed-writing
feedback and near-100% performance on the actual beginner-input scenario remain
unverified. Resume with real stroke captures and independent writing-validity
judgments, or another concrete source of evaluation evidence. The 44-case queue,
capture flow and review validator are ready; do not manufacture labels or waive
gates to declare completion. Deployed weights are unchanged from the confirmed
initial residual promotion.

**Review validation ready:** `tools/check-kana-reviews.py` verifies downloaded
review JSON against the exact queue and ink, preserving aliases and reviewer/
reveal provenance. It excludes unknown/ambiguous judgments from definite
validity counts and archives original bytes separately; no training/database
import occurs. Self-tests and binding against the current 44-task queue pass.
Actual reviewed judgments are still absent. The invocation is documented in
KANA-BEGINNER.md. Do not convert engineering observations into adjudicated labels.

**Review queue ready:** `kana-artifacts/kana-review/annotation-queue-v3/index.html`
contains 44 unique inks (four flagged parent disagreements, 20 other flagged
cases, 20 canonical controls). Exact duplicates are removed and all source aliases
retained; the two truncated フ disagreements were identical overlapping-source
ink. It starts blind and unreviewed, records separate
drawn/possible-intended labels and valid/invalid/ambiguous judgments, and exports
hash-bound sidecar JSON. Browser interaction/export checks passed; artificial
test judgments were discarded. The actual queue is unreviewed. Generator:
`tools/build-kana-review-queue.py`. Direct offline browser checks pass.
`engineering-observations.json` records informed assistant observations on the
four unique disagreements without assigning validity/intent. They remain
ambiguous and are not independent review labels. Next: collect
defensible review evidence; do not treat source lineage or unknown/ambiguous
reviews as certified writing validity. No model or training change occurred.

**Capture annotation correction:** new native/browser exports mark ordinary
captures `writing_validity: "unknown"`, keeping intended `expected` separate.
Explicit invalid-ink captures carry `invalid`. The paired corpus evaluator now
emits report version 2 with intended-identity metrics; it no longer labels every
prompted attempt as valid ink. Legacy kana-labelled samples default to unknown;
reviewed invalid writing can retain its intended label. Browser save/ZIP/offline
checks and corpus accounting/replay pass. All 27 native canvas tests pass;
log `capture-validity-native-tests.log`. The subsequent native reload fix
preserves supplied validity on unchanged ink/intent, discards it when either
changes, and rejects contradictory imports. All 29 canvas tests pass; see
`review-validity-native-tests-v2.log`. Reviews no longer silently become unknown
when an unchanged native sample is re-exported.
No training or model change occurred. Next: use this distinction when evaluating
missing-stroke/false-positive feedback, and add actual reviewed user strokes when
available; synthetic parent lineage alone cannot establish writing validity.

**Synthetic-affine comparison finished and rejected:**
`style-affine-finetune-residual-v1` completed on nixos-server with exit 0;
wrapper/child are terminal and artifacts/log are copied locally. ETL selection
chose epoch 4. It still scores 5/6 and fails the frozen gate. Original ETL4
+0.68027pp, clean ETL4 +0.45351pp, ETL5 unchanged, ETL7 −0.08695pp; native parity
passed. No model changed and no remote training remains.
Protocol: `style-affine-trial-v1.json`, four epochs, shared style preparation.
Only synthetic thickness augmentation is disabled; affine transforms, scan
augmentation, RNG consumption and all gates are preserved. Thirty-seed checks
confirm exact old defaults and paired random states. `shape-screen.json` records
the rejection. Stop further tuning against these same six development probes
for now: multiple controlled trials plateau at 5/6. Next priority is separating
intended identity from verified writing validity in capture/evaluation data,
and expanding error-feedback evidence. The remaining る failure stays archived.

**Style, weight and eight-epoch trials rejected; no remote training live:**
`style-finetune-residual-v1` completed with exit 0, artifacts copied locally.
Epoch 4 improved the observed loops to 4/6 from 3/6, but failed the required 6/6;
see its `shape-screen.json`. Original ETL4 +0.90703pp, clean ETL4 +0.34013pp,
ETL7 −0.08695pp, ETL5 unchanged; native parity passed. No deployment.
The weight trial completed with exit 0 and artifacts copied locally. It improves
the loops to 5/6, still below the 6/6 gate. Its remaining miss is `mild_shape_3`
(ろ 74.75%, る 24.75%). ETL changes are within the frozen limits; parity passes.
`style-eight-finetune-residual-v1` completed with exit 0 on nixos-server;
its wrapper/child are terminal and artifacts plus log are local. ETL selection
chose epoch 4, which still scores 5/6 and fails the gate. Original ETL4 +1.02041pp,
clean ETL4 +0.45351pp, ETL5 −0.04859pp, ETL7 −0.08695pp; native parity passed.
The deployed model is unchanged. This run used eight epochs with cosine decay spanning
eight epochs, from the deployed initialization; it is not a resumed checkpoint.
Protocol: `style-eight-trial-v1.json`. All other weight-trial settings and all
gates remain. The prior weight trial changed only synthetic supervised weight
from 0.25 to 0.50, keeping data, initialization, seed, epochs and all gates fixed.
Protocol: `style-weight-trial-v1.json`. The prior style trial differs from the rejected shape trial only in reference
style coverage: half of added hiragana variants now use clipped AnimCJK. That
source is explicitly development/training data from this trial onward, not
independent evaluation. KanjiVG and ETL validation/test remain out of training.
Prior protocol: `tools/fixtures/kana-beginner/style-trial-v1.json`; shared data:
`style-trial-prepared-v1/`. After completion copy the run locally and run the
early checker with `--required-loop-correct 6`, then all frozen gates if it passes.
The deployed model remains unchanged. Statements below about no remote jobs
describe previous completed trials.

Next experiment: isolate morphology on synthetic strokes. The diagnostic
`loop-morphology-v1/` shows that actual training-style 3×3 dilation changes four
previously correct AnimCJK る references/variants to ろ. Original parent recall
is 15/18, dilated 11/18, eroded 14/18. NumPy operations exactly match PyTorch
including padding. This is sensitivity evidence, not proof of invalid ink or
causation. A predeclared affine-only synthetic augmentation comparison can keep
ETL scan augmentation and all gates unchanged. No such training is running yet.

**Shape continuation finished and rejected:** `shape-finetune-residual-v1`
completed four epochs on `nixos-server` under
`~/kana-research/stroke-trial-20260912/`. Remote wrapper/child are terminal;
`shape-v1.exit` is 0 and all artifacts plus the log are copied locally. Epoch 4
was selected by the frozen ETL rule. `shape-screen.json` fails: six observed
small-loop controls fell from 3/6 to 2/6 despite ETL improvements. No confirmation
or promotion is warranted; the deployed model remains unchanged. The frozen
protocol is `tools/fixtures/kana-beginner/shape-trial-v1.json`; prepared arrays
are `shape-trial-prepared-v1/`, SHA
`6a9804837534017f4f6649f8e299d5bd83a0072d42dd959d94f1096f66bd247d`.
Four epochs from the deployed model use 6,992 embedded-source synthetic controls;
1,472 separately seeded transforms are excluded from training. ETL arrays are
unchanged. Next: develop actual small-loop shape/size variation rather than only
broader whole-character transformations. The observed AnimCJK failures remain
development diagnostics, not fresh holdout evidence. No remote training remains.

**Active continuation, 2026-09-12:** the user requested an ongoing recognition
and beginner-error loop. Read [KANA-BEGINNER.md](KANA-BEGINNER.md) first for the
4,723-case stroke/error corpus, per-kana hypotheses, baseline results, host
benchmarks, and frozen fine-tuning protocol. Both eight-epoch trials finished:
the residual candidate passed initial gates and both candidates reached 100% on
the 2,208 positive stroke controls. The plain candidate narrowly missed the
0.5-point gain gate (0.497). Exact residual reproduction and alternate-seed,
alternate-initialization confirmation **both finished and passed**, including
native/Wasm parity. No remote training remains running. Server artifacts are in
`~/kana-research/stroke-trial-20260912/`. Local runs and `gates.json` are under
`kana-artifacts/kana-training/stroke-finetune-{plain,residual}-v1/`.
The first residual trial is now the embedded model. Core inference tests, native
CLI build, release web build, browser canvas checks, worker parity and offline
update checks all passed. Logs are `stroke-model-*.log` under the training archive.
The 92-reference worker replay had maximum score delta 3.64e-12 and desktop
round-trip latency 21.85 ms median / 38.80 ms p95 (not an iPad measurement).
frost-8000 was offline and its SSH connection timed out.

The three historical fresh 30-epoch training runs and their evaluations finished
on September 9; the new remote trials above are separate. The working tree
contains substantial uncommitted and untracked work, including the kana code
and documentation; preserve it when continuing.

## Current result

The browser and native canvases run geometry and vision classifiers side by
side. The current embedded model is **1,989,236 bytes**, from
`kana-artifacts/kana-training/stroke-finetune-residual-v1`, selected epoch 3.
Its binary is `crates/core/assets/kana_vision.bin`, SHA-256:

```text
adfe3ccbc3de26e15d8cfb7e1ba132c0a1aab5eb77f6696fe661d4c92a101d2c
```

The old 777,964-byte model and its complete report/golden fixture are preserved
in `kana-artifacts/model-releases/pre-stroke-finetune-20260912/`, with file hashes.
The shipped report now describes the current weights and explicitly has no test
evaluation for them. The current corrected/original ETL4 validation scores are
91.50%/85.94%, ETL5 99.61%, ETL7 98.96%; both public-reference control sets score
100%. These are synthetic/scan-proxy results, not modern pen accuracy.

Historical comparison below: before the new stroke fine-tuning, neither residual
candidate cleared the predeclared promotion gates. These are **validation
stroke-proxy** accuracies; canonical results use the known script:

| Model | Corrected ETL4 | Original ETL4 | ETL5 | ETL7 | Canonical |
|---|---:|---:|---:|---:|---:|
| Deployed | 90.02% | 86.39% | 99.37% | 98.24% | 88/92 |
| Fresh plain | 89.57% | 83.11% | 99.42% | 98.20% | 88/92 |
| Fresh residual, seed 20260908 | 90.93% | 84.35% | 99.51% | 98.74% | 91/92 |
| Fresh residual, seed 20260909 | 90.82% | 84.24% | 99.51% | 98.46% | 91/92 |

Each residual binary is 1,989,236 bytes, comfortably within the user's roughly
10 MB budget. Their original ETL4 regression exceeds the allowed one point.
The deployed **test** proxy figures remain 76.98%/99.39%/99.16% for ETL4/5/7;
do not compare that 76.98% directly with corrected validation scores above.
No test evaluation was generated for these three fresh candidates.

Actual iPad/Pencil accuracy remains unmeasured. CNN scores are uncalibrated
and can be very confident on invalid ink. Geometry still has one known rejected
synthetic `マ` outside its top five. Neither classifier updates study progress,
and reference-shape distance is not a handwriting-quality grade.

## Saved artifacts

Everything below is under **`kana-artifacts/kana-training/`**, outside Cargo's
build directory. `cargo clean` or deleting `target/` no longer removes this
research. The directory is ignored by Git, so copy it separately when moving
to another checkout. Restricted ETL data and per-record diagnostics must stay
out of published assets.

The 2026-09-09 storage migration also preserved `kana-artifacts/kana-diagnostics/`
and `kana-artifacts/kana-review/`. `kana-artifacts/migration-manifest.json`
records all 1,480 migrated files with SHA-256 hashes; same-filesystem moves
preserved hard links without duplicating the datasets. Archived source and
report bytes were not rewritten.

Migration verification checked every saved file hash and reproduced an archived
synthetic run exactly from the new location. Cleanup/link-restoration and
conflict-refusal checks passed in a temporary directory; real build outputs
were not deleted. Evidence is in `kana-artifacts/storage-migration-*.json`
and `.log`.

Historical `target/kana-*` paths are compatibility symlinks, not extra copies.
After clearing `target`, restore them before using historical recipes:

```sh
python3 tools/kana-artifacts.py
```

The helper refuses to overwrite conflicting paths. New training runs should
use `--run kana-artifacts/kana-training/<new-name>`; the trainer's default data
path now points there too. Replay supports the preserved location and restores
compatibility links automatically. `kana-artifacts/release` is only a link to
disposable `target/release` executables, which must be rebuilt after cleaning.
The frozen preprocessing script is unchanged to preserve cohort provenance;
its standalone diagnostic output still goes under an ordinary `target/` path.

| Path | Contents |
|---|---|
| `fresh-quality-summary.json` | Completed results, verification, and next-work evidence |
| `fresh-clean-selection-v1.json` | Frozen recipe, promotion gates, final retain decision |
| `fresh-learning-curves.svg` / `.png` / `.json` | All 30 epochs, selected checkpoint markers, provenance |
| `fresh-clean-plain-v1/` | Completed plain run, best checkpoint at epoch 20 |
| `fresh-clean-residual-v1/` | Completed first residual run, best checkpoint at epoch 20 |
| `fresh-clean-residual-seed2-v1/` | Completed second residual run, best checkpoint at epoch 14 |
| `quality-clean-prepared-v1/` | Frozen corrected preparation, all 48,272 records nonempty |
| `cnn-64-etl7-v1b/` | Original preparation reused by the deployed model |
| `data/` | Official ETL4/5/7 archives and extracted data |
| `fresh-background-quantiles-v1/` | Exploratory background-threshold comparisons |
| `fresh-background-fallback.json` | Exploratory ETL4-only threshold with nonempty fallback |

Run directories retain their own configuration, source snapshots, split,
history, `best.pt`, model, and reports. Residual binary exports have separate
`portable-export.json` manifests. Do not rewrite archived reports or sources.
These three runs saved only their best checkpoint: later epoch weights cannot
be recovered from their learning curves. The local `fresh-finish-trial.py`
evaluation driver has finished; it is not a training-resume command.

The historical aggregate result is preserved in
`kana-artifacts/model-releases/pre-stroke-finetune-20260912/kana_vision_report.json`
under `fresh_training_2026_09_09`.
Never copy a run's private `golden.json` into shipped assets; shipped numerical
fixtures must contain synthetic ink only.

## Next work, in order

The user's current priority is the recognition/error loop in KANA-BEGINNER.md:
improve conservative missing-stroke
feedback, and assess the experimental mark detector. `marks-v2/` recognizes
324/324 procedural marked forms with no wrong compositions on 2,208 unmarked
controls. Raw AnimCJK medians include invisible animation extensions; the new
`kana_animcjk_clip.py` clips them to their filled SVG outlines for diagnostics,
improving the public marked result to 52/54, still with no wrong composition.
The two remaining cases rank は first but below the unchanged 90% acceptance
threshold. `marks-clipped-tentative-v1/` reports 54/54 tentative identities,
separately from 52/54 accepted compositions; neither measures stroke validity. Do not train
on the raw animation extensions or silently rewrite the old source benchmarks.
Missing-stroke hypotheses remain research-only: the conservative v2 had no false
flags on these positive controls but poor cross-source recovery. Neither error
detector is integrated in native/Wasm UI or study grading yet.

The next geometric experiment (`geometric-repairs-v1/`, implemented by
`tools/check-kana-geometric-repairs.py`) improves KanjiVG omission lineage
recovery to 18 correct among 19 flags / 197 probes, with zero flags on 2,208
positive controls or 26 non-kana probes. It compares symmetric raster Chamfer
distance to canonical and deletion references, then requires CNN identity
agreement. Geometry alone falsely flags 16 positive controls, so do not drop
that agreement gate. The remaining wrong-parent case derives from ゆ but looks
like incomplete り. Next: third-source evaluation and new transformations;
matching the source parent is still not proof that a drawn form is malformed.

The clipped-source continuation is complete: `beginner-clipped-corpus-v1/`
adds 552 positive controls per script and 1,432 corruption probes. Its native
audit (`beginner-clipped-residual-v1/`) scores 549/552 hiragana controls and
552/552 katakana controls. The three misses are mildly transformed る read as ろ.
Katakana shares the embedded AnimCJK source and is explicitly marked overlap.
`geometric-repairs-clipped-v1/` keeps zero false flags on added controls but only
flags 6/125 hiragana omissions, five with matching parent. Original flags and
candidate ordering are unchanged (repair-distance roundoff at most 2.98e-8).
Next: examine the る/ろ robustness gap and preserve original
stroke-to-fragment mappings for whole-stroke omission probes. No threshold or
model changed during this evaluation; no feedback integrated.

Trajectory-level omissions are now evaluated too: `whole-omissions-corpus-v1/`
preserves source trajectory-to-fragment mappings and removes complete groups.
It contains 228 new omission probes plus 92 canonical controls and the 289-entry
frozen geometric bank. All canonical controls are recognized and unflagged;
all 13 empty omissions abstain in both classifiers and receive no repair flag.
`geometric-whole-omissions-v1/` recovers five parents among six flags / 120 hiragana
probes, and 15 among 15 flags / 108 overlapping katakana probes. These source
animation groups still do not establish actual human pen lifts. The る/ろ
robustness gap and low error-feedback coverage remain the next targets.

The る/ろ ablation is complete (`loop-robustness-v2/`). Pure rotations −12°…+12°
pass for original paths from all three sources. Sixteen-point resampling breaks
AnimCJK る in eight of nine rotations, but the combined-transform failures persist
at higher density: 3/6 correct at 32/48 points, 4/6 at 96/192 points. The visible
loop survives in the failed strokes and actual rasters (review sheet under
`kana-artifacts/kana-review/loop-robustness-v1/`). This supports a model robustness
gap, not merely missing extracted geometry. Next training should develop wider
small-loop variations with confusable negatives; these failures are now observed
development diagnostics, not a fresh holdout. No model or thresholds changed.

The terminal-loop size diagnostic is archived in `terminal-loop-v1/`, with a
review sheet in `kana-artifacts/kana-review/terminal-loop-v1/`. Keeping the rest
of embedded る fixed, zero/20% loop width becomes ろ, 35% depends on rotation,
and 50% or larger becomes る throughout the tested aspect/rotation grid. These
are unknown-validity probes, not training labels or an acceptance threshold.
The embedded angular outer body still differs from the failing rounded AnimCJK
style; size changes alone do not supply the missing style coverage. No model
changed and no remote job is running.

The previous broader data backlog remains:

1. **Freeze better source-specific preparation.** A global stricter threshold
   blanked records and hurt ETL7. The exploratory ETL4-only variant with a
   fallback reached 91.84% validation proxies on the unchanged deployed model.
   Audit failures and nonempty guarantees before making a new preparation;
   this variant is not integrated into training or production. Preserve both
   existing cohorts and document the new protocol before selecting models.
2. **Predeclare the next training protocol.** Accuracy-aligned selection is now
   implemented as `--checkpoint-selection source-top1`: maximize the equal-source
   mean known-script validation accuracy, then minimize validation loss, then
   retain the earliest epoch. The default remains `loss` for existing recipes.
   Configuration, per-epoch keys, and archived-source replay preserve the choice.
   Freeze the new preparation and promotion gates before starting a full run.
   Do not retrospectively promote unavailable checkpoints or quietly relax the
   original-input gate.
   `--keep-checkpoints` and `--channels-last` are now available and verified
   for future runs; neither was enabled in these three completed runs.
   `--training-seed` varies training independently of the frozen cohort seed.
3. **Audit broader handwriting data.** ETL8G/ETL9G are plausible next sources;
   their archives have not been downloaded. Check record layouts, character
   coverage, terms, and actual writer metadata before claiming writer splits.
   Official references and format offsets are in KANA-VISION.md.
4. **Evaluate independent modern pen input and calibrate invalid rejection.**
   Keep the user free of testing work until the independent preparation and
   model work is ready. Historical scans and canonical templates cannot
   establish 99% accuracy on iPad handwriting.

Keep identity independent of the expected answer. Defer quality grading,
scheduler integration, and the generic WIT/plugin architecture.

## Restart and verification

From `/home/lukas/src/idiosepius`, serve the existing static build:

```sh
python3 -m http.server 8000 --directory web
# Open http://localhost:8000/kana.html
```

To rebuild and serve, use `nix-shell --run './tools/run-web.sh --release'`.
The native lab is `nix-shell --run 'cargo run -- --kana-canvas'` and needs no
study database. iPad offline/PWA testing needs an HTTPS origin.

The completed iteration passed workspace tests, no-audio and Wasm library
checks, release worker build, Chromium/WebKit/Firefox browser checks, native
and Wasm candidate parity, and synthetic training/reproduction checks. Logs
are named `fresh-*.log` under the training directory; detailed check results
are in KANA-VISION.md. Hardware iPad testing is still outstanding.

After new code changes, the main checks are:

```sh
nix-shell --run './tools/run-all-tests.sh'
nix-shell --run 'cargo check --no-default-features'
nix-shell --run 'cargo check -p idiosepius-app --lib --target wasm32-unknown-unknown'
```

Training uses Python 3.12, torch `2.14.0+cpu`, numpy `2.5.2` through `uv` inside
`nix-shell`; complete run recipes and replay commands are in KANA-VISION.md
and each run's configuration. No study database import or reimport is needed
to continue this work.
