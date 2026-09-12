#!/usr/bin/env python3
"""Controlled る/ろ rotation/resampling ablation; no model or policy tuning."""
import argparse
from collections import defaultdict
import hashlib
import importlib.util
import json
import math
import random
from pathlib import Path
import subprocess

ROOT = Path(__file__).resolve().parent.parent
spec = importlib.util.spec_from_file_location("builder", Path(__file__).with_name("build-kana-beginner-corpus.py"))
builder = importlib.util.module_from_spec(spec)
spec.loader.exec_module(builder)


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument("--corpus", type=Path, required=True)
    p.add_argument("--model", type=Path, required=True)
    p.add_argument("--output", type=Path, required=True)
    a=p.parse_args()
    if a.output.exists():
        p.error("Never overwrite a diagnostic")
    raw=(a.corpus/"samples.jsonl").read_bytes()
    manifest=json.loads((a.corpus/"manifest.json").read_text())
    assert hashlib.sha256(raw).hexdigest()==manifest["samples_sha256"]
    originals=list(map(json.loads,raw.splitlines()))
    rows=[]
    for row in originals:
        if row["parent_character"] not in ("る", "ろ") or row["operation"]!="canonical":
            continue
        ink=builder.xy(row["strokes"])
        for samples in (None,16,32,48,96):
            sampled=ink if samples is None else [builder.resample(s,samples) for s in ink]
            for degrees in range(-12,13,3):
                angle=math.radians(degrees)
                transformed=[[(50+(x-50)*math.cos(angle)-(y-50)*math.sin(angle),
                               50+(x-50)*math.sin(angle)+(y-50)*math.cos(angle)) for x,y in s] for s in sampled]
                probe=builder.document(row["source"],row["parent_character"],row["script"],
                    f"rotate_{degrees}_samples_{samples}",transformed,"valid_control")
                probe.update(degrees=degrees,resampling=samples)
                rows.append(probe)
    # Reproduce the known combined-transform misses alongside isolated factors.
    rows += [dict(r,degrees=None,resampling="combined") for r in originals
             if r["parent_character"]=="る" and r["source"]=="animcjk_clipped_hiragana"
             and r["operation"].startswith("mild_shape_")]
    # Same combined-transform parameters, changing only path sampling density.
    for original in originals:
        if original["parent_character"]!="る" or original["source"]!="animcjk_clipped_hiragana" or original["operation"]!="canonical":
            continue
        seed=int(builder.digest("clipped-v1:20260913:る".encode())[:16],16)
        for count in (32,48,96,192):
            rng=random.Random(seed)
            for variant in range(6):
                angle=math.radians(rng.uniform(-6,6))
                sx,sy,shear=rng.uniform(.94,1.06),rng.uniform(.94,1.06),rng.uniform(-.04,.04)
                phase=rng.uniform(0,math.tau)
                transformed=[]
                for stroke in builder.xy(original["strokes"]):
                    changed=[]
                    for j,(x,y) in enumerate(builder.resample(stroke,count)):
                        xx,yy=(x-50)*sx+shear*(y-50),(y-50)*sy
                        wobble=.6*math.sin(phase+j/(count-1)*math.tau)
                        changed.append((50+xx*math.cos(angle)-yy*math.sin(angle)+wobble,
                                        50+xx*math.sin(angle)+yy*math.cos(angle)+wobble))
                    transformed.append(changed)
                probe=builder.document(original["source"],"る","hiragana",
                    f"combined_{variant}_samples_{count}",transformed,"valid_control")
                probe.update(degrees=math.degrees(angle),resampling=f"combined_{count}")
                if count==32:
                    previous=next(r for r in rows if r["source"]==original["source"] and r["operation"]==f"mild_shape_{variant}")
                    # Archived canonical coordinates were rounded to six decimals.
                    assert len(probe["strokes"])==len(previous["strokes"])
                    assert max(abs(a[k]-b[k]) for left,right in zip(probe["strokes"],previous["strokes"],strict=True)
                               for a,b in zip(left,right,strict=True) for k in ("x","y")) <= 2e-6
                rows.append(probe)
    predictions=[]
    for start in range(0,len(rows),64):
        batch=rows[start:start+64]
        predictions.extend(json.loads(subprocess.check_output([str(ROOT/"target/release/kana-vision"),"--model",str(a.model)],
            input=json.dumps([dict(script=r["script"],strokes=r["strokes"]) for r in batch]).encode())))
    result=[]
    groups=defaultdict(list)
    for row,prediction in zip(rows,predictions,strict=True):
        candidates=prediction["vision"]["candidates"]
        out=dict(row,candidates=candidates,correct=bool(candidates) and candidates[0]["character"]==row["parent_character"])
        result.append(out)
        groups[f"{row['source']}:{row['parent_character']}:samples_{row['resampling']}"].append(out)
    summary={key:dict(samples=len(v),correct=sum(r["correct"] for r in v),
        misses=[dict(degrees=r["degrees"],operation=r["operation"],candidates=r["candidates"][:2]) for r in v if not r["correct"]]) for key,v in groups.items()}
    a.output.mkdir(parents=True)
    (a.output/"predictions.jsonl").write_text("".join(json.dumps(r,ensure_ascii=False)+"\n" for r in result))
    (a.output/"summary.json").write_text(json.dumps(dict(groups=summary,model_sha256=builder.digest(a.model.read_bytes()),
        corpus_sha256=builder.digest(raw),
        limitation="Targeted diagnostic after observing failures; not held-out accuracy. Rotations up to 12 degrees are robustness probes, not certified valid beginner forms."),ensure_ascii=False,indent=2)+"\n")
    (a.output/"source.py").write_bytes(Path(__file__).read_bytes())
    (a.output/"resampler.py").write_bytes(Path(builder.__file__).read_bytes())
    print(json.dumps(summary,ensure_ascii=False,indent=2))


if __name__=="__main__":
    main()
