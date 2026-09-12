# Kana handwriting review — 2026-09-07

Follow-through: all three findings below have now been fixed. The original
review and pre-fix measurements are retained here; corrected measurements,
browser collection instructions, validation and remaining limitations are in
[`JAPANESE.md`](JAPANESE.md#web-canvas-and-corrected-baseline-2026-09-07).

A trained CNN and paired evaluation have since been added; see
[`KANA-VISION.md`](KANA-VISION.md). The review below describes the original
state, not the corrected implementation.

The capture/evaluation infrastructure is substantial, but the current recognition
results contain a coordinate-system error. Correct that before collecting more
samples, changing thresholds, or choosing a different model. This review leaves
the in-progress implementation and the study database unchanged.

## Findings

### 1. Katakana templates are vertically reflected relative to canvas input

`tools/build-kana-templates.py` copies AnimCJK's JSON `medians` directly into
the embedded asset. Those coordinates use an upward Y axis; the canvas and
ctegaki hiragana use a downward Y axis. The conversion from this AnimCJK JSON
to its SVG coordinates is `[x, 900 - y]`.

For example, embedded `フ` begins at `[163, 594]` and ends at `[246, 53]`.
The same stroke in the [pinned upstream SVG](https://github.com/parsimonhi/animCJK/blob/ec5e17cca76c87587790bcbce5ea0b4d4fb753d6/svgsJaKana/12501.svg)
begins at `[163, 306]` and ends at `[246, 847]`. In the canvas, downward
movement increases Y (`kana_canvas::ink_point`); no later matcher transform
corrects the reflection.

Consequently, recognizing an embedded template against itself tests an upside-down
character. The synthetic generator starts from those same templates and cannot
detect this error either.

The same omission occurs in `tools/check-kana-cross-source.py` when it imports
AnimCJK hiragana. Its reported 5/46 nearest matches compare opposite orientations.
KanjiVG katakana is likewise compared against reflected embedded katakana.
The earlier conclusion in `JAPANESE.md` that these failures establish shape
diversity as the cause is therefore premature. KanjiVG hiragana and K49 do not
use this particular faulty conversion; this finding does not by itself
invalidate their results.

Fix both import paths, regenerate the asset through the generators, and add
independently specified, screen-oriented asymmetric kana fixtures. Re-run the
synthetic and cross-source measurements before drawing further conclusions.
Keep historical reports with their original fingerprints rather than silently
relabeling them as corrected evidence.

### 2. Canvas capture is not lossless for stationary samples

`CanvasApp::append` discards an event whenever its X/Y position equals the
previous point, without comparing pressure or time. A touch held at one position
while its force changes loses those observations; even a stationary release can
lose its final timestamp. This contradicts the raw-ink preservation claim and can
understate captured duration. The matcher already skips zero-length segments,
so recognition does not require this destructive filtering in capture.

Retain delivered samples and test a stationary pressure change and release.
Keep frame-clock timestamps explicitly documented; preserving events cannot
recover device timestamps that egui did not supply.

### 3. Replay provenance omits the variant acceptance thresholds

Saved records include `variant_maximum_distance` and `variant_minimum_margin`,
but neither the CLI's `SavedRecognizer` nor the canvas's
`InitialRecognizerMetadata` reads them. Both compare only the three ordinary
thresholds. Changing a stored variant threshold alone therefore cannot be
reported as a configuration change, despite the report's provenance promise.

Include both fields in replay comparisons and missing-metadata handling, with
backward compatibility for older records. Prefer one shared saved-sample and
recognizer-metadata representation over maintaining these contracts separately
in the app and CLI.

## Work that can proceed without user testing

1. Correct the orientation conversion and establish independent orientation
   regression fixtures. Recompute the baseline before tuning anything.
2. Fix capture preservation and metadata comparison, using injected input events
   and saved JSON fixtures. Exercise whole event sequences: touch ownership,
   emulated mouse events, cancellation, leaving/re-entering the square, and
   releasing outside it. Existing tests cover several helpers and save states,
   but do not comprehensively drive `capture_events`.
3. Run corrected cross-source comparisons and inspect representative failures
   through headless canvas captures. Measure point-density changes and densely
   sampled pen-down joins: the connector heuristic currently recognizes a long
   gap among short segments, which need not describe a continuously sampled
   movement between strokes.
4. Establish a reproducible native/Wasm build check and a bounded-input stress
   benchmark before browser integration. Matching and recursive polyline
   simplification currently run synchronously; long traces need measured limits
   or bounded derived representations while keeping original ink intact.
5. After the corrected baseline, decide whether additional modern handwriting
   prototypes or a raster baseline is the best experiment. Freeze evaluation
   splits by writer/run before selection or threshold tuning. The existing K49
   test split has already been inspected and should not become the next tuning
   source.

Real held-out modern handwriting remains necessary to establish user-facing
accuracy, but it does not block steps 1–4. Public canonical data and generated
input can validate plumbing and invariants, not substitute for that evidence.
Keep graded cards, scheduler changes, stroke-quality scoring and pack plugins
deferred. No immediate collection or manual testing is requested from the user.

## Validation

Ran `nix-shell --run './tools/run-all-tests.sh'`: all workspace tests and the
four Python tool self-tests passed. The hardware audio test remains opt-in.

Ran `nix-shell --run 'python3 tools/check-kana.py'`: all four synthetic seeds
passed, with 99.46–99.91% top-1, 2.17–3.99% valid rejection and 100% invalid
rejection. One seed accepted one wrong identity. The reports are under
`kana-artifacts/kana-diagnostics/kana-gate-v1/`.

The paired orientation probe uses all 46 embedded katakana both unchanged and
converted to screen coordinates; it isolates the coordinate bug, not cross-writer
accuracy. Each converted sample replaces every `[x, y]` with `[x, 900 - y]`,
keeping the same label and katakana scope.

| Input orientation | Nearest identity correct | Accepted correctly | Rejected |
| --- | ---: | ---: | ---: |
| Embedded, upward Y | 46/46 | 46/46 | 0/46 |
| Screen, downward Y | 12/46 | 1/46 | 45/46 |

Neither group had an accepted wrong identity. Upright `フ` ranked `ム` first
at distance 0.129664 and was rejected as a poor match. This demonstrates why
passing the current synthetic gate does not clear the orientation defect.

The 92 probe records are under `kana-artifacts/kana-review/orientation/`; the full
result is `kana-artifacts/kana-review/orientation-report.json`. Re-run with:

```sh
nix-shell --run 'cargo run --quiet -p idiosepius-core --bin kana-recognize -- kana-artifacts/kana-review/orientation > kana-artifacts/kana-review/orientation-report.json'
```

The orientation report contains the corpus fingerprint, embedded-template
fingerprint, algorithm and current ordinary thresholds. Diagnostic artifacts
are ignored build output; the findings and measured counts remain in this
tracked-review candidate. This review adds only this Markdown file to the
source tree. No implementation fixes, template regeneration, UI changes,
database operations, or new real-writer accuracy claims are included.
