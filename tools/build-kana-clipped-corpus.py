#!/usr/bin/env python3
"""Append clipped AnimCJK controls/probes to a frozen beginner corpus.

AnimCJK katakana overlaps embedded references. Hiragana is an additional source
for the repair bank, but has been examined in earlier research diagnostics.
"""
import argparse
from collections import Counter
import hashlib
import importlib.util
import json
from pathlib import Path

import kana_animcjk_clip

ROOT = Path(__file__).resolve().parent.parent
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
    raw = (a.base / "samples.jsonl").read_bytes()
    original_manifest = json.loads((a.base / "manifest.json").read_text())
    assert builder.digest(raw) == original_manifest["samples_sha256"]
    original_rows = [json.loads(line) for line in raw.splitlines()]
    labels = {r["parent_character"] for r in original_rows if r["operation"] == "canonical"}
    source_bytes = a.animcjk.read_bytes()
    assert hashlib.sha256(source_bytes).hexdigest() == "50041c8fb477e08d77bcf96716003989b8b1f95a751e4d2628b7339de3b58f38"
    added, found = [], set()
    for record in map(json.loads, source_bytes.splitlines()):
        character = record["character"]
        if character not in labels:
            continue
        found.add(character)
        script = "hiragana" if ord(character) < 0x30a0 else "katakana"
        source = "animcjk_clipped_hiragana" if script == "hiragana" else "animcjk_clipped_katakana_overlap"
        strokes = builder.normalize(builder.xy(kana_animcjk_clip.strokes(record)))
        seed = int(builder.digest(f"clipped-v1:20260913:{character}".encode())[:16], 16)
        for operation, ink in builder.variants(strokes, seed):
            added.append(builder.document(source, character, script, operation, ink, "valid_control"))
        for operation, ink, index in builder.corruptions(strokes):
            added.append(builder.document(source, character, script, operation, ink, "corruption_probe", index))
    assert found == labels
    for row in added:
        row["partition"] = "evaluation"
    rows = original_rows + added
    assert len({r["id"] for r in rows}) == len(rows)
    payload = raw + b"".join(builder.encode(row) for row in added)
    a.output.mkdir(parents=True)
    (a.output / "samples.jsonl").write_bytes(payload)
    manifest = dict(samples=len(rows), samples_sha256=builder.digest(payload),
        base_sha256=builder.digest(raw), source_sha256=builder.digest(source_bytes),
        counts=dict(Counter(f"{r['source']}:{r['kind']}" for r in rows)),
        source_url="https://raw.githubusercontent.com/parsimonhi/animCJK/ec5e17cca76c87587790bcbce5ea0b4d4fb753d6/graphicsJaKana.txt",
        limitations=["Katakana overlaps embedded AnimCJK references; not independent-source evaluation.",
            "Hiragana is additional to the repair bank, but previously examined in diagnostics.",
            "Clipped fragments are geometry proxies, not human pen-down strokes. Omission can remove a fragment, not a whole authored stroke.",
            "Controls are reference-derived; corruption validity remains unknown. No human accuracy estimate."])
    for name, path in [("generator.py", Path(__file__)), ("base-generator.py", Path(builder.__file__)),
                       ("clipper.py", Path(kana_animcjk_clip.__file__)),
                       ("svg-parser.py", Path(kana_animcjk_clip.cross_source.__file__))]:
        content = path.read_bytes()
        (a.output / name).write_bytes(content)
        manifest[name+"_sha256"] = builder.digest(content)
    (a.output / "manifest.json").write_text(json.dumps(manifest, indent=2)+"\n")
    print(json.dumps(manifest, indent=2))


if __name__ == "__main__":
    main()
