#!/usr/bin/env python3
"""Compare model checkpoints on one frozen validation cohort, never its test set.

Use the trainer's pinned PyTorch/NumPy environment. Models must share the same
rasterizer; architectures may differ, data comes from the cohort's prepared.npz. Output
contains aggregates only. This command never installs a model in the app.
"""

import argparse
import hashlib
import importlib.util
import json
import subprocess
from pathlib import Path

import numpy as np
import torch
from torch.nn import functional as F

spec = importlib.util.spec_from_file_location(
    "trainer", Path(__file__).with_name("train-kana-vision.py")
)
trainer = importlib.util.module_from_spec(spec)
spec.loader.exec_module(trainer)


def compare(cohort, runs):
    split_bytes = (cohort / "split.json").read_bytes()
    split = json.loads(split_bytes)
    config = json.loads((cohort / "config.json").read_text())
    ids = [i for i, r in enumerate(split["records"]) if r["split"] == "validation"]
    if not ids:
        raise ValueError("empty validation cohort")
    records = [split["records"][i] for i in ids]
    groups = {}
    for row in split["records"]:
        if groups.setdefault(row["group"], row["split"]) != row["split"]:
            raise ValueError("group leakage")
    with np.load(cohort / "prepared.npz") as prepared:
        y = torch.from_numpy(prepared["labels"][ids])
        inputs = {
            name: torch.from_numpy(prepared[name][ids])
            for name in ("images", "proxy_images")
            if name in prepared
        }
    side = inputs["images"].shape[-1]
    result = {
        "cohort": str(cohort),
        "split_sha256": hashlib.sha256(split_bytes).hexdigest(),
        "split": "validation",
        "clean_scans": config.get("clean_scans", False),
        "scan_preprocessor_sha256": config.get("scan_preprocessor_sha256"),
        "samples": len(ids),
        "models": {},
    }
    for run in runs:
        if (run / "split.json").read_bytes() != split_bytes:
            raise ValueError(f"{run}: incompatible frozen split")
        model_config = json.loads((run / "config.json").read_text())
        for field in ("rasterizer",):
            if model_config[field] != config[field]:
                raise ValueError(f"{run}: incompatible {field}")
        net = trainer.make_net(model_config)
        net.load_state_dict(torch.load(run / "best.pt", weights_only=True))
        net.eval()
        outputs = {}
        with torch.no_grad():
            for name, x in inputs.items():
                logits = torch.cat([net(batch) for batch in x.split(256)])
                scores = trainer.metrics(logits, y, records)
                for dataset, metrics in scores.items():
                    indices = [
                        i for i, row in enumerate(records) if row["dataset"] == dataset
                    ]
                    metrics["cross_entropy_92"] = F.cross_entropy(
                        logits[indices], y[indices]
                    ).item()
                    del metrics["confusion"], metrics["labels"]
                outputs[name] = scores
        templates = json.loads(
            (trainer.ROOT / "crates/core/assets/kana_templates.json").read_text()
        )["templates"]
        canonical = {}
        for template in templates:
            canonical.setdefault(template["label"], template["strokes"])
        if set(canonical) != set(trainer.LABELS):
            raise ValueError("Incomplete canonical references")
        strokes = [
            [[{"x": x, "y": y} for x, y in stroke] for stroke in canonical[label]]
            for label in trainer.LABELS
        ]
        raster_bytes = subprocess.check_output(
            [
                str(trainer.ROOT / "target/release/kana-rasterize"),
                "--ink-batch",
                "--side",
                str(side),
            ],
            input=json.dumps(strokes).encode(),
        )
        canonical_x = torch.from_numpy(
            np.frombuffer(raster_bytes, dtype="<f4").copy()
        ).reshape(92, 1, side, side)
        canonical_y = torch.arange(92)
        with torch.no_grad():
            canonical_logits = net(canonical_x)
            canonical_both_top1 = int((canonical_logits.argmax(1) == canonical_y).sum())
        known_logits = canonical_logits.clone()
        known_logits[canonical_y < 46, 46:] = -torch.inf
        known_logits[canonical_y >= 46, :46] = -torch.inf
        result["models"][str(run)] = {
            "canonical_known_top1": int((known_logits.argmax(1) == canonical_y).sum()),
            "canonical_both_top1": canonical_both_top1,
            "canonical_samples": len(canonical),
            "binary_model_sha256": hashlib.sha256(
                (run / "model.bin").read_bytes()
            ).hexdigest()
            if (run / "model.bin").exists()
            else None,
            "checkpoint_sha256": hashlib.sha256(
                (run / "best.pt").read_bytes()
            ).hexdigest(),
            "canonical_predictions": [
                {
                    "expected": label,
                    "both": trainer.LABELS[int(canonical_logits[i].argmax())],
                    "known": trainer.LABELS[int(known_logits[i].argmax())],
                }
                for i, label in enumerate(trainer.LABELS)
            ],
            "results": outputs,
        }
    return result


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument("cohort", type=Path)
    p.add_argument("models", type=Path, nargs="+")
    p.add_argument("--output", type=Path)
    p.add_argument("--threads", type=int, default=3)
    a = p.parse_args()
    if a.threads < 1:
        raise ValueError("threads must be positive")
    torch.set_num_threads(a.threads)
    torch.use_deterministic_algorithms(True)
    result = compare(a.cohort, a.models)
    encoded = json.dumps(result, ensure_ascii=False, indent=2) + "\n"
    if a.output:
        a.output.write_text(encoded)
    print(encoded, end="")


if __name__ == "__main__":
    main()
