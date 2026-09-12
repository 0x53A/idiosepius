#!/usr/bin/env python3
"""Export train/validation proxy arrays for a frozen stroke-domain fine-tuning trial.

Test records are excluded from the exported data. ETL derivatives remain private.
Only embedded-source valid controls are synthetic training data; KanjiVG and all
corruption probes are excluded. Both existing ETL preparations are preserved.
"""
import argparse
import hashlib
import json
from pathlib import Path
import subprocess

import numpy as np

ROOT = Path(__file__).resolve().parent.parent


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument("--original", type=Path, required=True)
    p.add_argument("--clean", type=Path, required=True)
    p.add_argument("--corpus", type=Path, required=True)
    p.add_argument("--output", type=Path, required=True)
    a = p.parse_args()
    if a.output.exists() or not a.output.resolve().is_relative_to((ROOT / "kana-artifacts").resolve()):
        p.error("Choose a new directory under kana-artifacts/")
    split_bytes = (a.original / "split.json").read_bytes()
    assert split_bytes == (a.clean / "split.json").read_bytes()
    split = json.loads(split_bytes)
    a.output.mkdir(parents=True)
    arrays, records = {}, {}
    for partition in ["train", "validation"]:
        ids = [i for i, r in enumerate(split["records"]) if r["split"] == partition]
        records[partition] = [split["records"][i] for i in ids]
        for name, directory in [("original", a.original), ("clean", a.clean)]:
            with np.load(directory / "prepared.npz") as data:
                labels = data["labels"][ids]
                arrays[f"{partition}_{name}"] = data["proxy_images"][ids]
                if f"{partition}_labels" in arrays:
                    assert np.array_equal(arrays[f"{partition}_labels"], labels)
                arrays[f"{partition}_labels"] = labels
    manifest = json.loads((a.corpus / "manifest.json").read_text())
    assert sha(a.corpus / "samples.jsonl") == manifest["samples_sha256"]
    samples = [json.loads(line) for line in (a.corpus / "samples.jsonl").read_text().splitlines()]
    samples = [r for r in samples if r["source"] == "embedded" and r["kind"] == "valid_control"]
    assert len(samples) == 1104 and all(r["partition"] == "development" for r in samples)
    labels = [t["label"] for t in json.loads((ROOT / "crates/core/assets/kana_templates.json").read_text())["templates"]]
    raw = subprocess.check_output([str(ROOT / "target/release/kana-rasterize"), "--ink-batch", "--side", "64"],
                                  input=json.dumps([r["strokes"] for r in samples]).encode())
    arrays["synthetic_images"] = np.frombuffer(raw, dtype="<f4").copy().reshape(-1, 1, 64, 64)
    arrays["synthetic_labels"] = np.array([labels.index(r["parent_character"]) for r in samples])
    assert not {r["group"] for r in records["train"]} & {r["group"] for r in records["validation"]}
    np.savez_compressed(a.output / "trial.npz", **arrays)
    (a.output / "records.json").write_text(json.dumps(records) + "\n")
    provenance = dict(version="kana-stroke-trial-preparation-v1", partition_counts={k: len(v) for k, v in records.items()},
        synthetic_count=len(samples), synthetic_ids=[r["id"] for r in samples],
        sources={str(d): sha(d / "prepared.npz") for d in [a.original, a.clean]},
        split_sha256=hashlib.sha256(split_bytes).hexdigest(), corpus_sha256=manifest["samples_sha256"],
        trial_sha256=sha(a.output / "trial.npz"), records_sha256=sha(a.output / "records.json"),
        script_sha256=sha(Path(__file__)),
        note="Only train and validation ETL proxy rows exported; no test, KanjiVG, or corruptions in training. Restricted ETL derivatives; do not publish.")
    (a.output / "preparation.json").write_text(json.dumps(provenance, indent=2) + "\n")
    (a.output / "prepare.py").write_bytes(Path(__file__).read_bytes())
    print(json.dumps({k: provenance[k] for k in ["partition_counts", "synthetic_count", "trial_sha256"]}, indent=2))


if __name__ == "__main__":
    main()
