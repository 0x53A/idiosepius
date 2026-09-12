#!/usr/bin/env python3
"""Research-only deletion retrieval using symmetric raster Chamfer distance.

The bank uses embedded canonical references and their single-stroke deletions.
KanjiVG is evaluation-only. Parent labels measure lineage, not writing validity.
"""
import argparse
from collections import defaultdict
import hashlib
import json
from pathlib import Path
import subprocess

import numpy as np
from scipy.ndimage import distance_transform_edt

ROOT = Path(__file__).resolve().parent.parent
POLICY = dict(maximum_distance=.05, maximum_edit_to_valid_ratio=.65,
              minimum_parent_margin=.003, cnn_support="top1")


def representation(images):
    masks = images > .15
    distances = np.stack([distance_transform_edt(~mask) / images.shape[-1]
                          for mask in masks]).astype(np.float32)
    weights = masks.astype(np.float32).reshape(len(images), -1)
    weights /= np.maximum(weights.sum(axis=1, keepdims=True), 1)
    return weights, distances.reshape(len(images), -1)


def chamfer(left, right):
    lw, ld = left
    rw, rd = right
    return (lw @ rd.T + ld @ rw.T) / 2


def self_test():
    ink = np.zeros((3, 16, 16), dtype=np.float32)
    ink[0, 3:12, 4] = 1
    ink[1, 3:12, 5] = 1
    ink[2] = ink[0]
    result = chamfer(representation(ink), representation(ink))
    assert np.allclose(result, result.T)
    assert np.allclose(np.diag(result), 0)
    assert result[0, 2] == 0
    assert abs(result[0, 1] - 1/16) < 1e-6
    print("Chamfer checks passed: identity, symmetry, duplicate ink, known translation")


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument("--corpus", type=Path)
    p.add_argument("--audit", type=Path)
    p.add_argument("--output", type=Path)
    p.add_argument("--self-test", action="store_true")
    a = p.parse_args()
    if a.self_test:
        self_test()
        return
    if not all((a.corpus, a.audit, a.output)):
        p.error("--corpus, --audit and --output are required")
    if a.output.exists():
        p.error("Never overwrite an experiment")
    raw = (a.corpus / "samples.jsonl").read_bytes()
    manifest = json.loads((a.corpus / "manifest.json").read_text())
    assert hashlib.sha256(raw).hexdigest() == manifest["samples_sha256"]
    rows = [json.loads(line) for line in raw.splitlines()]
    audit_bytes = (a.audit / "predictions.jsonl").read_bytes()
    audit = {r["id"]: r for r in map(json.loads, audit_bytes.splitlines())}
    assert len(audit) == len(rows) and all(r["id"] in audit for r in rows)
    rasters = []
    for start in range(0, len(rows), 128):
        batch = rows[start:start+128]
        raw_ink = subprocess.check_output(
            [str(ROOT / "target/release/kana-rasterize"), "--ink-batch", "--side", "64"],
            input=json.dumps([r["strokes"] for r in batch]).encode())
        rasters.append(np.frombuffer(raw_ink, dtype="<f4").reshape(-1, 64, 64))
    weights, distances = representation(np.concatenate(rasters))
    result = []
    for script in ("hiragana", "katakana"):
        valid = [i for i, r in enumerate(rows) if r["source"] == "embedded"
                 and r["script"] == script and r["operation"] == "canonical"]
        edits = [i for i, r in enumerate(rows) if r["source"] == "embedded"
                 and r["script"] == script and r["operation"].startswith("omit_")]
        ids = [i for i, r in enumerate(rows) if r["script"] == script]
        bank = valid + edits
        matrix = chamfer((weights[ids], distances[ids]), (weights[bank], distances[bank]))
        for index, ds in zip(ids, matrix, strict=True):
            row = rows[index]
            ci = int(ds[:len(valid)].argmin())
            clean = float(ds[ci])
            parents = {}
            for k, prototype in enumerate(edits):
                parent = rows[prototype]["parent_character"]
                d = float(ds[len(valid)+k])
                if parent not in parents or d < parents[parent]["distance"]:
                    parents[parent] = dict(character=parent, distance=d,
                        prototype_id=rows[prototype]["id"], missing_stroke=rows[prototype]["affected_stroke"])
            ranked = sorted(parents.values(), key=lambda v: (v["distance"], v["character"]))
            cnn = audit[row["id"]]["vision"]["candidates"]
            top1 = cnn[0]["character"] if cnn else None
            geometric_flag = bool(weights[index].sum() > 0 and clean > 1e-6
                and ranked[0]["distance"] <= POLICY["maximum_distance"]
                and ranked[0]["distance"] < clean * POLICY["maximum_edit_to_valid_ratio"]
                and ranked[1]["distance"] - ranked[0]["distance"] >= POLICY["minimum_parent_margin"])
            result.append(dict(id=row["id"], source=row["source"], kind=row["kind"],
                operation=row["operation"], parent_character=row["parent_character"],
                closest_valid=dict(character=rows[valid[ci]]["parent_character"], distance=clean),
                repair_candidates=ranked[:3], cnn_top1=top1, geometric_flag=geometric_flag,
                possible_missing_stroke=geometric_flag and ranked[0]["character"] == top1))
    groups = defaultdict(list)
    for r in result:
        category = "omission" if r["operation"].startswith("omit_") else r["kind"]
        groups[f"{r['source']}:{category}"].append(r)
    summary = {}
    for key, values in groups.items():
        flagged = [r for r in values if r["possible_missing_stroke"]]
        summary[key] = dict(samples=len(values), flagged=len(flagged),
            geometric_flagged=sum(r["geometric_flag"] for r in values),
            parent_top1=sum(r["repair_candidates"][0]["character"] == r["parent_character"] for r in values),
            flagged_parent_correct=sum(r["repair_candidates"][0]["character"] == r["parent_character"] for r in flagged))
    a.output.mkdir(parents=True)
    (a.output / "predictions.jsonl").write_text("".join(json.dumps(r, ensure_ascii=False)+"\n" for r in result))
    (a.output / "summary.json").write_text(json.dumps(dict(policy=POLICY, groups=summary,
        corpus_sha256=manifest["samples_sha256"], audit_sha256=hashlib.sha256(audit_bytes).hexdigest(),
        rasterizer_sha256=hashlib.sha256((ROOT/"target/release/kana-rasterize").read_bytes()).hexdigest(),
        limitation="Diagnostic only. Deletions may be valid other kana; parent recovery is not accuracy. Embedded deletions are in the bank. No deployment."), indent=2)+"\n")
    (a.output / "source.py").write_bytes(Path(__file__).read_bytes())
    print(json.dumps(summary, indent=2))


if __name__ == "__main__":
    main()
