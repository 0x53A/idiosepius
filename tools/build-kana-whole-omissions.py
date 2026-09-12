#!/usr/bin/env python3
"""Freeze probes removing every fragment of a source animation trajectory.

Shared identical medians form one trajectory group. These are a better proxy
for whole-stroke omissions, but still not recovered human pen-down events.
"""
import argparse
from collections import Counter
import importlib.util
import json
from pathlib import Path

import kana_animcjk_clip as clip

spec = importlib.util.spec_from_file_location("builder", Path(__file__).with_name("build-kana-beginner-corpus.py"))
builder = importlib.util.module_from_spec(spec)
spec.loader.exec_module(builder)


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument("--base", type=Path, required=True)
    p.add_argument("--animcjk", type=Path, required=True)
    p.add_argument("--output", type=Path, required=True)
    a = p.parse_args()
    if a.output.exists():
        p.error("Never overwrite a corpus")
    raw = (a.base/"samples.jsonl").read_bytes()
    manifest = json.loads((a.base/"manifest.json").read_text())
    assert builder.digest(raw)==manifest["samples_sha256"]
    # Keep precisely the frozen geometric bank, without repeating all old probes.
    rows = [r for r in map(json.loads,raw.splitlines()) if r["source"]=="embedded"
            and (r["operation"]=="canonical" or r["operation"].startswith("omit_"))]
    labels={r["parent_character"] for r in rows}
    assert len(rows)==289 and len(labels)==92
    source=a.animcjk.read_bytes()
    assert builder.digest(source)=="50041c8fb477e08d77bcf96716003989b8b1f95a751e4d2628b7339de3b58f38"
    mappings={}
    for record in map(json.loads,source.splitlines()):
        character=record["character"]
        if character not in labels:
            continue
        script="hiragana" if ord(character)<0x30a0 else "katakana"
        name="animcjk_whole_hiragana" if script=="hiragana" else "animcjk_whole_katakana_overlap"
        ink=builder.normalize(builder.xy(clip.strokes(record)))
        groups=clip.trajectory_groups(record)
        assert sorted(i for g in groups for i in g["fragment_indices"])==list(range(len(ink)))
        mappings[character]=groups
        canonical=builder.document(name,character,script,"canonical",ink,"valid_control")
        canonical["partition"]="evaluation"
        rows.append(canonical)
        for index, group in enumerate(groups):
            indices=set(group["fragment_indices"])
            if not indices:
                continue
            remaining=[stroke for j,stroke in enumerate(ink) if j not in indices]
            # Include empty one-trajectory omissions: the detector must abstain.
            row=builder.document(name,character,script,f"omit_trajectory_{index}",remaining,"corruption_probe",index)
            row.update(partition="evaluation",removed_fragment_indices=sorted(indices),
                       removed_outline_indices=group["outline_indices"],omission_unit="source_animation_trajectory")
            rows.append(row)
    assert len(mappings)==92 and len({r["id"] for r in rows})==len(rows)
    payload=b"".join(builder.encode(r) for r in rows)
    a.output.mkdir(parents=True)
    (a.output/"samples.jsonl").write_bytes(payload)
    (a.output/"trajectory-groups.json").write_text(json.dumps(mappings,ensure_ascii=False,indent=2)+"\n")
    report=dict(samples=len(rows),samples_sha256=builder.digest(payload),base_sha256=builder.digest(raw),
        source_sha256=builder.digest(source),counts=dict(Counter(f"{r['source']}:{r['kind']}" for r in rows)),
        limitations=["Trajectory groups are source animation evidence, not observed human stroke boundaries.",
            "Katakana source overlaps the embedded bank. Hiragana source was previously examined.",
            "Corruptions include empty ink and valid alternatives; parent identity is not certified intent."])
    for name,path in [("generator.py",Path(__file__)),("clipper.py",Path(clip.__file__)),
                      ("base-generator.py",Path(builder.__file__)),("svg-parser.py",Path(clip.cross_source.__file__))]:
        content=path.read_bytes()
        (a.output/name).write_bytes(content)
        report[name+"_sha256"]=builder.digest(content)
    (a.output/"manifest.json").write_text(json.dumps(report,indent=2)+"\n")
    print(json.dumps(report,indent=2))


if __name__=="__main__":
    main()
