#!/usr/bin/env python3
"""Early shape-trial rejection checks; passing is not promotion eligibility."""
import argparse
import hashlib
import json
from pathlib import Path
import subprocess

ROOT=Path(__file__).resolve().parent.parent


def main():
    p=argparse.ArgumentParser(description=__doc__)
    p.add_argument("--run",type=Path,required=True)
    p.add_argument("--baseline",type=Path,required=True)
    p.add_argument("--corpus",type=Path,required=True)
    p.add_argument("--required-loop-correct",type=int,choices=range(7),default=0,
                   help="Additional predeclared absolute requirement; improvement remains required")
    a=p.parse_args()
    output=a.run/"shape-screen.json"
    if output.exists():
        p.error("Never overwrite a diagnostic")
    report=json.loads((a.run/"report.json").read_text())
    baseline=json.loads((a.baseline/"report.json").read_text())
    model=a.run/"model.bin"
    assert hashlib.sha256(model.read_bytes()).hexdigest()==report["model_sha256"]
    assert hashlib.sha256((a.baseline/"model.bin").read_bytes()).hexdigest()==baseline["model_sha256"]
    raw=(a.corpus/"samples.jsonl").read_bytes()
    manifest=json.loads((a.corpus/"manifest.json").read_text())
    assert hashlib.sha256(raw).hexdigest()==manifest["samples_sha256"]
    rows=[r for r in map(json.loads,raw.splitlines()) if r["source"]=="animcjk_clipped_hiragana"
          and r["parent_character"]=="る" and r["operation"].startswith("mild_shape_")]
    assert len(rows)==6
    payload=json.dumps([dict(strokes=r["strokes"],script=r["script"]) for r in rows]).encode()
    scores={}
    predictions={}
    for name,path in [("baseline",a.baseline/"model.bin"),("candidate",model)]:
        actual=json.loads(subprocess.check_output([str(ROOT/"target/release/kana-vision"),"--model",str(path)],input=payload))
        predictions[name]=[dict(id=r["id"],candidates=v["vision"]["candidates"]) for r,v in zip(rows,actual,strict=True)]
        scores[name]=sum(bool(v["vision"]["candidates"]) and v["vision"]["candidates"][0]["character"]==r["parent_character"]
                         for r,v in zip(rows,actual,strict=True))
    deltas={k:100*(v["top1"]-baseline["validation"][k]["top1"]) for k,v in report["validation"].items()}
    failures=[f"{k} delta {v:.6f}pp below -0.25pp" for k,v in deltas.items() if v < -.25]
    if scores["candidate"]<=scores["baseline"]:
        failures.append("No improvement on the six observed small-loop controls")
    if scores["candidate"]<a.required_loop_correct:
        failures.append(f"Fewer than {a.required_loop_correct} required small-loop controls correct")
    golden=json.loads((a.run/"synthetic-golden.json").read_text())
    native=json.loads(subprocess.check_output([str(ROOT/"target/release/kana-rasterize"),"--logits","--model",str(model)],
        input=json.dumps(golden["input"]).encode()))
    delta=max(abs(x-y) for left,right in zip(native,golden["logits"],strict=True) for x,y in zip(left,right,strict=True))
    if delta>=.0002:
        failures.append("Native synthetic-logit parity failed")
    result=dict(passed_early_checks=not failures,failures=failures,etl_delta_pp=deltas,
        small_loop_correct=scores,predictions=predictions,native_logit_delta=delta,
        model_sha256=report["model_sha256"],baseline_sha256=baseline["model_sha256"],corpus_sha256=manifest["samples_sha256"],
        remaining_gates="Original archived-baseline gates, all 2208 positive controls, clipped controls, false positives, independent seed confirmation and Wasm parity. Passing this screen is not permission to deploy.")
    output.write_text(json.dumps(result,ensure_ascii=False,indent=2)+"\n")
    print(json.dumps({k:v for k,v in result.items() if k!="predictions"},indent=2))


if __name__=="__main__":
    main()
