#!/usr/bin/env python3
"""Explore answer-independent missing-stroke hypotheses in CNN feature space.

The bank contains embedded valid controls and single-stroke deletions. A closer
deleted reference is a repair hypothesis, NOT proof of malformed handwriting.
Public-source controls measure false flags; no expected answer enters matching.
"""
import argparse
from collections import defaultdict
import hashlib
import importlib.util
import json
from pathlib import Path
import subprocess

import numpy as np
import torch
from torch.nn import functional as F

spec = importlib.util.spec_from_file_location("trainer", Path(__file__).with_name("train-kana-vision.py"))
trainer = importlib.util.module_from_spec(spec)
spec.loader.exec_module(trainer)


def repair_flag(has_ink, clean, ranked, cnn, policy):
    """An alternative identity is ambiguity, not evidence of a writing defect."""
    supported = cnn[:1] if policy["cnn_support"] == "top1" else cnn
    return (has_ink and clean > 1e-6
            and ranked[0]["distance"] <= policy["maximum_cosine_distance"]
            and ranked[0]["distance"] < clean * policy["maximum_edit_to_valid_ratio"]
            and ranked[1]["distance"] - ranked[0]["distance"] >= policy["minimum_parent_margin"]
            and ranked[0]["character"] in supported)


def self_test():
    policy = dict(maximum_cosine_distance=.2, maximum_edit_to_valid_ratio=.65,
                  minimum_parent_margin=.005, cnn_support="top1")
    ranked = [dict(character="チ", distance=.05), dict(character="ナ", distance=.10)]
    assert not repair_flag(True, .15, ranked, ["ナ", "チ"], policy)
    assert repair_flag(True, .15, ranked, ["チ", "ナ"], policy)
    assert not repair_flag(False, .15, ranked, ["チ", "ナ"], policy)
    assert not repair_flag(True, .04, ranked, ["チ", "ナ"], policy)
    print("Repair decision tests passed: valid alternative, parent agreement, empty ink, better clean match")


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument("corpus", type=Path, nargs="?")
    p.add_argument("model", type=Path, nargs="?")
    p.add_argument("--output", type=Path)
    p.add_argument("--cnn-support", choices=["top1", "top5"], default="top1")
    p.add_argument("--self-test", action="store_true")
    a = p.parse_args()
    if a.self_test:
        self_test()
        return
    if a.corpus is None or a.model is None or a.output is None:
        p.error("corpus, model and --output are required")
    if a.output.exists():
        p.error("Never overwrite a diagnostic")
    raw = (a.corpus / "samples.jsonl").read_bytes()
    manifest = json.loads((a.corpus / "manifest.json").read_text())
    assert hashlib.sha256(raw).hexdigest() == manifest["samples_sha256"]
    rows = [json.loads(line) for line in raw.splitlines()]
    config = json.loads((a.model / "config.json").read_text())
    torch.set_num_threads(3)
    net = trainer.make_net(config)
    net.load_state_dict(torch.load(a.model / "best.pt", weights_only=True))
    net.eval()
    a.output.mkdir(parents=True)
    features, logits = [], []
    handle = net.fc.register_forward_pre_hook(lambda module, inputs: features.append(F.normalize(inputs[0].detach(), dim=1)))
    with torch.no_grad():
        for start in range(0, len(rows), 128):
            batch = rows[start:start+128]
            raster = subprocess.check_output([str(trainer.ROOT / "target/release/kana-rasterize"), "--ink-batch", "--side", "64"],
                input=json.dumps([r["strokes"] for r in batch]).encode())
            x = torch.from_numpy(np.frombuffer(raster, dtype="<f4").copy()).reshape(-1, 1, 64, 64)
            logits.append(net(x))
    handle.remove()
    features, logits = torch.cat(features), torch.cat(logits)
    predictions = []
    # Numerical thresholds were frozen before v1 cross-source output. V2 requires
    # top-1 identity agreement after v1 exposed valid alternative false flags.
    policy = dict(maximum_cosine_distance=.2, maximum_edit_to_valid_ratio=.65,
                  minimum_parent_margin=.005, cnn_support=a.cnn_support)
    for script, offset in [("hiragana", 0), ("katakana", 46)]:
        valid = [i for i, r in enumerate(rows) if r["source"] == "embedded" and r["script"] == script and r["kind"] == "valid_control"]
        edits = [i for i, r in enumerate(rows) if r["source"] == "embedded" and r["script"] == script and r["operation"].startswith("omit_")]
        assert valid and edits
        ids = [i for i, r in enumerate(rows) if r["script"] == script]
        for batch in torch.tensor(ids).split(128):
            clean_distance = (1 - features[batch] @ features[valid].T).clamp_min(0)
            edit_distance = (1 - features[batch] @ features[edits].T).clamp_min(0)
            for j, index in enumerate(batch.tolist()):
                row = rows[index]
                ci = int(clean_distance[j].argmin())
                by_parent = {}
                for k, prototype in enumerate(edits):
                    parent = rows[prototype]["parent_character"]
                    distance = float(edit_distance[j, k])
                    if parent not in by_parent or distance < by_parent[parent]["distance"]:
                        by_parent[parent] = dict(character=parent, distance=distance, prototype_id=rows[prototype]["id"],
                                                 missing_stroke=rows[prototype]["affected_stroke"])
                ranked = sorted(by_parent.values(), key=lambda r: (r["distance"], r["character"]))
                closest = ranked[0]
                clean = float(clean_distance[j, ci])
                cnn = [trainer.LABELS[offset + k] for k in logits[index, offset:offset+46].topk(5).indices.tolist()]
                flag = repair_flag(bool(row["strokes"]), clean, ranked, cnn, policy)
                predictions.append(dict(id=row["id"], source=row["source"], kind=row["kind"], operation=row["operation"],
                    parent_character=row["parent_character"], cnn_top1=cnn[0],
                    closest_valid=dict(character=rows[valid[ci]]["parent_character"], distance=clean),
                    repair_candidates=ranked[:3], possible_missing_stroke=flag))
    groups = defaultdict(list)
    for row in predictions:
        category = "omission" if row["operation"].startswith("omit_") else row["kind"]
        groups[f"{row['source']}:{category}"].append(row)
    summary = {}
    for key, values in groups.items():
        flagged = [r for r in values if r["possible_missing_stroke"]]
        summary[key] = dict(samples=len(values), flagged=len(flagged), flagged_rate=len(flagged)/len(values),
            parent_top1=sum(r["repair_candidates"][0]["character"] == r["parent_character"] for r in values)/len(values),
            flagged_parent_correct=sum(r["repair_candidates"][0]["character"] == r["parent_character"] for r in flagged),
            note="Parent recovery is lineage retrieval, not verified intent or malformed-ink accuracy.")
    trainer.write(a.output / "summary.json", dict(policy=policy, groups=summary, corpus_sha256=manifest["samples_sha256"],
        checkpoint_sha256=hashlib.sha256((a.model / "best.pt").read_bytes()).hexdigest(),
        script_sha256=hashlib.sha256(Path(__file__).read_bytes()).hexdigest(),
        limitation="Experimental missing-stroke hypotheses only. Embedded deletion probes are in the bank, so their self-retrieval is not a generalization score. No feedback deployed."))
    with (a.output / "predictions.jsonl").open("w") as stream:
        for row in predictions:
            stream.write(json.dumps(row, ensure_ascii=False) + "\n")
    (a.output / "source.py").write_bytes(Path(__file__).read_bytes())
    print(json.dumps(summary, indent=2))


if __name__ == "__main__":
    main()
