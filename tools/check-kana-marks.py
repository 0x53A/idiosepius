#!/usr/bin/env python3
"""Evaluate a label-blind marked-kana adapter on procedural and public references."""
import argparse
from collections import defaultdict
import hashlib
import json
from pathlib import Path
import subprocess
import unicodedata

import kana_marks
from kana_geometry import animcjk_screen_strokes

ROOT = Path(__file__).resolve().parent.parent


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument("--model", type=Path, required=True)
    p.add_argument("--controls", type=Path, required=True)
    p.add_argument("--challenges", type=Path, required=True)
    p.add_argument("--animcjk", type=Path, required=True)
    p.add_argument("--output", type=Path, required=True)
    p.add_argument("--clip-animcjk", action="store_true")
    a = p.parse_args()
    if a.output.exists():
        p.error("Never overwrite an experiment")
    controls = [json.loads(line) for line in (a.controls / "samples.jsonl").read_text().splitlines()]
    rows = [dict(r, group=r["source"]+":unmarked_control", actual=r["parent_character"]) for r in controls if r["kind"]=="valid_control"]
    challenges = [json.loads(line) for line in (a.challenges / "samples.jsonl").read_text().splitlines()]
    rows += [dict(r, group=r["source"]+":procedural_marked",actual=r["parent_character"]) for r in challenges if r["kind"]=="outside_basic_inventory"]
    labels = {t["label"] for t in json.loads((ROOT/"crates/core/assets/kana_templates.json").read_text())["templates"]}
    for record in map(json.loads,a.animcjk.read_text().splitlines()):
        character = record["character"]
        decomposition = unicodedata.normalize("NFD",character)
        if len(decomposition) != 2 or decomposition[-1] not in "\u3099\u309a" or decomposition[0] not in labels:
            continue
        if a.clip_animcjk:
            import kana_animcjk_clip
            strokes = kana_animcjk_clip.strokes(record)
        else:
            strokes = [[dict(x=x,y=y) for x,y in s] for s in animcjk_screen_strokes(record["medians"])]
        rows.append(dict(id=f"animcjk:{ord(character):04x}",group="animcjk:public_marked",actual=character,
                         script="hiragana" if ord(decomposition[0])<0x30a0 else "katakana",strokes=strokes))
    jobs, choices = [], []
    for row in rows:
        candidates = kana_marks.candidates(row["strokes"])
        group = []
        for candidate in candidates:
            body = [s for i,s in enumerate(row["strokes"]) if i not in candidate["stroke_indices"]]
            group.append((candidate,len(jobs)))
            jobs.append(dict(script=row["script"],strokes=body))
        choices.append(group)
    predictions = []
    for start in range(0,len(jobs),64):
        predictions.extend(json.loads(subprocess.check_output([str(ROOT/"target/release/kana-vision"),"--model",str(a.model)],input=json.dumps(jobs[start:start+64]).encode())))
    result = []
    for row,options in zip(rows,choices,strict=True):
        composed = []
        tentative = []
        for mark,index in options:
            candidates = predictions[index]["vision"]["candidates"]
            if not candidates:
                continue
            base = candidates[0]
            character = unicodedata.normalize("NFC",base["character"]+("\u3099" if mark["mark"]=="dakuten" else "\u309a"))
            if len(character)==1:
                reading = dict(character=character,base=base["character"],base_score=base["score"],**mark)
                tentative.append(reading)
                if base["score"]>=.90:
                    composed.append(reading)
        unique = {c["character"] for c in composed}
        tentative_unique = {c["character"] for c in tentative}
        result.append(dict(id=row["id"],group=row["group"],actual=row["actual"],mark_candidates=len(options),
            tentative=tentative,tentative_selected=next(iter(tentative_unique)) if len(tentative_unique)==1 else None,
            composed=composed,selected=next(iter(unique)) if len(unique)==1 else None))
    groups=defaultdict(list)
    for row in result:
        groups[row["group"]].append(row)
    summary={k:dict(samples=len(v),mark_detected=sum(r["mark_candidates"]>0 for r in v),
        tentative_correct=sum(r["tentative_selected"]==r["actual"] for r in v),
        tentative_wrong=sum(r["tentative_selected"] is not None and r["tentative_selected"]!=r["actual"] for r in v),
        composed=sum(r["selected"] is not None for r in v),correct_composed=sum(r["selected"]==r["actual"] for r in v),
        wrong_composed=sum(r["selected"] is not None and r["selected"]!=r["actual"] for r in v)) for k,v in groups.items()}
    a.output.mkdir(parents=True)
    (a.output/"predictions.json").write_text(json.dumps(result,ensure_ascii=False,indent=2)+"\n")
    (a.output/"summary.json").write_text(json.dumps(dict(groups=summary,detector=kana_marks.VERSION,
        clipped_animation_medians=a.clip_animcjk,
        clipper_sha256=hashlib.sha256(Path(kana_animcjk_clip.__file__).read_bytes()).hexdigest() if a.clip_animcjk else None,
        model_sha256=hashlib.sha256(a.model.read_bytes()).hexdigest(),source_sha256=hashlib.sha256(Path(kana_marks.__file__).read_bytes()).hexdigest(),
        animcjk_sha256=hashlib.sha256(a.animcjk.read_bytes()).hexdigest(),
        source_url="https://raw.githubusercontent.com/parsimonhi/animCJK/ec5e17cca76c87587790bcbce5ea0b4d4fb753d6/graphicsJaKana.txt",
        note="Experimental component recognition. Public references are not independent writers. No model deployed; base softmax is not a probability that the composed character is valid."),indent=2)+"\n")
    (a.output/"detector.py").write_bytes(Path(kana_marks.__file__).read_bytes())
    (a.output/"evaluator.py").write_bytes(Path(__file__).read_bytes())
    if a.clip_animcjk:
        (a.output/"clipper.py").write_bytes(Path(kana_animcjk_clip.__file__).read_bytes())
    print(json.dumps(summary,indent=2))


if __name__ == "__main__":
    main()
