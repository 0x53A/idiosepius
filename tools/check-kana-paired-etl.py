#!/usr/bin/env python3
"""Compare both classifiers on identical skeletonized ETL validation ink.

This is a scan-to-trajectory proxy, not captured online handwriting. CNN scores
on these rasters must not be conflated with its direct-grayscale test results.
Restricted derived traces remain under the training run in ignored target/.
"""

import argparse
import hashlib
import importlib.util
import json
import subprocess
from pathlib import Path

import kana_etl7

ROOT = Path(__file__).resolve().parent.parent
spec = importlib.util.spec_from_file_location("etl", ROOT / "tools/check-kana-etl.py")
etl = importlib.util.module_from_spec(spec)
spec.loader.exec_module(etl)


def counts_for(rows):
    counts = {
        "samples": len(rows),
        "geometry_top1": 0,
        "vision_top1": 0,
        "both_correct": 0,
        "vision_only_correct": 0,
        "geometry_only_correct": 0,
        "geometry_accepted": 0,
        "geometry_accepted_wrong": 0,
    }
    for row in rows:
        g = row["geometry"]["candidates"]
        v = row["vision"]["candidates"]
        gc = bool(g and g[0]["character"] == row["expected"])
        vc = bool(v and v[0]["character"] == row["expected"])
        counts["geometry_top1"] += gc
        counts["vision_top1"] += vc
        counts["both_correct"] += gc and vc
        counts["vision_only_correct"] += vc and not gc
        counts["geometry_only_correct"] += gc and not vc
        accepted = row["geometry"]["state"] == "recognized"
        counts["geometry_accepted"] += accepted
        counts["geometry_accepted_wrong"] += accepted and not gc
    return counts


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("run", type=Path)
    parser.add_argument("--data", type=Path, default=ROOT / "target/kana-training/data")
    parser.add_argument(
        "--model", type=Path, help="Replay an unpromoted candidate binary"
    )
    args = parser.parse_args()
    if not any(
        args.run.resolve().is_relative_to((ROOT / area).resolve())
        for area in ("target", "kana-artifacts")
    ):
        raise SystemExit("Keep ETL-derived traces below target/ or kana-artifacts/")
    config = json.loads((args.run / "config.json").read_text())
    clean = None
    if config.get("clean_scans"):
        path = ROOT / "tools/check-kana-preprocessing.py"
        if (
            hashlib.sha256(path.read_bytes()).hexdigest()
            != config["scan_preprocessor_sha256"]
        ):
            raise ValueError("Scan preprocessor differs from training run")
        clean_spec = importlib.util.spec_from_file_location("clean", path)
        clean = importlib.util.module_from_spec(clean_spec)
        clean_spec.loader.exec_module(clean)

    def vectors_for(pixels):
        return (
            etl.trace_skeleton(etl.thin(clean.clean_image(pixels)[1]))
            if clean is not None
            else etl.vectorize_image(pixels)
        )

    split = json.loads((args.run / "split.json").read_text())
    data = {}
    for dataset in ["etl4", "etl5"]:
        data[dataset] = (
            args.data / dataset.upper() / (dataset.upper() + "C")
        ).read_bytes()
        assert (
            hashlib.sha256(data[dataset]).hexdigest()
            == split["sources"][dataset]["sha256"]
        )
    if "etl7" in split["sources"]:
        for filename, source in split["sources"]["etl7"]["files"].items():
            raw = (args.data / "ETL7" / filename).read_bytes()
            assert hashlib.sha256(raw).hexdigest() == source["sha256"]
            data[filename] = raw
    samples = []
    for row in split["records"]:
        if row["split"] != "validation":
            continue
        if row["dataset"] == "etl7":
            offset = row["record"] * kana_etl7.RECORD_BYTES
            decoded = kana_etl7.decode(
                data[row["file"]][offset : offset + kana_etl7.RECORD_BYTES]
            )
            assert decoded["label"] == row["label"] and decoded["group"] == row["group"]
            vectors = vectors_for(decoded["image"])
        else:
            offset = row["record"] * etl.RECORD_BYTES
            raw = data[row["dataset"]][offset : offset + etl.RECORD_BYTES]
            assert etl.decode_label(raw, row["dataset"]) == row["label"]
            vectors = vectors_for(etl.grayscale(raw))
        strokes = [[{"x": x, "y": y} for x, y in stroke] for stroke in vectors]
        samples.append(
            {
                "expected": row["label"],
                "script": "katakana" if row["dataset"] == "etl5" else "hiragana",
                "strokes": strokes,
                "external_source": row,
            }
        )
    print(f"Vectorized {len(samples)} validation records", flush=True)
    results = json.loads(
        subprocess.check_output(
            [
                str(ROOT / "target/release/kana-vision"),
                *(["--model", str(args.model)] if args.model else []),
            ],
            input=json.dumps(samples).encode(),
        )
    )
    fingerprint = results[0]["vision"]["model_fingerprint"]
    summary = {
        "model_fingerprint": fingerprint,
        "geometry_source_sha256": hashlib.sha256(
            (ROOT / "crates/core/src/kana.rs").read_bytes()
        ).hexdigest(),
        "template_sha256": hashlib.sha256(
            (ROOT / "crates/core/assets/kana_templates.json").read_bytes()
        ).hexdigest(),
        "vectorizer_sha256": hashlib.sha256(
            (ROOT / "tools/check-kana-etl.py").read_bytes()
        ).hexdigest(),
        "split_sha256": hashlib.sha256(
            (args.run / "split.json").read_bytes()
        ).hexdigest(),
        "scope": "validation skeleton proxy; not online handwriting or direct grayscale evaluation",
        "clean_scans": config.get("clean_scans", False),
        "scan_preprocessor_sha256": config.get("scan_preprocessor_sha256"),
        "scripts": {},
        "datasets": {},
        "etl7_reader_sha256": hashlib.sha256(
            Path(kana_etl7.__file__).read_bytes()
        ).hexdigest(),
    }
    for script in ["hiragana", "katakana"]:
        summary["scripts"][script] = counts_for(
            [r for r in results if r["script"] == script]
        )
    for dataset in sorted(split["sources"]):
        summary["datasets"][dataset] = counts_for(
            [
                r
                for sample, r in zip(samples, results, strict=True)
                if sample["external_source"]["dataset"] == dataset
            ]
        )
    output = args.run / f"paired-validation-{fingerprint}"
    output.mkdir(exist_ok=True)
    etl.atomic_json(output / "samples.json", samples)
    etl.atomic_json(output / "results.json", results)
    etl.atomic_json(output / "summary.json", summary)
    print(json.dumps(summary, indent=2))


if __name__ == "__main__":
    main()
