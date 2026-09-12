#!/usr/bin/env python3
"""Export an archived residual research checkpoint for candidate Rust/Wasm checks.

The original training report stays immutable. A separate manifest binds this
portable export to the completed report, JSON weights, and checkpoint.
"""

import argparse
import importlib.util
import json
import struct
from pathlib import Path

import numpy as np
import torch

spec = importlib.util.spec_from_file_location(
    "trainer", Path(__file__).with_name("train-kana-vision.py")
)
trainer = importlib.util.module_from_spec(spec)
spec.loader.exec_module(trainer)


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument("run", type=Path)
    a = p.parse_args()
    if not any(
        a.run.resolve().is_relative_to((trainer.ROOT / area).resolve())
        for area in ("target", "kana-artifacts")
    ):
        raise ValueError("Keep research artifacts under target/ or kana-artifacts/")
    config = json.loads((a.run / "config.json").read_text())
    report = json.loads((a.run / "report.json").read_text())
    model = json.loads((a.run / "model.json").read_text())
    if config != report["config"] or config.get("family") != "residual":
        raise ValueError("Expected a completed residual run")
    if trainer.digest((a.run / "model.json").read_bytes()) != report["model_sha256"]:
        raise ValueError("Archived JSON weights changed")
    side = int(config["rasterizer"].split("-")[2])
    depth, width = config["depth"], config["width"]
    if side not in [32, 64] or depth not in [2, 3] or width not in [1, 2, 4]:
        raise ValueError("Unsupported descriptor")
    net = trainer.make_net(config)
    net.load_state_dict(torch.load(a.run / "best.pt", weights_only=True))
    weights = np.concatenate(
        [v.detach().numpy().flatten() for v in net.parameters()]
    ).astype("<f4")
    if not np.array_equal(weights, np.array(model["weights"], dtype="<f4")):
        raise ValueError("Checkpoint differs from archived JSON weights")
    if (
        model["labels"] != trainer.LABELS
        or not np.isfinite(weights).all()
        or (abs(weights) > 100).any()
    ):
        raise ValueError("Invalid labels or weights")
    binary = (
        b"IDKANA04"
        + struct.pack("<III", side, depth, width)
        + struct.pack("<92I", *map(ord, trainer.LABELS))
        + weights.tobytes()
    )
    if len(binary) > 10_000_000:
        raise ValueError("Model exceeds the quality-first byte budget")
    path = a.run / "model.bin"
    if path.exists():
        raise ValueError("Never overwrite an export")
    path.write_bytes(binary)
    trainer.write(
        a.run / "portable-export.json",
        {
            "format": "IDKANA04",
            "binary_model_sha256": trainer.digest(binary),
            "model_sha256": report["model_sha256"],
            "report_sha256": trainer.digest((a.run / "report.json").read_bytes()),
            "checkpoint_sha256": trainer.digest((a.run / "best.pt").read_bytes()),
            "bytes": len(binary),
            "note": "Candidate export only; runtime parity is verified separately. No app weights installed.",
        },
    )
    print(f"Exported {len(binary)} bytes to {path}")


if __name__ == "__main__":
    main()
