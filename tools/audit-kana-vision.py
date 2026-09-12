#!/usr/bin/env python3
"""Audit scan/proxy disagreements on frozen validation data; keep images local."""

import argparse
import hashlib
import importlib.util
import json
import math
from collections import Counter, defaultdict
from pathlib import Path

import numpy as np
import torch
from PIL import Image, ImageDraw

spec = importlib.util.spec_from_file_location(
    "trainer", Path(__file__).with_name("train-kana-vision.py")
)
trainer = importlib.util.module_from_spec(spec)
spec.loader.exec_module(trainer)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("run", type=Path)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--threads", type=int, default=3)
    args = parser.parse_args()
    root = Path(__file__).resolve().parent.parent
    if not any(
        args.output.resolve().is_relative_to((root / area).resolve())
        for area in ("target", "kana-artifacts")
    ):
        raise ValueError(
            "Derived ETL images must remain under target/ or kana-artifacts/"
        )
    args.output.mkdir(parents=True, exist_ok=True)
    config = json.loads((args.run / "config.json").read_text())
    split = json.loads((args.run / "split.json").read_text())
    ids = [i for i, r in enumerate(split["records"]) if r["split"] == "validation"]
    records = [split["records"][i] for i in ids]
    with np.load(args.run / "prepared.npz") as data:
        inputs = {k: torch.from_numpy(data[k][ids]) for k in ["images", "proxy_images"]}
    if args.threads < 1:
        raise ValueError("threads must be positive")
    torch.set_num_threads(args.threads)
    net = trainer.make_net(config)
    net.load_state_dict(torch.load(args.run / "best.pt", weights_only=True))
    net.eval()
    predictions = {}
    confidence = {}
    with torch.no_grad():
        for name, x in inputs.items():
            logits = torch.cat([net(b) for b in x.split(128)])
            predictions[name] = []
            confidence[name] = []
            for i, r in enumerate(records):
                offset = 46 if r["dataset"] == "etl5" else 0
                confidence[name].append(
                    float(logits[i, offset : offset + 46].softmax(0).max())
                )
                predictions[name].append(
                    trainer.LABELS[
                        offset + int(logits[i, offset : offset + 46].argmax())
                    ]
                )

    result = {
        "split": "validation",
        "model_sha256": hashlib.sha256(
            (args.run / "model.bin").read_bytes()
        ).hexdigest(),
        "datasets": {},
    }
    for dataset in sorted({r["dataset"] for r in records}):
        counts, pairs = Counter(), Counter()
        groups = defaultdict(Counter)
        failures = []
        for i, r in enumerate(records):
            if r["dataset"] != dataset:
                continue
            scan = predictions["images"][i]
            proxy = predictions["proxy_images"][i]
            sc, pc = scan == r["label"], proxy == r["label"]
            counts["samples"] += 1
            counts[
                "both_correct"
                if sc and pc
                else "scan_only_correct"
                if sc
                else "proxy_only_correct"
                if pc
                else "both_wrong"
            ] += 1
            groups[r["group"]]["samples"] += 1
            groups[r["group"]]["proxy_errors"] += not pc
            if not pc:
                pairs[r["label"] + "→" + proxy] += 1
                failures.append(
                    {
                        "index": i,
                        "expected": r["label"],
                        "scan": scan,
                        "proxy": proxy,
                        "scan_correct": sc,
                        "source": r,
                    }
                )
        result["datasets"][dataset] = {
            "counts": dict(counts),
            "proxy_confusions": pairs.most_common(20),
            "groups": dict(groups),
        }
        risk = []
        for threshold in [0.0, 0.5, 0.8, 0.9, 0.95, 0.99, 0.999]:
            accepted = [
                i
                for i, r in enumerate(records)
                if r["dataset"] == dataset
                and confidence["proxy_images"][i] >= threshold
            ]
            n = len(accepted)
            errors = sum(
                predictions["proxy_images"][i] != records[i]["label"] for i in accepted
            )
            rate = errors / n if n else 0
            z = 1.959963984540054
            upper = (
                (
                    rate
                    + z * z / (2 * n)
                    + z * math.sqrt(rate * (1 - rate) / n + z * z / (4 * n * n))
                )
                / (1 + z * z / n)
                if n
                else None
            )
            risk.append(
                {
                    "threshold": threshold,
                    "accepted": n,
                    "errors": errors,
                    "coverage": n / counts["samples"],
                    "accuracy": 1 - rate if n else None,
                    "wilson_95_error_upper": upper,
                }
            )
        result["datasets"][dataset]["proxy_confidence_diagnostic"] = risk
        result["datasets"][dataset]["confidence_note"] = (
            "Validation valid-ink only; descriptive, not a calibrated acceptance policy. Writer clustering is not represented by the binomial Wilson interval."
        )
        # A deterministic first-page sample, rather than cherry-picking illustrations.
        selected = sorted(failures, key=lambda r: (not r["scan_correct"], r["index"]))[
            :48
        ]
        sheet = Image.new("RGB", (8 * 136, 6 * 100), "white")
        draw = ImageDraw.Draw(sheet)
        for j, row in enumerate(selected):
            x0, y0 = j % 8 * 136, j // 8 * 100
            for col, name in enumerate(["images", "proxy_images"]):
                pixels = (255 * (1 - inputs[name][row["index"], 0].numpy())).astype(
                    np.uint8
                )
                sheet.paste(Image.fromarray(pixels).convert("RGB"), (x0 + col * 66, y0))
            draw.text(
                (x0, y0 + 66),
                f"#{row['index']} scan={int(row['scan_correct'])}",
                fill="black",
            )
            draw.text(
                (x0, y0 + 79),
                f"U+{ord(row['expected']):04X}->{ord(row['proxy']):04X}",
                fill="black",
            )
        sheet.save(args.output / f"{dataset}-scan-proxy.png")
        (args.output / f"{dataset}-failures.json").write_text(
            json.dumps(failures, ensure_ascii=False, indent=2) + "\n"
        )
    (args.output / "summary.json").write_text(
        json.dumps(result, ensure_ascii=False, indent=2) + "\n"
    )
    print(json.dumps(result, ensure_ascii=False, indent=2))


if __name__ == "__main__":
    main()
