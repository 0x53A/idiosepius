#!/usr/bin/env python3
"""Freeze stroke controls and beginner-error hypotheses across all 92 basic kana.

Corruptions have UNKNOWN validity: deleting a stroke can produce a different
valid kana. Their parent label is provenance, never an answer supplied to the
recognizer. KanjiVG is reserved for evaluation; embedded-source variants are
development data, not independent handwriting. No study database is opened.
"""
from __future__ import annotations

import argparse
import copy
import csv
import hashlib
import json
import math
import random
from collections import Counter
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
# Inventory comes from the actual embedded library; do not duplicate its labels.
VERSION = "kana-beginner-strokes-v1"


def digest(data):
    return hashlib.sha256(data).hexdigest()


def encode(value):
    return (json.dumps(value, ensure_ascii=False, sort_keys=True, separators=(",", ":")) + "\n").encode()


def xy(strokes):
    return [[(float(p["x"]), float(p["y"])) if isinstance(p, dict) else tuple(map(float, p)) for p in s] for s in strokes]


def normalize(strokes):
    points = [p for s in strokes for p in s]
    x0, y0 = (min(p[i] for p in points) for i in (0, 1))
    x1, y1 = (max(p[i] for p in points) for i in (0, 1))
    side = max(x1 - x0, y1 - y0, 1e-9)
    return [[((x - (x0 + x1) / 2) * 80 / side + 50,
              (y - (y0 + y1) / 2) * 80 / side + 50) for x, y in s] for s in strokes]


def resample(stroke, count=32):
    lengths = [0.0]
    for a, b in zip(stroke, stroke[1:]):
        lengths.append(lengths[-1] + math.dist(a, b))
    if lengths[-1] == 0:
        return [stroke[0]] * count
    out, j = [], 1
    for i in range(count):
        distance = lengths[-1] * i / (count - 1)
        while j < len(stroke) - 1 and lengths[j] < distance:
            j += 1
        t = (distance - lengths[j - 1]) / max(lengths[j] - lengths[j - 1], 1e-12)
        out.append(tuple(stroke[j - 1][k] * (1 - t) + stroke[j][k] * t for k in (0, 1)))
    return out


def variants(strokes, seed):
    """Mild shape controls plus exact order/direction/pen-lift invariances."""
    yield "canonical", strokes
    yield "reverse_order", list(reversed(strokes))
    yield "reverse_direction", [list(reversed(s)) for s in strokes]
    yield "split_strokes", [part for s in strokes for part in (s[:len(s)//2+1], s[len(s)//2:])]
    yield "resampled", [resample(s, 48) for s in strokes]
    yield "retraced", strokes + copy.deepcopy(strokes)
    rng = random.Random(seed)
    for i in range(6):
        angle = math.radians(rng.uniform(-6, 6))
        sx, sy, shear = rng.uniform(.94, 1.06), rng.uniform(.94, 1.06), rng.uniform(-.04, .04)
        phase = rng.uniform(0, math.tau)
        changed = []
        for s in strokes:
            points = []
            for j, (x, y) in enumerate(resample(s)):
                xx, yy = (x - 50) * sx + shear * (y - 50), (y - 50) * sy
                wobble = .6 * math.sin(phase + j / 31 * math.tau)
                points.append((50 + xx * math.cos(angle) - yy * math.sin(angle) + wobble,
                               50 + xx * math.sin(angle) + yy * math.cos(angle) + wobble))
            changed.append(points)
        yield f"mild_shape_{i}", changed


def corruptions(strokes):
    for index, stroke in enumerate(strokes):
        if len(strokes) > 1:
            yield f"omit_{index}", strokes[:index] + strokes[index+1:], index
        for axis in [0, 1]:
            center = (min(p[axis] for p in stroke) + max(p[axis] for p in stroke)) / 2
            reflected = [tuple(2 * center - v if k == axis else v for k, v in enumerate(p)) for p in stroke]
            if max(math.dist(a, b) for a, b in zip(reflected, stroke)) > 2:
                changed = copy.deepcopy(strokes)
                changed[index] = reflected
                yield f"mirror_{'xy'[axis]}_{index}", changed, index
        for end in ["head", "tail"]:
            changed = copy.deepcopy(strokes)
            sampled = resample(stroke, 41)
            changed[index] = sampled[12:] if end == "head" else sampled[:-12]
            yield f"truncate_{end}_{index}", changed, index
        changed = copy.deepcopy(strokes)
        changed[index] = [(x + 18, y - 12) for x, y in stroke]
        yield f"displace_{index}", changed, index


def invalid_shapes():
    yield "empty", []
    for count in [5, 8, 11]:
        positions = [10 + i * 80 / (count - 1) for i in range(count)]
        yield f"grid_{count}", [[(x, 10), (x, 90)] for x in positions] + [[(10, y), (90, y)] for y in positions]
        yield f"concentric_{count}", [[(50 + r * math.cos(i * math.tau / 80), 50 + r * math.sin(i * math.tau / 80)) for i in range(81)] for r in [40 * (j + 1) / count for j in range(count)]]
    rng = random.Random(20260912)
    for i in range(6):
        yield f"dense_scribble_{i}", [[(rng.uniform(10, 90), rng.uniform(10, 90)) for _ in range(100)]]


def document(source, character, script, operation, strokes, kind, index=None):
    return dict(id=f"{source}:{ord(character):04x}:{operation}" if character else f"invalid:{script}:{operation}",
        source=source, partition="evaluation" if source == "kanjivg" else "development",
        parent_character=character, expected=character if kind == "valid_control" else None,
        script=script, operation=operation, affected_stroke=index, kind=kind,
        validity="valid_control" if kind == "valid_control" else "invalid" if kind == "non_kana" else "unknown",
        strokes=[[dict(x=round(x, 6), y=round(y, 6)) for x, y in s] for s in strokes])


def build(templates, external, seed):
    rows = []
    for source, source_rows in [("embedded", templates), ("kanjivg", external)]:
        for item in source_rows:
            character, script = item["label"], item["script"]
            strokes = normalize(xy(item["strokes"]))
            variant_seed = int(digest(f"{VERSION}:{seed}:{source}:{character}".encode())[:16], 16)
            for operation, ink in variants(strokes, variant_seed):
                rows.append(document(source, character, script, operation, ink, "valid_control"))
            for operation, ink, index in corruptions(strokes):
                rows.append(document(source, character, script, operation, ink, "corruption_probe", index))
    for script in ["hiragana", "katakana"]:
        for operation, strokes in invalid_shapes():
            rows.append(document("procedural", None, script, operation, strokes, "non_kana"))
    return rows


def self_test():
    s = [[(10, 10), (30, 50), (60, 20)], [(20, 60), (70, 80)]]
    assert resample([(0, 0), (10, 0)], 3) == [(0., 0.), (5., 0.), (10., 0.)]
    before = encode(s)
    assert len(list(variants(s, 5))) == 12
    assert encode(list(variants(s, 5))) == encode(list(variants(s, 5)))
    assert encode(list(variants(s, 5))) != encode(list(variants(s, 6)))
    list(corruptions(s))
    assert encode(s) == before
    for operation, ink, index in corruptions(s):
        assert ink and all(len(stroke) >= 2 for stroke in ink)
        row = document("embedded", "う", "hiragana", operation, ink, "corruption_probe", index)
        assert row["expected"] is None and row["validity"] == "unknown"
    assert dict(variants(s, 5))["reverse_direction"] == [list(reversed(t)) for t in s]
    print("Beginner corpus tests passed: deterministic variation, resampling, immutable parents, unknown corruption validity")


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument("--output", type=Path)
    p.add_argument("--seed", type=int, default=20260912)
    p.add_argument("--external", type=Path, default=ROOT / "kana-artifacts/kana-diagnostics/kana-cross-source-v1/samples")
    p.add_argument("--self-test", action="store_true")
    a = p.parse_args()
    if a.self_test:
        self_test()
        return
    if a.output is None or not a.output.resolve().is_relative_to((ROOT / "kana-artifacts").resolve()):
        p.error("Choose a new output under kana-artifacts/")
    template_path = ROOT / "crates/core/assets/kana_templates.json"
    template_document = json.loads(template_path.read_text())
    templates = template_document["templates"]
    hypotheses_path = ROOT / "tools/fixtures/kana-beginner/hypotheses.tsv"
    hypotheses = list(csv.DictReader(hypotheses_path.open(), delimiter="\t"))
    assert len(templates) == len(hypotheses) == 92
    assert {t["label"] for t in templates} == {h["character"] for h in hypotheses}
    external, source_hashes = [], {}
    for script in ["hiragana", "katakana"]:
        for path in sorted((a.external / f"kanjivg-{script}").glob("*.json")):
            original = json.loads(path.read_text())
            assert original["external_source"]["commit"] == "61e39cfc29724132a6f8823b166296932985a0ff"
            source_hashes[str(path.relative_to(a.external))] = digest(path.read_bytes())
            external.append(dict(label=original["expected"], script=script, strokes=original["strokes"]))
    assert len(external) == 92 and {t["label"] for t in external} == {t["label"] for t in templates}
    rows = build(templates, external, a.seed)
    assert len({r["id"] for r in rows}) == len(rows)
    a.output.mkdir(parents=True, exist_ok=False)
    payload = b"".join(encode(row) for row in rows)
    (a.output / "samples.jsonl").write_bytes(payload)
    (a.output / "generator.py").write_bytes(Path(__file__).read_bytes())
    (a.output / "hypotheses.tsv").write_bytes(hypotheses_path.read_bytes())
    manifest = dict(generator=VERSION, seed=a.seed, samples=len(rows),
        counts=dict(Counter(f"{r['source']}:{r['kind']}" for r in rows)),
        samples_sha256=digest(payload), generator_sha256=digest(Path(__file__).read_bytes()),
        template_sha256=digest(template_path.read_bytes()), source_hashes=source_hashes,
        template_provenance={k: template_document[k] for k in ("source", "source_commit", "hiragana_source", "hiragana_source_commit", "coordinate_origin")},
        external_provenance=dict(name="KanjiVG", commit="61e39cfc29724132a6f8823b166296932985a0ff", license="CC-BY-SA-3.0", url="https://github.com/KanjiVG/kanjivg"),
        hypotheses_sha256=digest(hypotheses_path.read_bytes()),
        limitations=["Synthetic reference-derived controls are not human handwriting accuracy.",
            "Corruption parent identity is provenance, not proof of intended identity or invalidity.",
            "Known-script scope cannot distinguish visually identical characters in other scripts.",
            "KanjiVG is evaluation-only; do not train on these derivatives.",
            "Per-kana error descriptions are authored hypotheses, not measured beginner error frequencies."])
    (a.output / "manifest.json").write_bytes(encode(manifest))
    print(json.dumps({k: manifest[k] for k in ["generator", "samples", "counts", "samples_sha256"]}, indent=2))


if __name__ == "__main__":
    main()
