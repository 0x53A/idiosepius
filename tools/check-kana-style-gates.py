#!/usr/bin/env python3
"""Combine predeclared style adaptation gates; confirmation remains separate."""
import argparse
import hashlib
import json
from pathlib import Path


def high_confidence_invalids(directory):
    rows=map(json.loads,(directory/"predictions.jsonl").read_text().splitlines())
    return {r["id"] for r in rows if r["kind"]=="non_kana" and r["vision"]["candidates"]
            and r["vision"]["candidates"][0]["score"]>=.99}


def main():
    p=argparse.ArgumentParser(description=__doc__)
    p.add_argument("--run",type=Path,required=True)
    p.add_argument("--audit",type=Path,required=True)
    p.add_argument("--baseline-audit",type=Path,required=True)
    a=p.parse_args()
    output=a.run/"style-gates.json"
    if output.exists():
        p.error("Never overwrite a gate report")
    original=json.loads((a.run/"gates.json").read_text())
    screen=json.loads((a.run/"shape-screen.json").read_text())
    current=json.loads((a.audit/"summary.json").read_text())
    baseline=json.loads((a.baseline_audit/"summary.json").read_text())
    sha=hashlib.sha256((a.run/"model.bin").read_bytes()).hexdigest()
    assert sha==current["model_sha256"]==screen["model_sha256"]
    assert original["model_sha256"]==sha
    assert baseline["model_sha256"]==screen["baseline_sha256"]
    assert baseline["corpus_sha256"]==current["corpus_sha256"]==screen["corpus_sha256"]
    failures=[]
    if not original["eligible_for_confirmation"]:
        failures.append("Original promotion gates failed")
    if not screen["passed_early_checks"]:
        failures.extend(screen["failures"])
    if screen["small_loop_correct"]["candidate"]!=6:
        failures.append("Not all six observed loop controls correct")
    positives={}
    for source in ["embedded","kanjivg","animcjk_clipped_hiragana","animcjk_clipped_katakana_overlap"]:
        key=source+":valid_control"
        new,old=current["groups"][key],baseline["groups"][key]
        assert new["samples"]==old["samples"]
        score=new["vision"]["parent_top1"]
        positives[source]=dict(samples=new["samples"],top1=score,baseline_top1=old["vision"]["parent_top1"])
        required=1. if source in ["embedded","kanjivg"] else old["vision"]["parent_top1"]
        if score<required:
            failures.append(f"{source} positive controls below required {required}")
    additional=sorted(high_confidence_invalids(a.audit)-high_confidence_invalids(a.baseline_audit))
    if additional:
        failures.append("New >=0.99-softmax non-kana cases")
    result=dict(eligible_for_confirmation=not failures,failures=failures,positive_controls=positives,
        new_high_confidence_invalid_ids=additional,model_sha256=sha,
        original_gates_sha256=hashlib.sha256((a.run/"gates.json").read_bytes()).hexdigest(),
        screen_sha256=hashlib.sha256((a.run/"shape-screen.json").read_bytes()).hexdigest(),
        audit_summary_sha256=hashlib.sha256((a.audit/"summary.json").read_bytes()).hexdigest(),
        note="Confirmation eligibility only. Require independent seed and Wasm parity before deployment. AnimCJK hiragana is adapted development-source data, not independent accuracy.")
    output.write_text(json.dumps(result,indent=2)+"\n")
    print(json.dumps(result,indent=2))


if __name__=="__main__":
    main()
