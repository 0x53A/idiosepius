#!/usr/bin/env python3
"""Exercise wide training, archived-source reproduction, and Rust parity without ETL data.

Run from the repository root in the trainer's pinned environment, after building
kana-rasterize and kana-vision. The output directory must be new and under target/.
"""

import argparse
import importlib.util
import json
import subprocess
import sys
from pathlib import Path

import numpy as np
import torch

sys.path.insert(0, str(Path("tools").resolve()))
spec = importlib.util.spec_from_file_location("trainer", "tools/train-kana-vision.py")
m = importlib.util.module_from_spec(spec)
spec.loader.exec_module(m)
parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument("--output", type=Path, required=True)
parser.add_argument("--checkpoint-selection", choices=["loss", "source-top1"], default="source-top1")
args = parser.parse_args()
root = args.output
if not root.resolve().is_relative_to(
    (Path(__file__).resolve().parent.parent / "target").resolve()
):
    raise ValueError("Keep smoke artifacts under target/")
root.mkdir()
source = root / "prepared"
source.mkdir()
config = {
    "seed": 20260908,
    "ink_proxy": True,
    "etl7": False,
    "rasterizer": m.RASTER,
    "etl7_reader_sha256": m.digest(Path("tools/kana_etl7.py").read_bytes()),
    "depth": 3,
    "width": 1,
    "clean_scans": False,
    "scan_preprocessor_sha256": None,
}
(source / "config.json").write_text(json.dumps(config))
records = [
    {
        "dataset": "etl4" if i < 46 else "etl5",
        "label": c,
        "group": split,
        "sheet": j,
        "split": split,
    }
    for j, split in enumerate(["train", "validation", "test"])
    for i, c in enumerate(m.LABELS)
]
(source / "split.json").write_text(
    json.dumps({"records": records, "sources": {"synthetic_fixture": True}})
)
rng = np.random.default_rng(42)
x = rng.random((len(records), 1, 32, 32), dtype=np.float32)
np.savez_compressed(
    source / "prepared.npz",
    images=x,
    proxy_images=x,
    labels=np.array([m.LABELS.index(r["label"]) for r in records]),
)
(source / "proxy-preparation.json").write_text(json.dumps({"synthetic_fixture": True}))
torch.save(m.Net(32, 3).state_dict(), source / "best.pt")
first = root / "first"
second = root / "second"
subprocess.run(
    [
        sys.executable,
        "tools/train-kana-vision.py",
        "--keep-checkpoints",
        "--checkpoint-selection",
        args.checkpoint_selection,
        "--validation-only",
        "--channels-last",
        "--run",
        str(first),
        "--prepared-from",
        str(source),
        "--initialize-from",
        str(source),
        "--epochs",
        "3",
        "--threads",
        "1",
        "--side",
        "32",
        "--depth",
        "3",
        "--width",
        "2",
        "--source-balanced",
        "--ink-proxy",
    ],
    check=True,
    stdout=subprocess.DEVNULL,
)
history = json.loads((first / "history.json").read_text())
selected = min(history, key=lambda row: m.checkpoint_key(
    args.checkpoint_selection, row["validation_loss"], row["validation_metrics"]
))["epoch"]
report = json.loads((first / "report.json").read_text())
assert report["best_epoch"] == selected
assert set(report["results"]) == {"validation"}
assert set(report["proxy_results"]) == {"validation"}
best = torch.load(first / "best.pt", weights_only=True)
retained = torch.load(first / f"epoch-{selected:03}.pt", weights_only=True)
assert best.keys() == retained.keys()
assert all(torch.equal(best[key], retained[key]) for key in best)
subprocess.run(
    [sys.executable, "tools/reproduce-kana-training.py", str(first), str(second)],
    check=True,
    stdout=subprocess.DEVNULL,
)
subprocess.run(
    [sys.executable, "tools/check-kana-reproduction.py", str(first), str(second)],
    check=True,
)
subprocess.run(
    [sys.executable, "tools/check-kana-vision.py", str(first), "--candidate"],
    check=True,
    stdout=subprocess.DEVNULL,
)
print(
    "Synthetic end-to-end wide training, snapshot reproduction, and Rust parity passed"
)
