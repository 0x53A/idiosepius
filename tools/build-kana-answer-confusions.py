#!/usr/bin/env python3
"""Build valid-but-wrong-answer and voiced-mark challenge cases, never invalid labels.

The 92-class recognizer cannot emit voiced kana. These generated marked forms
test whether a future grader would silently accept the unmarked base instead.
Marks are hand-designed procedural strokes, not collected pen trajectories.
"""
import argparse
import copy
import csv
import hashlib
import json
import math
from pathlib import Path
import unicodedata

ROOT = Path(__file__).resolve().parent.parent


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument("corpus", type=Path)
    p.add_argument("--output", type=Path, required=True)
    a = p.parse_args()
    if a.output.exists():
        p.error("Never overwrite a corpus")
    raw = (a.corpus / "samples.jsonl").read_bytes()
    parent = json.loads((a.corpus / "manifest.json").read_text())
    assert hashlib.sha256(raw).hexdigest() == parent["samples_sha256"]
    originals = [r for r in map(json.loads, raw.splitlines()) if r["operation"] == "canonical"]
    references = {(r["source"], r["parent_character"]): r for r in originals}
    hypotheses_path = ROOT / "tools/fixtures/kana-beginner/hypotheses.tsv"
    hypotheses = list(csv.DictReader(hypotheses_path.open(), delimiter="\t"))
    rows = []
    for original in originals:
        base, source = original["parent_character"], original["source"]
        partners = next(h["confusables"] for h in hypotheses if h["character"] == base)
        for partner in partners:
            other = references.get((source, partner))
            if other is None or other["script"] != original["script"]:
                continue
            row = copy.deepcopy(other)
            row.update(id=f"{source}:{ord(base):04x}:answer-{ord(partner):04x}", requested_answer=base,
                       kind="valid_different_answer", validity="valid", expected=partner,
                       operation="different_valid_kana", partition="evaluation")
            rows.append(row)
        for mark, name in [("\u3099", "dakuten"), ("\u309a", "handakuten")]:
            composed = unicodedata.normalize("NFC", base + mark)
            if len(composed) != 1:
                continue
            for variant in range(3):
                row = copy.deepcopy(original)
                dx, dy = variant * 2, -variant
                if name == "dakuten":
                    marks = [[dict(x=82+dx, y=2+dy), dict(x=87+dx, y=8+dy)],
                             [dict(x=91+dx, y=0+dy), dict(x=96+dx, y=6+dy)]]
                else:
                    marks = [[dict(x=91+dx+4*math.cos(i*math.tau/24), y=5+dy+4*math.sin(i*math.tau/24)) for i in range(25)]]
                row["strokes"] += marks
                row.update(id=f"{source}:{ord(base):04x}:{name}-{variant}", requested_answer=base,
                           parent_character=composed, expected=composed, kind="outside_basic_inventory",
                           validity="constructed_marked_kana", operation=name, partition="evaluation")
                rows.append(row)
    assert len({r["id"] for r in rows}) == len(rows)
    assert all(r["parent_character"] != r["requested_answer"] for r in rows)
    a.output.mkdir(parents=True)
    payload = b"".join((json.dumps(r, ensure_ascii=False, sort_keys=True) + "\n").encode() for r in rows)
    (a.output / "samples.jsonl").write_bytes(payload)
    (a.output / "generator.py").write_bytes(Path(__file__).read_bytes())
    manifest = dict(generator="kana-answer-confusions-v1", samples=len(rows), samples_sha256=hashlib.sha256(payload).hexdigest(),
        parent_corpus_sha256=parent["samples_sha256"], generator_sha256=hashlib.sha256(Path(__file__).read_bytes()).hexdigest(),
        hypotheses_sha256=hashlib.sha256(hypotheses_path.read_bytes()).hexdigest(),
        source_provenance=parent,
        note="Requested answer is never passed to recognition. Voiced-mark placement is procedural, not teacher-rated handwriting. These are valid-alternative or outside-inventory challenges, not non-kana negatives.")
    (a.output / "manifest.json").write_text(json.dumps(manifest, ensure_ascii=False, indent=2) + "\n")
    print(f"Wrote {len(rows)} valid-alternative/marked challenges")


if __name__ == "__main__":
    main()
