#!/usr/bin/env python3
"""Matched validation gains with descriptive split-group bootstrap intervals.

Groups are the dataset's frozen split groups, not verified modern pen writers.
Intervals do not correct for repeated model selection on this validation cohort.
"""

import argparse
import importlib.util
import json
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
    p.add_argument("cohort", type=Path)
    p.add_argument("baseline", type=Path)
    p.add_argument("candidate", type=Path)
    p.add_argument("--output", type=Path, required=True)
    a = p.parse_args()
    torch.set_num_threads(1)
    split_bytes = (a.cohort / "split.json").read_bytes()
    split = json.loads(split_bytes)
    ids = [i for i, row in enumerate(split["records"]) if row["split"] == "validation"]
    records = [split["records"][i] for i in ids]
    with np.load(a.cohort / "prepared.npz") as data:
        x = torch.from_numpy(data["proxy_images"][ids])
        y = torch.from_numpy(data["labels"][ids])
    correct, hashes = [], []
    for run in [a.baseline, a.candidate]:
        if (run / "split.json").read_bytes() != split_bytes:
            raise ValueError("Frozen cohorts differ")
        config = json.loads((run / "config.json").read_text())
        net = trainer.make_net(config)
        net.load_state_dict(torch.load(run / "best.pt", weights_only=True))
        net.eval()
        with torch.no_grad():
            logits = torch.cat([net(batch) for batch in x.split(128)])
        logits[y < 46, 46:] = -torch.inf
        logits[y >= 46, :46] = -torch.inf
        correct.append((logits.argmax(1) == y).numpy())
        hashes.append(trainer.digest((run / "best.pt").read_bytes()))
    rng = np.random.default_rng(8723)
    result = {}
    draws = {}
    for dataset in sorted({row["dataset"] for row in records}):
        indices = [i for i, row in enumerate(records) if row["dataset"] == dataset]
        before, after = [values[indices] for values in correct]
        groups = {}
        for i in indices:
            n, gain = groups.get(records[i]["group"], (0, 0))
            groups[records[i]["group"]] = (
                n + 1,
                gain + int(correct[1][i]) - int(correct[0][i]),
            )
        values = np.array(list(groups.values()))
        if len(values) < 2:
            raise ValueError("Need at least two split groups")
        samples = values[rng.integers(0, len(values), size=(2000, len(values)))].sum(
            axis=1
        )
        draws[dataset] = 100 * samples[:, 1] / samples[:, 0]
        result[dataset] = {
            "samples": len(indices),
            "split_groups": len(groups),
            "fixed": int((~before & after).sum()),
            "regressed": int((before & ~after).sum()),
            "gain_pp": 100 * (after.mean() - before.mean()),
            "descriptive_group_bootstrap_95_pp": np.quantile(
                draws[dataset], [0.025, 0.975]
            ).tolist(),
        }
    output = {
        "split": "validation",
        "cohort": str(a.cohort),
        "baseline": str(a.baseline),
        "candidate": str(a.candidate),
        "checkpoint_sha256": hashes,
        "results": result,
        "mean_hiragana_group_bootstrap_95_pp": np.quantile(
            (draws["etl4"] + draws["etl7"]) / 2, [0.025, 0.975]
        ).tolist(),
        "bootstrap": {"seed": 8723, "replicates": 2000},
        "note": "Descriptive paired resampling of frozen split groups on reused validation. No correction for model selection. ETL5/7 groups are not verified writer identities. Not an iPad accuracy interval or a new promotion gate.",
    }
    a.output.write_text(json.dumps(output, indent=2) + "\n")
    print(json.dumps(output, indent=2))


if __name__ == "__main__":
    main()
