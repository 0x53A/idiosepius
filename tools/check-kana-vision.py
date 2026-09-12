#!/usr/bin/env python3
"""Validate a completed training run against the embedded Rust model.

No retraining or checkpoint selection. ETL inputs stay under target; only
aggregate metrics and synthetic arithmetic fixtures are suitable for committing.
"""

import argparse
import hashlib
import json
import subprocess
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("run", type=Path)
    parser.add_argument("--browser-corpus", type=Path)
    parser.add_argument(
        "--candidate",
        action="store_true",
        help="Verify run/model.bin without changing the embedded model",
    )
    args = parser.parse_args()
    run = args.run
    report = json.loads((run / "report.json").read_text())
    split = json.loads((run / "split.json").read_text())
    golden = json.loads((run / "golden.json").read_text())
    assert (
        hashlib.sha256((run / "split.json").read_bytes()).hexdigest()
        == report["split_sha256"]
    )
    model_path = (
        run / "model.bin"
        if args.candidate
        else ROOT / "crates/core/assets/kana_vision.bin"
    )
    model_args = ["--model", str(model_path)] if args.candidate else []
    model_sha = report["binary_model_sha256"]
    if model_sha is None:
        export = json.loads((run / "portable-export.json").read_text())
        assert (
            export["report_sha256"]
            == hashlib.sha256((run / "report.json").read_bytes()).hexdigest()
        )
        assert export["model_sha256"] == report["model_sha256"]
        assert (
            export["checkpoint_sha256"]
            == hashlib.sha256((run / "best.pt").read_bytes()).hexdigest()
        )
        model_sha = export["binary_model_sha256"]
    assert hashlib.sha256(model_path.read_bytes()).hexdigest() == model_sha
    groups = {}
    sheets = {}
    counts = {}
    for row in split["records"]:
        assert groups.setdefault(row["group"], row["split"]) == row["split"], (
            "group leakage"
        )
        assert (
            sheets.setdefault((row["dataset"], row["sheet"]), row["split"])
            == row["split"]
        ), "sheet leakage"
        counts.setdefault((row["dataset"], row["split"]), set()).add(row["label"])
    assert all(len(labels) == 46 for labels in counts.values()), (
        "missing class in split"
    )
    actual = json.loads(
        subprocess.check_output(
            [str(ROOT / "target/release/kana-rasterize"), "--logits", *model_args],
            input=json.dumps(golden["input"]).encode(),
        )
    )
    delta = max(
        abs(a - b)
        for row, expected in zip(actual, golden["logits"], strict=True)
        for a, b in zip(row, expected, strict=True)
    )
    assert delta < 0.0002, delta
    canonical = json.loads(
        subprocess.check_output(
            [
                str(ROOT / "target/release/kana-vision"),
                "--canonical",
                "--rasters",
                *model_args,
            ]
        )
    )
    output = {
        "model_sha256": model_sha,
        "split_sha256": report["split_sha256"],
        "pytorch_rust_max_logit_delta": delta,
        "canonical": {},
    }
    for script in ["hiragana", "katakana"]:
        rows = [r for r in canonical if r["script"] == script]
        output["canonical"][script] = {
            "samples": len(rows),
            "top1": sum(
                r["vision"]["candidates"][0]["character"] == r["expected"] for r in rows
            ),
            "top5": sum(
                any(c["character"] == r["expected"] for c in r["vision"]["candidates"])
                for r in rows
            ),
            "mean_ms": sum(r["vision_ms"] for r in rows) / len(rows),
            "failures": [
                {"expected": r["expected"], "vision": r["vision"]["candidates"]}
                for r in rows
                if r["vision"]["candidates"][0]["character"] != r["expected"]
            ],
        }
    if args.browser_corpus:
        samples = [
            json.loads(p.read_text())
            for p in sorted(args.browser_corpus.rglob("*.json"))
        ]
        assert samples, "empty browser corpus"
        current = json.loads(
            subprocess.check_output(
                [str(ROOT / "target/release/kana-vision"), *model_args],
                input=json.dumps(samples).encode(),
            )
        )
        maximum = 0.0
        for stored, replay in zip(samples, current, strict=True):
            before, after = stored["vision"], replay["vision"]
            assert before["model_fingerprint"] == after["model_fingerprint"]
            assert before["error"] == after["error"]
            assert [c["character"] for c in before["candidates"]] == [
                c["character"] for c in after["candidates"]
            ]
            maximum = max(
                maximum,
                max(
                    (
                        abs(a["score"] - b["score"])
                        for a, b in zip(
                            before["candidates"], after["candidates"], strict=True
                        )
                    ),
                    default=0.0,
                ),
            )
        assert maximum < 0.00002, maximum
        output["browser_native_parity"] = {
            "samples": len(samples),
            "maximum_score_delta": maximum,
        }
    (run / "canonical.json").write_text(
        json.dumps(canonical, ensure_ascii=False, indent=2) + "\n"
    )
    (run / "verification.json").write_text(
        json.dumps(output, ensure_ascii=False, indent=2) + "\n"
    )
    print(json.dumps(output, ensure_ascii=False, indent=2))


if __name__ == "__main__":
    main()
