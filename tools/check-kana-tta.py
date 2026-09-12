#!/usr/bin/env python3
"""Evaluate fixed translation-averaging policies on validation and canonical ink.

This does not modify weights or install a policy. All shifts zero-pad rather
than wrapping image edges. Test records are never evaluated.
"""

import argparse
import importlib.util
import json
import subprocess
from pathlib import Path

import numpy as np
import torch

spec = importlib.util.spec_from_file_location(
    "trainer", Path(__file__).with_name("train-kana-vision.py")
)
trainer = importlib.util.module_from_spec(spec)
spec.loader.exec_module(trainer)
POLICIES = {
    "single": [(0, 0)],
    "cross1": [(0, 0), (-1, 0), (1, 0), (0, -1), (0, 1)],
    "cross2": [(0, 0), (-2, 0), (2, 0), (0, -2), (0, 2)],
}


def shifted(x, dx, dy):
    out = torch.zeros_like(x)
    h, w = x.shape[-2:]
    out[..., max(0, dy) : min(h, h + dy), max(0, dx) : min(w, w + dx)] = x[
        ..., max(0, -dy) : min(h, h - dy), max(0, -dx) : min(w, w - dx)
    ]
    return out


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("model", type=Path)
    parser.add_argument("cohorts", type=Path, nargs="+")
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    torch.set_num_threads(3)
    config = json.loads((args.model / "config.json").read_text())
    side = int(config["rasterizer"].split("-")[2])
    net = trainer.Net(side, config["depth"], config.get("width", 1))
    net.load_state_dict(torch.load(args.model / "best.pt", weights_only=True))
    net.eval()
    result = {
        "scope": "validation only; fixed policies, mean logits, known-script reporting",
        "model": str(args.model),
        "policies": POLICIES,
        "cohorts": {},
        "canonical": {},
    }

    def evaluate(x):
        outputs = {p: [] for p in POLICIES}
        with torch.no_grad():
            for batch in x.split(128):
                logits = {
                    shift: net(shifted(batch, *shift))
                    for shift in {s for shifts in POLICIES.values() for s in shifts}
                }
                for p, shifts in POLICIES.items():
                    outputs[p].append(sum(logits[s] for s in shifts) / len(shifts))
        return {p: torch.cat(batches) for p, batches in outputs.items()}

    for cohort in args.cohorts:
        split = json.loads((cohort / "split.json").read_text())
        ids = [i for i, r in enumerate(split["records"]) if r["split"] == "validation"]
        records = [split["records"][i] for i in ids]
        with np.load(cohort / "prepared.npz") as data:
            x = torch.from_numpy(data["proxy_images"][ids])
            y = torch.from_numpy(data["labels"][ids])
        results = {}
        for policy, logits in evaluate(x).items():
            results[policy] = trainer.metrics(logits, y, records)
            for metrics in results[policy].values():
                del metrics["confusion"], metrics["labels"]
        result["cohorts"][str(cohort)] = results
        print("Evaluated " + str(cohort), flush=True)
    canonical = json.loads(
        subprocess.check_output(
            [
                "target/release/kana-vision",
                "--model",
                str(args.model / "model.bin"),
                "--canonical",
                "--rasters",
            ]
        )
    )
    x = torch.tensor([r["raster"] for r in canonical]).reshape(-1, 1, side, side)
    y = torch.tensor([trainer.LABELS.index(r["expected"]) for r in canonical])
    for policy, z in evaluate(x).items():
        scores = {
            "both_top1": int((z.argmax(1) == y).sum()),
            "known_script_top1": 0,
            "samples": len(y),
        }
        for i, r in enumerate(canonical):
            offset = 46 if r["script"] == "katakana" else 0
            scores["known_script_top1"] += int(
                z[i, offset : offset + 46].argmax() + offset == y[i]
            )
        result["canonical"][policy] = scores
    args.output.write_text(json.dumps(result, indent=2) + "\n")
    print(json.dumps(result["canonical"], indent=2))


if __name__ == "__main__":
    main()
