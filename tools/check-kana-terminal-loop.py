#!/usr/bin/env python3
"""Probe a source-anchored terminal loop without changing the rest of る.

These are shape hypotheses, not certified valid kana. The zero-scale endpoint
is an omission probe and must never be used as a positively labelled る sample.
"""
import argparse
import hashlib
import json
import math
from pathlib import Path
import subprocess

ROOT=Path(__file__).resolve().parent.parent


def deform(stroke, start, sx, sy):
    anchor=stroke[start]
    return [list(point) for point in stroke[:start]]+[
        [anchor[0]+(x-anchor[0])*sx,anchor[1]+(y-anchor[1])*sy] for x,y in stroke[start:]]


def main():
    p=argparse.ArgumentParser(description=__doc__)
    p.add_argument("--output",type=Path)
    p.add_argument("--self-test",action="store_true")
    a=p.parse_args()
    if a.self_test:
        stroke=[[0,0],[1,1],[2,2],[1,3]]
        assert deform(stroke,1,1,1)==stroke
        assert deform(stroke,1,0,0)==[[0,0],[1,1],[1,1],[1,1]]
        assert deform(stroke,1,2,.5)[:2]==stroke[:2]
        assert stroke==[[0,0],[1,1],[2,2],[1,3]]
        print("Loop deformation tests passed: identity, collapse, fixed prefix/anchor, immutable input")
        return
    if not a.output or a.output.exists():
        p.error("Choose a new --output")
    template_path=ROOT/"crates/core/assets/kana_templates.json"
    document=json.loads(template_path.read_text())
    templates={t["label"]:t for t in document["templates"]}
    stroke=templates["る"]["strokes"][0]
    # Manually inspected point where the long outside curve enters the terminal loop.
    assert len(stroke)==88 and stroke[53]==[152,163]
    rows=[]
    for sx in [0,.2,.35,.5,.75,1,1.25]:
        for aspect in [.5,1,1.5]:
            sy=sx*aspect
            for degrees in [-9,0,9]:
                angle=math.radians(degrees)
                changed=deform(stroke,53,sx,sy)
                ink=[[dict(x=100+(x-100)*math.cos(angle)-(y-120)*math.sin(angle),
                           y=120+(x-100)*math.sin(angle)+(y-120)*math.cos(angle)) for x,y in changed]]
                rows.append(dict(id=f"terminal-loop:{sx}:{aspect}:{degrees}",strokes=ink,script="hiragana",
                    loop_scale_x=sx,loop_scale_y=sy,degrees=degrees,source_parent="る",
                    validity="unknown",kind="omission_probe" if sx==0 else "shape_hypothesis"))
    for degrees in [-9,0,9]:
        angle=math.radians(degrees)
        ink=[[dict(x=100+(x-100)*math.cos(angle)-(y-120)*math.sin(angle),
                   y=120+(x-100)*math.sin(angle)+(y-120)*math.cos(angle)) for x,y in templates["ろ"]["strokes"][0]]]
        rows.append(dict(id=f"ro-control:{degrees}",strokes=ink,script="hiragana",source_parent="ろ",degrees=degrees,
            validity="reference_control",kind="control"))
    results=[]
    for start in range(0,len(rows),64):
        batch=rows[start:start+64]
        predictions=json.loads(subprocess.check_output([str(ROOT/"target/release/kana-vision")],
            input=json.dumps([dict(strokes=r["strokes"],script=r["script"]) for r in batch]).encode()))
        results.extend(dict(row,vision=prediction["vision"],geometry=prediction["geometry"]) for row,prediction in zip(batch,predictions,strict=True))
    a.output.mkdir(parents=True)
    (a.output/"predictions.jsonl").write_text("".join(json.dumps(r,ensure_ascii=False)+"\n" for r in results))
    summary=dict(samples=len(rows),template_sha256=hashlib.sha256(template_path.read_bytes()).hexdigest(),
        model_sha256=hashlib.sha256((ROOT/"crates/core/assets/kana_vision.bin").read_bytes()).hexdigest(),
        model_fingerprints=sorted({r["vision"]["model_fingerprint"] for r in results}),anchor_index=53,anchor=stroke[53],
        limitation="Manually anchored synthetic shape/omission hypotheses. No validity labels for training. No independent handwriting accuracy.")
    (a.output/"summary.json").write_text(json.dumps(summary,indent=2)+"\n")
    (a.output/"source.py").write_bytes(Path(__file__).read_bytes())
    for sx in [0,.2,.35,.5,.75,1,1.25]:
        values=[r for r in results if r.get("loop_scale_x")==sx]
        print(sx,[(r['loop_scale_y'],r['degrees'],r['vision']['candidates'][0]['character']) for r in values])


if __name__=="__main__":
    main()
