#!/usr/bin/env python3
"""Apply the frozen quality-first validation gates to checkpoint comparisons.

Passing these gates only establishes eligibility for further checks. It does not
install weights or establish real pen accuracy, calibration, or runtime parity.
"""

import argparse
import json
from pathlib import Path


def assess(baseline_clean, baseline_original, candidate_clean, candidate_original):
    clean = candidate_clean["results"]["proxy_images"]
    original = candidate_original["results"]["proxy_images"]
    old_clean = baseline_clean["results"]["proxy_images"]
    old_original = baseline_original["results"]["proxy_images"]
    delta_clean = {
        d: 100 * (clean[d]["top1"] - old_clean[d]["top1"]) for d in old_clean
    }
    delta_original = {
        d: 100 * (original[d]["top1"] - old_original[d]["top1"]) for d in old_original
    }
    gain = (delta_clean["etl4"] + delta_clean["etl7"]) / 2
    failures = []
    if gain < 0.5:
        failures.append("Mean cleaned hiragana gain below 0.5 percentage points")
    for d in ["etl5", "etl7"]:
        if delta_clean[d] < -0.5:
            failures.append(f"Clean {d} loss exceeds 0.5 percentage points")
    for d, limit in [("etl4", 1), ("etl5", 0.5), ("etl7", 0.5)]:
        if delta_original[d] < -limit:
            failures.append(f"Original {d} loss exceeds {limit} percentage points")
    for field in ["canonical_known_top1", "canonical_both_top1"]:
        if candidate_clean[field] < baseline_clean[field] - 2:
            failures.append(f"{field} loses more than two identities")
    return {
        "validation_gates_passed": not failures,
        "mean_clean_hiragana_gain_pp": gain,
        "clean_deltas_pp": delta_clean,
        "original_deltas_pp": delta_original,
        "failures": failures,
    }


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument("clean", type=Path)
    p.add_argument("original", type=Path)
    p.add_argument("--baseline", required=True)
    p.add_argument("--output", type=Path, required=True)
    a = p.parse_args()
    clean, original = [json.loads(path.read_text()) for path in [a.clean, a.original]]
    if clean["split"] != "validation" or original["split"] != "validation":
        raise ValueError("Only validation comparisons are eligible")
    if not clean["clean_scans"] or original["clean_scans"]:
        raise ValueError("Require corrected and original preparations separately")
    if clean["split_sha256"] != original["split_sha256"]:
        raise ValueError("Frozen cohorts differ")
    if set(clean["models"]) != set(original["models"]):
        raise ValueError("Model inventories differ")
    for name in clean["models"]:
        if (
            clean["models"][name]["checkpoint_sha256"]
            != original["models"][name]["checkpoint_sha256"]
        ):
            raise ValueError("Checkpoint changed between comparisons")
    results = {
        name: assess(
            clean["models"][a.baseline],
            original["models"][a.baseline],
            row,
            original["models"][name],
        )
        for name, row in clean["models"].items()
        if name != a.baseline
    }
    a.output.write_text(
        json.dumps(
            {
                "baseline": a.baseline,
                "candidates": results,
                "note": "Eligibility only. Parameter budget, alternate seeds, portable parity, references and invalid ink require separate evidence.",
            },
            indent=2,
        )
        + "\n"
    )
    print(json.dumps(results, indent=2))


if __name__ == "__main__":
    main()
