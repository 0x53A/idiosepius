#!/usr/bin/env python3
"""Evaluate stricter scan-background thresholds without changing frozen training.

Reuses the validation-only loader/rasterizer/evaluator from the original cleaning
experiment. Each variant records both source hashes; restricted derivatives stay
under target. No checkpoint is trained or installed by this diagnostic.
"""

import argparse
import importlib.util
import json
import sys
from pathlib import Path

spec = importlib.util.spec_from_file_location(
    "cleaning", Path(__file__).with_name("check-kana-preprocessing.py")
)
cleaning = importlib.util.module_from_spec(spec)
spec.loader.exec_module(cleaning)


def threshold_image(image, percentile):
    border = (
        image[0]
        + image[-1]
        + [r[0] for r in image[1:-1]]
        + [r[-1] for r in image[1:-1]]
    )
    values = [v for row in image for v in row]
    low, high = min(values), max(values)
    background = sorted(border)[len(border) // 2]
    invert = background - low > high - background
    oriented = [[15 - v if invert else v for v in row] for row in image]
    border = [15 - v if invert else v for v in border]
    threshold = max(
        cleaning.etl.otsu_threshold(oriented),
        sorted(border)[percentile * len(border) // 100],
    )
    ink = {
        (x, y)
        for y, row in enumerate(oriented)
        for x, v in enumerate(row)
        if v > threshold
    }
    ink = cleaning.etl.keep_components(ink, minimum=3)
    return [
        [v * 17 if (x, y) in ink else 0 for x, v in enumerate(row)]
        for y, row in enumerate(oriented)
    ], ink


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("run", type=Path)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    if not args.output.resolve().is_relative_to((cleaning.ROOT / "target").resolve()):
        raise ValueError("Keep derivatives under target/")
    args.output.mkdir(parents=True, exist_ok=False)
    original_source = Path(cleaning.__file__).read_bytes()
    source = Path(__file__).read_bytes()
    (args.output / "background-quantiles.py").write_bytes(source)
    (args.output / "original-preprocessing.py").write_bytes(original_source)
    # Verify that the diagnostic generalization agrees with the frozen cleaner.
    for image in [
        [[4 + (x + y) % 3 for x in range(20)] for y in range(20)],
        [[15 - (4 + (x + y) % 3) for x in range(20)] for y in range(20)],
    ]:
        assert threshold_image(image, 75) == cleaning.clean_image(image)
    manifest = {
        "quantiles": [95, 99],
        "model": str(args.run),
        "preprocessing_sha256": cleaning.hashlib.sha256(source).hexdigest(),
        "loader_source_sha256": cleaning.hashlib.sha256(original_source).hexdigest(),
        "note": "Validation-only input diagnostics. Frozen training arrays and the app rasterizer are unchanged. Thresholds chosen after visually auditing residual noise, before these diagnostic scores were observed.",
        "results": {},
    }
    path = args.output / "summary.json"
    path.write_text(json.dumps(manifest, indent=2) + "\n")
    for percentile in manifest["quantiles"]:
        output = args.output / str(percentile)
        cleaning.clean_image = lambda image, q=percentile: threshold_image(image, q)
        sys.argv = [
            str(Path(cleaning.__file__)),
            str(args.run),
            "--output",
            str(output),
        ]
        cleaning.main()
        result = json.loads((output / "summary.json").read_text())
        result.update(
            {
                "border_percentile": percentile,
                "preprocessing_sha256": manifest["preprocessing_sha256"],
                "loader_source_sha256": manifest["loader_source_sha256"],
            }
        )
        (output / "summary.json").write_text(json.dumps(result, indent=2) + "\n")
        manifest["results"][str(percentile)] = result
        path.write_text(json.dumps(manifest, indent=2) + "\n")


if __name__ == "__main__":
    main()
