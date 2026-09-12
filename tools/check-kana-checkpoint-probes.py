#!/usr/bin/env python3
"""Inspect confidence on fixed non-kana probes; never infer an acceptance policy.

Research checkpoints are evaluated directly in PyTorch, including residual models
without a portable runtime. These synthetic probes are not a human ink benchmark.
"""

import argparse
import importlib.util
import json
import math
import subprocess
from pathlib import Path

import numpy as np
import torch

spec = importlib.util.spec_from_file_location(
    "trainer", Path(__file__).with_name("train-kana-vision.py")
)
trainer = importlib.util.module_from_spec(spec)
spec.loader.exec_module(trainer)


def probes():
    # Explicitly non-character compositions. Retain raw strokes for reproducibility.
    rng = np.random.default_rng(9173)
    return {
        "empty": [],
        "dense_grid": [
            [{"x": t, "y": 0}, {"x": t, "y": 100}] for t in range(0, 101, 10)
        ]
        + [[{"x": 0, "y": t}, {"x": 100, "y": t}] for t in range(0, 101, 10)],
        "concentric_circles": [
            [
                {"x": 50 + r * math.cos(t), "y": 50 + r * math.sin(t)}
                for t in np.linspace(0, 2 * math.pi, 101)
            ]
            for r in [10, 20, 30, 40, 50]
        ],
        "dense_scribble": [
            [{"x": float(x), "y": float(y)} for x, y in rng.uniform(0, 100, (150, 2))]
        ],
    }


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument("models", type=Path, nargs="+")
    p.add_argument("--output", type=Path, required=True)
    a = p.parse_args()
    torch.set_num_threads(1)
    examples = probes()
    result = {
        "note": "Uncalibrated checkpoint softmax on synthetic non-kana compositions. Empty ink is rejected before CNN inference in the app. These scores define no acceptance threshold or invalid-ink accuracy claim.",
        "probes": examples,
        "models": {},
    }
    for run in a.models:
        config = json.loads((run / "config.json").read_text())
        side = int(config["rasterizer"].split("-")[2])
        net = trainer.make_net(config)
        net.load_state_dict(torch.load(run / "best.pt", weights_only=True))
        net.eval()
        raw = subprocess.check_output(
            [
                str(trainer.ROOT / "target/release/kana-rasterize"),
                "--ink-batch",
                "--side",
                str(side),
            ],
            input=json.dumps(list(examples.values())).encode(),
        )
        x = torch.from_numpy(np.frombuffer(raw, dtype="<f4").copy()).reshape(
            -1, 1, side, side
        )
        with torch.no_grad():
            logits = net(x)
        rows = {}
        for i, name in enumerate(examples):
            rows[name] = {}
            for scope, start, end in [
                ("both", 0, 92),
                ("hiragana", 0, 46),
                ("katakana", 46, 92),
            ]:
                scores = logits[i, start:end].softmax(0)
                score, pred = scores.max(0)
                rows[name][scope] = {
                    "candidate": trainer.LABELS[start + int(pred)],
                    "score": float(score),
                }
        result["models"][str(run)] = {
            "checkpoint_sha256": trainer.digest((run / "best.pt").read_bytes()),
            "results": rows,
        }
    a.output.write_text(json.dumps(result, ensure_ascii=False, indent=2) + "\n")
    print(f"Wrote {len(examples)} probes for {len(a.models)} checkpoints to {a.output}")


if __name__ == "__main__":
    main()
