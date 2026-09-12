#!/usr/bin/env python3
"""Expand embedded-only shape training; preserve frozen ETL train/validation arrays."""
import argparse
import importlib.util
import json
import math
from pathlib import Path
import random
import subprocess

import numpy as np

ROOT=Path(__file__).resolve().parent.parent
spec=importlib.util.spec_from_file_location("builder",Path(__file__).with_name("build-kana-beginner-corpus.py"))
builder=importlib.util.module_from_spec(spec)
spec.loader.exec_module(builder)


def main():
    p=argparse.ArgumentParser(description=__doc__)
    p.add_argument("--base",type=Path,required=True)
    p.add_argument("--corpus",type=Path,required=True)
    p.add_argument("--output",type=Path,required=True)
    p.add_argument("--adapt-clipped-hiragana",type=Path,
                   help="Explicitly move this previously evaluated source into development training")
    a=p.parse_args()
    if a.output.exists():
        p.error("Never overwrite a preparation")
    raw=(a.corpus/"samples.jsonl").read_bytes()
    manifest=json.loads((a.corpus/"manifest.json").read_text())
    assert builder.digest(raw)==manifest["samples_sha256"]
    canonical=[r for r in map(json.loads,raw.splitlines()) if r["source"]=="embedded" and r["operation"]=="canonical"]
    assert len(canonical)==92
    extra={}
    extra_hash=None
    if a.adapt_clipped_hiragana:
        extra_raw=(a.adapt_clipped_hiragana/"samples.jsonl").read_bytes()
        extra_manifest=json.loads((a.adapt_clipped_hiragana/"manifest.json").read_text())
        extra_hash=builder.digest(extra_raw)
        assert extra_hash==extra_manifest["samples_sha256"]
        extra={r["parent_character"]:r for r in map(json.loads,extra_raw.splitlines())
               if r["source"]=="animcjk_clipped_hiragana" and r["operation"]=="canonical" and r["kind"]=="valid_control"}
        assert len(extra)==46 and all(r["script"]=="hiragana" for r in extra.values())
    rows=[]
    for original in canonical:
        label=original["parent_character"]
        for partition,seed,count in [("train",20260914,64),("validation",20260915,16)]:
            rng=random.Random(int(builder.digest(f"shape-v1:{seed}:{label}".encode())[:16],16))
            for variant in range(count):
                reference=extra[label] if label in extra and variant>=count//2 else original
                angle=math.radians(rng.uniform(-12,12))
                sx,sy,shear=rng.uniform(.85,1.15),rng.uniform(.85,1.15),rng.uniform(-.10,.10)
                phase,amplitude=rng.uniform(0,math.tau),rng.uniform(0,1.2)
                points_per_stroke=rng.choice([32,48,96])
                strokes=[]
                for stroke in builder.xy(reference["strokes"]):
                    changed=[]
                    for j,(x,y) in enumerate(builder.resample(stroke,points_per_stroke)):
                        xx,yy=(x-50)*sx+shear*(y-50),(y-50)*sy
                        wobble=amplitude*math.sin(phase+j/(points_per_stroke-1)*math.tau)
                        changed.append((50+xx*math.cos(angle)-yy*math.sin(angle)+wobble,
                                        50+xx*math.sin(angle)+yy*math.cos(angle)+wobble))
                    strokes.append(changed)
                row=builder.document(reference["source"],label,original["script"],f"shape_v1_{partition}_{variant}",strokes,"valid_control")
                row.update(partition=partition,parameters=dict(angle=angle,sx=sx,sy=sy,shear=shear,
                    phase=phase,amplitude=amplitude,points_per_stroke=points_per_stroke))
                rows.append(row)
    prep=json.loads((a.base/"preparation.json").read_text())
    assert builder.digest((a.base/"trial.npz").read_bytes())==prep["trial_sha256"]
    assert builder.digest((a.base/"records.json").read_bytes())==prep["records_sha256"]
    with np.load(a.base/"trial.npz") as data:
        arrays={k:data[k] for k in data.files}
    train=[r for r in rows if r["partition"]=="train"]
    images=[]
    for start in range(0,len(train),128):
        raster=subprocess.check_output([str(ROOT/"target/release/kana-rasterize"),"--ink-batch","--side","64"],
            input=json.dumps([r["strokes"] for r in train[start:start+128]]).encode())
        images.append(np.frombuffer(raster,dtype="<f4").copy().reshape(-1,1,64,64))
    labels=[r["parent_character"] for r in canonical]
    assert labels==[t["label"] for t in json.loads((ROOT/"crates/core/assets/kana_templates.json").read_text())["templates"]]
    arrays["synthetic_images"]=np.concatenate([arrays["synthetic_images"],*images])
    arrays["synthetic_labels"]=np.concatenate([arrays["synthetic_labels"],np.array([labels.index(r["parent_character"]) for r in train])])
    a.output.mkdir(parents=True)
    np.savez_compressed(a.output/"trial.npz",**arrays)
    (a.output/"records.json").write_bytes((a.base/"records.json").read_bytes())
    payload=b"".join(builder.encode(r) for r in rows)
    (a.output/"samples.jsonl").write_bytes(payload)
    (a.output/"manifest.json").write_text(json.dumps(dict(samples=len(rows),samples_sha256=builder.digest(payload),
        adapted_source_corpus_sha256=extra_hash,
        note="Reference-derived transform split only; not independent writers. Validation transforms excluded from training. Adapted source is development data when present."),indent=2)+"\n")
    prep.update(version="kana-shape-trial-preparation-v1",base_trial_sha256=prep["trial_sha256"],
        trial_sha256=builder.digest((a.output/"trial.npz").read_bytes()),
        synthetic_ids=prep["synthetic_ids"]+[r["id"] for r in train],synthetic_count=len(arrays["synthetic_labels"]),
        shape_samples_sha256=builder.digest(payload),script_sha256=builder.digest(Path(__file__).read_bytes()),
        note="Frozen ETL arrays unchanged. Added 64 embedded-only shapes per class. Sixteen other seeded transforms per class are excluded. No AnimCJK hiragana, KanjiVG, corruption or test records added.")
    if extra:
        prep.update(version="kana-style-adaptation-preparation-v1",adapted_source_corpus_sha256=extra_hash,
            note="Frozen ETL and original controls unchanged. Half the 64 new hiragana shapes per class use clipped AnimCJK canonical references, explicitly moved into development. KanjiVG, corruptions and ETL test remain excluded. Adapted-source results are no longer source generalization evidence.")
    (a.output/"preparation.json").write_text(json.dumps(prep,indent=2)+"\n")
    (a.output/"prepare.py").write_bytes(Path(__file__).read_bytes())
    (a.output/"builder.py").write_bytes(Path(builder.__file__).read_bytes())
    print(json.dumps({k:prep[k] for k in ["synthetic_count","trial_sha256","shape_samples_sha256"]}))


if __name__=="__main__":
    main()
