#!/usr/bin/env python3
"""Probe training-style thickness changes on る/ろ; transformed validity is unknown."""
import argparse
import hashlib
import json
from pathlib import Path
import subprocess

import numpy as np

ROOT=Path(__file__).resolve().parent.parent


def morphology(images, maximum):
    padded=np.pad(images,((0,0),(1,1),(1,1)),constant_values=0 if maximum else 1)
    windows=np.lib.stride_tricks.sliding_window_view(padded,(3,3),axis=(1,2))
    return windows.max(axis=(-2,-1)) if maximum else windows.min(axis=(-2,-1))


def main():
    p=argparse.ArgumentParser(description=__doc__)
    p.add_argument("--corpus",type=Path,required=True)
    p.add_argument("--model",type=Path,required=True)
    p.add_argument("--output",type=Path,required=True)
    a=p.parse_args()
    if a.output.exists():
        p.error("Never overwrite a diagnostic")
    raw=(a.corpus/"samples.jsonl").read_bytes()
    assert hashlib.sha256(raw).hexdigest()==json.loads((a.corpus/"manifest.json").read_text())["samples_sha256"]
    rows=[r for r in map(json.loads,raw.splitlines()) if r["parent_character"] in ("る","ろ")
          and (r["operation"]=="canonical" or (r["source"]=="animcjk_clipped_hiragana" and r["operation"].startswith("mild_shape_")))]
    raster=subprocess.check_output([str(ROOT/"target/release/kana-rasterize"),"--ink-batch","--side","64"],
        input=json.dumps([r["strokes"] for r in rows]).encode())
    images=np.frombuffer(raster,dtype="<f4").reshape(-1,64,64)
    labels=[t["label"] for t in json.loads((ROOT/"crates/core/assets/kana_templates.json").read_text())["templates"]]
    result=[]
    for name,ink in [("original",images),("dilated",morphology(images,True)),("eroded",morphology(images,False))]:
        logits=json.loads(subprocess.check_output([str(ROOT/"target/release/kana-rasterize"),"--logits","--model",str(a.model)],
            input=json.dumps(ink.reshape(-1,4096).tolist()).encode()))
        for row,z,x in zip(rows,logits,ink,strict=True):
            order=sorted(range(46),key=lambda i:z[i],reverse=True)
            result.append(dict(id=row["id"],operation=name,parent=row["parent_character"],
                top1=labels[order[0]],ru_minus_ro_logit=z[labels.index("る")]-z[labels.index("ろ")],
                ink_mass=float(x.sum()),validity="unknown" if name!="original" else "reference_control"))
    a.output.mkdir(parents=True)
    (a.output/"predictions.json").write_text(json.dumps(result,ensure_ascii=False,indent=2)+"\n")
    (a.output/"summary.json").write_text(json.dumps(dict(model_sha256=hashlib.sha256(a.model.read_bytes()).hexdigest(),
        corpus_sha256=hashlib.sha256(raw).hexdigest(),samples=len(result),
        note="Isolates 3x3 max/min-pooling thickness changes from affine augmentation. Changed identity is not proof the transformed ink is invalid. No training or deployment."),indent=2)+"\n")
    (a.output/"source.py").write_bytes(Path(__file__).read_bytes())
    for name in ["original","dilated","eroded"]:
        values=[r for r in result if r["operation"]==name]
        print(name,sum(r["top1"]==r["parent"] for r in values),"/",len(values))


if __name__=="__main__":
    main()
