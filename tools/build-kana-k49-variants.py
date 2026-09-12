#!/usr/bin/env python3
"""Add compact K49 training prototypes to a generated kana template asset.

Selection is intentionally independent of the recognizer: for each basic
hiragana, take a deterministic candidate pool from the official training
split and choose the bitmap nearest its pixel centroid.  Samples reserved by
``check-kana-k49.py`` for development evaluation are excluded.
"""

from __future__ import annotations

import argparse
import hashlib
import importlib.util
import json
from pathlib import Path


ROOT = Path(__file__).resolve().parent.parent


def load_k49_tool():
    path = ROOT / "tools" / "check-kana-k49.py"
    spec = importlib.util.spec_from_file_location("idiosepius_k49", path)
    if spec is None or spec.loader is None:
        raise ValueError(f"cannot load K49 tooling from {path}")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def prototype_key(index: int) -> bytes:
    return hashlib.sha256(f"idiosepius-k49-centroid-v1:{index}".encode()).digest()


def centroid_medoid(images: bytes, indices: list[int]) -> int:
    pixels = 28 * 28
    sums = [0] * pixels
    for index in indices:
        offset = index * pixels
        for coordinate, value in enumerate(images[offset : offset + pixels]):
            sums[coordinate] += value
    count = len(indices)

    def distance(index: int) -> int:
        offset = index * pixels
        return sum(
            (value * count - total) ** 2
            for value, total in zip(images[offset : offset + pixels], sums, strict=True)
        )

    return min(indices, key=lambda index: (distance(index), index))


def self_test() -> None:
    pixels = 28 * 28
    images = bytes([0] * pixels + [10] * pixels + [255] * pixels)
    assert centroid_medoid(images, [0, 1, 2]) == 1
    assert prototype_key(42) == prototype_key(42)
    assert prototype_key(42) != prototype_key(43)


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("input", type=Path, nargs="?", help="base kana_templates.json")
    parser.add_argument("output", type=Path, nargs="?")
    parser.add_argument("--images", type=Path)
    parser.add_argument("--labels", type=Path)
    parser.add_argument("--classmap", type=Path)
    parser.add_argument("--candidate-pool", type=int, default=256)
    parser.add_argument("--development-exclusion", type=int, default=24)
    parser.add_argument("--self-test", action="store_true")
    args = parser.parse_args()
    if args.self_test:
        self_test()
        print("K49 prototype selector self-test passed")
        return
    if any(path is None for path in (args.input, args.output, args.images, args.labels, args.classmap)):
        parser.error("provide input, output, --images, --labels, and --classmap, or use --self-test")
    if args.candidate_pool < 1:
        parser.error("--candidate-pool must be positive")
    if args.development_exclusion < 1:
        parser.error("--development-exclusion must be positive")

    k49 = load_k49_tool()
    expected = k49.EXPECTED["development"]
    image_hash = k49.require_hash(args.images, expected["images_sha256"], "K49 development images")
    label_hash = k49.require_hash(args.labels, expected["labels_sha256"], "K49 development labels")
    mapping = k49.classmap(args.classmap)
    records = expected["records"]
    images = k49.npy_from_npz(args.images, (records, 28, 28))
    labels = k49.npy_from_npz(args.labels, (records,))
    excluded = k49.selected_indices(labels, mapping, args.development_exclusion)
    by_character = {character: [] for character in k49.HIRAGANA}
    excluded_sets = {character: set(indices) for character, indices in excluded.items()}
    for index, label in enumerate(labels):
        character = mapping.get(label)
        if character in by_character and index not in excluded_sets[character]:
            by_character[character].append(index)

    convert = k49.vectorizer()
    selected = {}
    variants = {}
    for character, indices in by_character.items():
        indices.sort(key=prototype_key)
        pool = indices[: args.candidate_pool]
        if len(pool) < args.candidate_pool:
            raise ValueError(f"K49 has only {len(pool)} available training samples for {character}")
        index = centroid_medoid(images, pool)
        offset = index * 28 * 28
        pixels = images[offset : offset + 28 * 28]
        image = [list(pixels[row * 28 : (row + 1) * 28]) for row in range(28)]
        strokes = convert(image)
        if not strokes:
            raise ValueError(f"selected K49 prototype for {character} has no usable skeleton")
        selected[character] = index
        variants[character] = strokes

    document = json.loads(args.input.read_text(encoding="utf-8"))
    templates = document.get("templates", [])
    seen = set()
    for template in templates:
        character = template["label"]
        if character in variants:
            if template.get("variants"):
                raise ValueError(f"base asset already has variants for {character}")
            template["variants"] = [variants[character]]
            seen.add(character)
    missing = set(k49.HIRAGANA) - seen
    if missing:
        raise ValueError(f"base asset lacks basic hiragana: {''.join(sorted(missing))}")
    document["hiragana_variant_source"] = "Kuzushiji-49 training split"
    document["hiragana_variant_source_commit"] = k49.SOURCE_COMMIT
    document["hiragana_variant_license"] = "CC BY-SA 4.0"
    document["hiragana_variant_selection"] = {
        "algorithm": "deterministic-pool-pixel-centroid-medoid-v1",
        "candidate_pool": args.candidate_pool,
        "development_exclusion": args.development_exclusion,
        "images_sha256": image_hash,
        "labels_sha256": label_hash,
        "classmap_sha256": k49.CLASSMAP_SHA256,
        "record_indices": selected,
    }
    args.output.write_text(
        json.dumps(document, ensure_ascii=False, separators=(",", ":")) + "\n",
        encoding="utf-8",
    )


if __name__ == "__main__":
    main()
