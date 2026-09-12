#!/usr/bin/env python3
"""Compare a border-aware scan threshold on validation only, preserving old inputs.

Restricted source images and derived arrays stay under target/. This is an
input-processing experiment, not an improvement to browser ink recognition.
"""

import argparse
import hashlib
import importlib.util
import json
import struct
import subprocess
from pathlib import Path

import kana_etl7

ROOT = Path(__file__).resolve().parent.parent
spec = importlib.util.spec_from_file_location("etl", ROOT / "tools/check-kana-etl.py")
etl = importlib.util.module_from_spec(spec)
spec.loader.exec_module(etl)


def clean_image(image):
    """Orient contrast, then keep Otsu ink above the border's 75th percentile."""
    if not image or not image[0] or any(len(r) != len(image[0]) for r in image):
        raise ValueError("Expected rectangular nonempty image")
    border = (
        image[0]
        + image[-1]
        + [r[0] for r in image[1:-1]]
        + [r[-1] for r in image[1:-1]]
    )
    values = [v for r in image for v in r]
    low, high = min(values), max(values)
    background = sorted(border)[len(border) // 2]
    invert = background - low > high - background
    oriented = [[15 - v if invert else v for v in row] for row in image]
    border = [15 - v if invert else v for v in border]
    threshold = max(etl.otsu_threshold(oriented), sorted(border)[3 * len(border) // 4])
    ink = {
        (x, y)
        for y, row in enumerate(oriented)
        for x, v in enumerate(row)
        if v > threshold
    }
    ink = etl.keep_components(ink, minimum=3)
    gray = [
        [v * 17 if (x, y) in ink else 0 for x, v in enumerate(row)]
        for y, row in enumerate(oriented)
    ]
    return gray, ink


def self_test():
    image = [[4 + int((x + y) % 3 == 0) for x in range(20)] for y in range(20)]
    for y in range(4, 16):
        for x in range(8, 11):
            image[y][x] = 8
    gray, ink = clean_image(image)
    assert ink == {(x, y) for x in range(8, 11) for y in range(4, 16)}
    assert clean_image([[15 - v for v in r] for r in image])[1] == ink
    assert not clean_image([[4] * 8 for _ in range(8)])[1]
    binary = [[int(2 <= x <= 4 and 2 <= y <= 4) for x in range(8)] for y in range(8)]
    assert len(clean_image(binary)[1]) == 9
    assert max(v for r in gray for v in r) == 8 * 17
    print("Scan preprocessing self-tests passed")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("run", type=Path, nargs="?")
    parser.add_argument("--output", type=Path)
    parser.add_argument("--self-test", action="store_true")
    args = parser.parse_args()
    if args.self_test:
        self_test()
        return
    if args.run is None or args.output is None:
        parser.error("run and --output are required")
    if not args.output.resolve().is_relative_to((ROOT / "target").resolve()):
        raise ValueError("Keep ETL derivatives under target/")
    import numpy as np
    import torch

    trainer_spec = importlib.util.spec_from_file_location(
        "trainer", ROOT / "tools/train-kana-vision.py"
    )
    trainer = importlib.util.module_from_spec(trainer_spec)
    trainer_spec.loader.exec_module(trainer)
    args.output.mkdir(parents=True, exist_ok=True)
    split = json.loads((args.run / "split.json").read_text())
    records = [r for r in split["records"] if r["split"] == "validation"]
    data = {}
    for ds in ["etl4", "etl5"]:
        data[ds] = (
            ROOT / "target/kana-training/data" / ds.upper() / (ds.upper() + "C")
        ).read_bytes()
        assert hashlib.sha256(data[ds]).hexdigest() == split["sources"][ds]["sha256"]
    for filename, source in split["sources"].get("etl7", {}).get("files", {}).items():
        data[filename] = (
            ROOT / "target/kana-training/data/ETL7" / filename
        ).read_bytes()
        assert hashlib.sha256(data[filename]).hexdigest() == source["sha256"]
    payload, samples = bytearray(), []
    for row in records:
        if row["dataset"] == "etl7":
            start = row["record"] * kana_etl7.RECORD_BYTES
            image = kana_etl7.decode(
                data[row["file"]][start : start + kana_etl7.RECORD_BYTES]
            )["image"]
        else:
            start = row["record"] * etl.RECORD_BYTES
            image = etl.grayscale(
                data[row["dataset"]][start : start + etl.RECORD_BYTES]
            )
        gray, ink = clean_image(image)
        payload.extend(struct.pack("<HH", len(gray[0]), len(gray)))
        payload.extend(bytes(v for line in gray for v in line))
        samples.append(
            {
                "strokes": [
                    [{"x": x, "y": y} for x, y in path]
                    for path in etl.trace_skeleton(etl.thin(ink))
                ]
            }
        )
    exe = str(ROOT / "target/release/kana-rasterize")
    arrays = {}
    for name, command, raw in [
        ("images", [exe, "--side", "64"], payload),
        (
            "proxy_images",
            [exe, "--side", "64", "--ink-batch"],
            json.dumps([s["strokes"] for s in samples]).encode(),
        ),
    ]:
        output = subprocess.check_output(command, input=raw)
        arrays[name] = np.frombuffer(output, dtype="<f4").copy().reshape(-1, 1, 64, 64)
    np.savez_compressed(args.output / "validation.npz", **arrays)
    torch.set_num_threads(3)
    config = json.loads((args.run / "config.json").read_text())
    net = trainer.Net(64, config["depth"], config.get("width", 1))
    net.load_state_dict(torch.load(args.run / "best.pt", weights_only=True))
    net.eval()
    results = {}
    with torch.no_grad():
        for name, array in arrays.items():
            logits = torch.cat([net(b) for b in torch.from_numpy(array).split(128)])
            scores = trainer.metrics(
                logits,
                torch.tensor([trainer.LABELS.index(r["label"]) for r in records]),
                records,
            )
            for metrics in scores.values():
                del metrics["confusion"], metrics["labels"]
            results[name] = scores
    summary = {
        "split": "validation",
        "model_sha256": hashlib.sha256(
            (args.run / "model.bin").read_bytes()
        ).hexdigest(),
        "preprocessing_sha256": hashlib.sha256(Path(__file__).read_bytes()).hexdigest(),
        "note": "Same checkpoint and records, changed scan preprocessing. Not canvas accuracy.",
        "results": results,
    }
    (args.output / "summary.json").write_text(json.dumps(summary, indent=2) + "\n")
    print(json.dumps(summary, indent=2))


if __name__ == "__main__":
    main()
