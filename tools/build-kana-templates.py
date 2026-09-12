#!/usr/bin/env python3
"""Extract deterministic basic-kana trajectory templates from AnimCJK data."""

from __future__ import annotations

import argparse
import json
from pathlib import Path
import xml.etree.ElementTree as ET
from kana_geometry import animcjk_screen_strokes


HIRAGANA = "あいうえおかきくけこさしすせそたちつてとなにぬねのはひふへほまみむめもやゆよらりるれろわをん"
KATAKANA = "アイウエオカキクケコサシスセソタチツテトナニヌネノハヒフヘホマミムメモヤユヨラリルレロワヲン"
HIRAGANA_FILES = (
    "a i u e o ka ki ku ke ko sa si su se so ta chi tsu te to "
    "na ni nu ne no ha hi fu he ho ma mi mu me mo ya yu yo "
    "ra ri ru re ro wa wo n"
).split()


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("source", type=Path, help="AnimCJK graphicsJaKana.txt")
    parser.add_argument("output", type=Path)
    parser.add_argument("--source-commit", required=True)
    parser.add_argument("--hiragana-dir", type=Path, required=True)
    parser.add_argument("--hiragana-commit", required=True)
    parser.add_argument("--variants-from", type=Path,
                        help="retain previously generated hiragana variants, verifying unchanged canonical hiragana")
    args = parser.parse_args()

    rows = {}
    with args.source.open(encoding="utf-8") as source:
        for line in source:
            row = json.loads(line)
            rows[row["character"]] = row

    missing = [character for character in HIRAGANA + KATAKANA if character not in rows]
    if missing:
        parser.error(f"source lacks templates for: {''.join(missing)}")

    templates = []
    for character, name in zip(HIRAGANA, HIRAGANA_FILES, strict=True):
        root = ET.parse(args.hiragana_dir / f"{name}.xml").getroot()
        strokes = [
            [[int(point.attrib["x"]), int(point.attrib["y"])] for point in stroke]
            for stroke in root.findall("stroke")
        ]
        templates.append(
            {"label": character, "script": "hiragana", "strokes": strokes}
        )
    for character in KATAKANA:
        templates.append(
            {
                "label": character,
                "script": "katakana",
                "strokes": animcjk_screen_strokes(rows[character]["medians"]),
            }
        )

    result = {
        "source": "AnimCJK graphicsJaKana.txt",
        "source_commit": args.source_commit,
        "hiragana_source": "ctegaki-lib hiragana XML",
        "hiragana_source_commit": args.hiragana_commit,
        "coordinate_origin": "top_left",
        "animcjk_coordinate_conversion": "x,900-y",
        "templates": templates,
    }
    if args.variants_from:
        previous = json.loads(args.variants_from.read_text(encoding="utf-8"))
        if previous["hiragana_source_commit"] != args.hiragana_commit:
            parser.error("retained variants must have the same canonical hiragana source")
        old = {t["label"]: t for t in previous["templates"]}
        for template in templates:
            if template["script"] == "hiragana":
                prior = old[template["label"]]
                if prior["strokes"] != template["strokes"]:
                    parser.error("retained variants require unchanged canonical hiragana")
                if "variants" in prior:
                    template["variants"] = prior["variants"]
        result.update({key: value for key, value in previous.items()
                       if key.startswith("hiragana_variant_")})
    args.output.write_text(
        json.dumps(result, ensure_ascii=False, separators=(",", ":")) + "\n",
        encoding="utf-8",
    )


if __name__ == "__main__":
    main()
