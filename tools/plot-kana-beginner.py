#!/usr/bin/env python3
"""Make per-kana SVG audit sheets with raw strokes, mutations, and predictions."""
import argparse
import html
import json
from pathlib import Path


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument("corpus", type=Path)
    p.add_argument("audit", type=Path)
    p.add_argument("--output", type=Path, required=True)
    a = p.parse_args()
    a.output.mkdir(parents=True, exist_ok=False)
    rows = [json.loads(line) for line in (a.corpus / "samples.jsonl").read_text().splitlines()]
    predictions = {r["id"]: r for r in map(json.loads, (a.audit / "predictions.jsonl").read_text().splitlines())}
    labels = sorted({r["parent_character"] for r in rows if r["parent_character"]})
    links = []
    for label in labels:
        selected = [r for r in rows if r["parent_character"] == label and
                    (r["operation"] == "canonical" or r["kind"] == "corruption_probe")]
        width, height = 900, ((len(selected) + 4) // 5) * 185 + 90
        parts = [f'<svg xmlns="http://www.w3.org/2000/svg" width="{width}" height="{height}" viewBox="0 0 {width} {height}">',
            '<rect width="100%" height="100%" fill="#060A0D"/>',
            f'<text x="16" y="29" fill="#D8E6EA" font-family="monospace" font-size="20">{label} — structural hypotheses</text>',
            '<text x="16" y="52" fill="#74919A" font-family="monospace" font-size="12">Mutations are not certified invalid. Parent labels are hidden from inference.</text>']
        for i, row in enumerate(selected):
            x, y = i % 5 * 180 + 10, i // 5 * 185 + 70
            parts.append(f'<g transform="translate({x},{y})"><rect width="165" height="172" fill="#0B1216" stroke="#1E3037"/>')
            parts.append('<g transform="translate(23,12) scale(1.15)">')
            for stroke in row["strokes"]:
                points = " ".join(f'{p["x"]},{p["y"]}' for p in stroke)
                parts.append(f'<polyline points="{points}" fill="none" stroke="#D8E6EA" stroke-width="1.6"/>')
            parts.append('</g>')
            prediction = predictions[row["id"]]
            candidate = prediction["vision"]["candidates"][0]
            text = [row["source"] + ' ' + row["operation"],
                    f'CNN {candidate["character"]} {candidate["score"]:.1%}',
                    'geometry ' + ('accepted' if prediction["geometry"]["state"] == "recognized" else 'rejected')]
            for j, line in enumerate(text):
                parts.append(f'<text x="5" y="{136 + j * 14}" fill="#74919A" font-family="monospace" font-size="10">{html.escape(line)}</text>')
            parts.append('</g>')
        parts += ['<metadata>Research diagnostic. Embedded references: ctegaki/AnimCJK; KanjiVG CC-BY-SA-3.0. Exact source commits and corpus hashes are in the adjacent corpus manifest. No teacher-certified labels.</metadata>', '</svg>']
        name = f'u{ord(label):04x}.svg'
        (a.output / name).write_text("\n".join(parts))
        links.append(f'<li><a href="{name}">{label}</a></li>')
    (a.output / "index.html").write_text('<!doctype html><meta charset="utf-8"><title>Kana mutation audit</title><h1>Kana mutation audit</h1><p>Raw reference-derived ink and independent predictions. A mutation is not automatically invalid.</p><ul>' + ''.join(links) + '</ul>')
    print(f"Wrote {len(labels)} per-character SVG audit sheets to {a.output}")


if __name__ == "__main__":
    main()
