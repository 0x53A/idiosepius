#!/usr/bin/env python3
"""Evaluate kana identity recognition against ETL4/ETL5 handwriting.

ETL4 and ETL5 are offline grayscale images, while Idiosepius recognizes online
pointer trajectories. This tool therefore extracts an order-free one-pixel
skeleton and traces its graph into polylines. The result can evaluate character
identity geometry, but cannot evaluate stroke order, direction, timing,
pressure, or the app's pointer sampling.

The ETL Character Database may only be downloaded from AIST after accepting its
terms, and the data itself may not be redistributed. This tool never downloads
it and writes generated diagnostic samples only below ``target/``.
"""

from __future__ import annotations

import argparse
from collections import defaultdict
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import unicodedata


ROOT = Path(__file__).resolve().parent.parent
OUTPUT = ROOT / "target" / "kana-diagnostics" / "kana-etl-v1"
RECORD_BYTES = 2952
IMAGE_OFFSET = 216
WIDTH = 72
HEIGHT = 76
HIRAGANA = "あいうえおかきくけこさしすせそたちつてとなにぬねのはひふへほまみむめもやゆよらりるれろわをん"
KATAKANA = "アイウエオカキクケコサシスセソタチツテトナニヌネノハヒフヘホマミムメモヤユヨラリルレロワヲン"
INVENTORIES = {"etl4": set(HIRAGANA), "etl5": set(KATAKANA)}
EXPECTED_RECORDS = {"etl4": 6120, "etl5": 10608}
NEIGHBORS = (
    (-1, -1),
    (0, -1),
    (1, -1),
    (-1, 0),
    (1, 0),
    (-1, 1),
    (0, 1),
    (1, 1),
)


def atomic_json(path: Path, document: object) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    data = json.dumps(document, ensure_ascii=False, indent=2).encode() + b"\n"
    with tempfile.NamedTemporaryFile(dir=path.parent, prefix=f".{path.name}.", delete=False) as pending:
        pending.write(data)
        pending.flush()
        os.fsync(pending.fileno())
        pending_path = Path(pending.name)
    os.replace(pending_path, path)


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as source:
        while block := source.read(1024 * 1024):
            digest.update(block)
    return digest.hexdigest()


def decode_label(record: bytes, dataset: str) -> str | None:
    # In a C-type record the JIS X 0201 field starts at bit 72 and keeps its
    # effective eight bits left-aligned, so it is exactly byte nine.
    try:
        halfwidth = bytes([record[9]]).decode("shift_jis")
    except UnicodeDecodeError:
        return None
    katakana = unicodedata.normalize("NFKC", halfwidth)
    if len(katakana) != 1:
        return None
    if dataset == "etl5":
        return katakana
    codepoint = ord(katakana)
    if 0x30A1 <= codepoint <= 0x30F6:
        hiragana = chr(codepoint - 0x60)
        # ETL's legacy labels use small i/e for the obsolete wi/we slots.
        return hiragana.replace("ぃ", "ゐ").replace("ぇ", "ゑ")
    return None


def grayscale(record: bytes) -> list[list[int]]:
    packed = record[IMAGE_OFFSET:]
    if len(packed) != WIDTH * HEIGHT // 2:
        raise ValueError("C-type record has a truncated image payload")
    values = []
    for byte in packed:
        values.extend((byte >> 4, byte & 0x0F))
    return [values[row * WIDTH : (row + 1) * WIDTH] for row in range(HEIGHT)]


def otsu_threshold(image: list[list[int]]) -> int:
    values = [value for row in image for value in row]
    if not values:
        return 0
    histogram = [0] * (max(values) + 1)
    for value in values:
        histogram[value] += 1
    total = len(values)
    weighted_total = sum(index * count for index, count in enumerate(histogram))
    low_count = 0
    low_sum = 0
    best_variance = -1.0
    best_threshold = 0
    for threshold in range(len(histogram) - 1):
        low_count += histogram[threshold]
        low_sum += threshold * histogram[threshold]
        high_count = total - low_count
        if low_count == 0 or high_count == 0:
            continue
        low_mean = low_sum / low_count
        high_mean = (weighted_total - low_sum) / high_count
        variance = low_count * high_count * (low_mean - high_mean) ** 2
        if variance > best_variance:
            best_variance = variance
            best_threshold = threshold
    return best_threshold


def foreground(image: list[list[int]]) -> set[tuple[int, int]]:
    if not image or not image[0]:
        return set()
    if any(len(row) != len(image[0]) for row in image):
        raise ValueError("grayscale image rows have inconsistent widths")
    threshold = otsu_threshold(image)
    border = (
        image[0]
        + image[-1]
        + [row[0] for row in image[1:-1]]
        + [row[-1] for row in image[1:-1]]
    )
    background = sorted(border)[len(border) // 2]
    if background > threshold:
        ink = {
            (x, y)
            for y, row in enumerate(image)
            for x, value in enumerate(row)
            if value <= threshold
        }
    else:
        ink = {
            (x, y)
            for y, row in enumerate(image)
            for x, value in enumerate(row)
            if value > threshold
        }
    return keep_components(ink, minimum=3)


def pixel_neighbors(pixel: tuple[int, int], pixels: set[tuple[int, int]]) -> list[tuple[int, int]]:
    x, y = pixel
    return [(x + dx, y + dy) for dx, dy in NEIGHBORS if (x + dx, y + dy) in pixels]


def keep_components(pixels: set[tuple[int, int]], minimum: int) -> set[tuple[int, int]]:
    remaining = set(pixels)
    kept = set()
    while remaining:
        seed = remaining.pop()
        component = {seed}
        frontier = [seed]
        while frontier:
            pixel = frontier.pop()
            for neighbor in pixel_neighbors(pixel, remaining):
                remaining.remove(neighbor)
                component.add(neighbor)
                frontier.append(neighbor)
        if len(component) >= minimum:
            kept.update(component)
    return kept


def transitions(neighbors: list[bool]) -> int:
    return sum(not neighbors[index] and neighbors[(index + 1) % 8] for index in range(8))


def thin(pixels: set[tuple[int, int]]) -> set[tuple[int, int]]:
    pixels = set(pixels)
    # Zhang-Suen thinning. Its neighbor order is clockwise from north.
    ring = ((0, -1), (1, -1), (1, 0), (1, 1), (0, 1), (-1, 1), (-1, 0), (-1, -1))
    changed = True
    while changed:
        changed = False
        for first_step in (True, False):
            remove = set()
            for x, y in pixels:
                neighbors = [(x + dx, y + dy) in pixels for dx, dy in ring]
                count = sum(neighbors)
                if not 2 <= count <= 6 or transitions(neighbors) != 1:
                    continue
                north, east, south, west = neighbors[0], neighbors[2], neighbors[4], neighbors[6]
                if first_step:
                    preserve1 = north and east and south
                    preserve2 = east and south and west
                else:
                    preserve1 = north and east and west
                    preserve2 = north and south and west
                if not preserve1 and not preserve2:
                    remove.add((x, y))
            if remove:
                pixels.difference_update(remove)
                changed = True
    return pixels


def edge(left: tuple[int, int], right: tuple[int, int]) -> tuple[tuple[int, int], tuple[int, int]]:
    return (left, right) if left < right else (right, left)


def trace_skeleton(pixels: set[tuple[int, int]]) -> list[list[list[float]]]:
    if not pixels:
        return []
    adjacency = {pixel: pixel_neighbors(pixel, pixels) for pixel in pixels}
    nodes = sorted(pixel for pixel, neighbors in adjacency.items() if len(neighbors) != 2)
    visited: set[tuple[tuple[int, int], tuple[int, int]]] = set()
    paths: list[list[tuple[int, int]]] = [
        [pixel] for pixel in nodes if not adjacency[pixel]
    ]

    def trace(start: tuple[int, int], following: tuple[int, int]) -> list[tuple[int, int]]:
        path = [start, following]
        visited.add(edge(start, following))
        previous, current = start, following
        while len(adjacency[current]) == 2:
            candidates = [neighbor for neighbor in adjacency[current] if neighbor != previous]
            if not candidates or edge(current, candidates[0]) in visited:
                break
            following = candidates[0]
            visited.add(edge(current, following))
            path.append(following)
            previous, current = current, following
        return path

    for node in nodes:
        for neighbor in sorted(adjacency[node]):
            if edge(node, neighbor) not in visited:
                paths.append(trace(node, neighbor))

    for start in sorted(pixels):
        for neighbor in sorted(adjacency[start]):
            if edge(start, neighbor) not in visited:
                paths.append(trace(start, neighbor))

    # Connected-component filtering already removed isolated input noise. Keep
    # a skeleton that collapses to one pixel as a tiny segment: it may be a
    # genuine dot-like stroke. Consecutive collinear pixels are simplified.
    result = []
    for path in paths:
        if len(path) == 1:
            x, y = path[0]
            result.append([[x - 0.5, float(y)], [x + 0.5, float(y)]])
            continue
        simplified = [path[0]]
        for index in range(1, len(path) - 1):
            before = path[index - 1]
            current = path[index]
            after = path[index + 1]
            if (current[0] - before[0], current[1] - before[1]) != (
                after[0] - current[0],
                after[1] - current[1],
            ):
                simplified.append(current)
        simplified.append(path[-1])
        result.append([[float(x), float(y)] for x, y in simplified])
    return result


def vectorize_image(image: list[list[int]]) -> list[list[list[float]]]:
    """Convert an arbitrary rectangular grayscale bitmap to order-free paths."""
    return trace_skeleton(thin(foreground(image)))


def vectorize(record: bytes) -> list[list[list[float]]]:
    return vectorize_image(grayscale(record))


def sample(character: str, script: str, strokes: list[list[list[float]]], metadata: dict) -> dict:
    return {
        "format": "idiosepius-kana-sample",
        "format_version": 1,
        "strokes": strokes,
        "expected": character,
        "expected_source": "manual",
        "recognizer": {"script_scope": script},
        "external_source": metadata,
    }


def generate(path: Path, dataset: str, limit: int, output: Path) -> dict:
    size = path.stat().st_size
    if size % RECORD_BYTES:
        raise ValueError(f"{path} is not a whole number of {RECORD_BYTES}-byte C-type records")
    records = size // RECORD_BYTES
    if records != EXPECTED_RECORDS[dataset]:
        raise ValueError(f"{path} contains {records} records; official {dataset.upper()} has {EXPECTED_RECORDS[dataset]}")
    counts: defaultdict[str, int] = defaultdict(int)
    skipped_empty = 0
    script = "hiragana" if dataset == "etl4" else "katakana"
    digest = sha256(path)
    with path.open("rb") as source:
        for record_index in range(records):
            record = source.read(RECORD_BYTES)
            character = decode_label(record, dataset)
            if character not in INVENTORIES[dataset] or counts[character] >= limit:
                continue
            strokes = vectorize(record)
            if not strokes:
                skipped_empty += 1
                continue
            counts[character] += 1
            atomic_json(
                output / dataset / f"u{ord(character):04x}-{counts[character]:03d}.json",
                sample(
                    character,
                    script,
                    strokes,
                    {
                        "name": f"ETL Character Database {dataset.upper()}",
                        "record_index": record_index,
                        "source_sha256": digest,
                        "vectorizer": "otsu-zhang-suen-graph-v1",
                        "reference": "Electrotechnical Laboratory, Japanese Technical Committee for Optical Character Recognition, ETL Character Database, 1973-1984",
                    },
                ),
            )
    return {
        "path": str(path),
        "sha256": digest,
        "records": records,
        "selected": sum(counts.values()),
        "per_character": dict(sorted(counts.items())),
        "missing": "".join(character for character in (HIRAGANA if dataset == "etl4" else KATAKANA) if not counts[character]),
        "skipped_empty": skipped_empty,
    }


def evaluate(samples: Path) -> dict:
    environment = os.environ.copy()
    environment["CARGO_TARGET_DIR"] = str(ROOT / "target")
    completed = subprocess.run(
        (
            environment.get("CARGO", "cargo"),
            "run",
            "--quiet",
            "--release",
            "-p",
            "idiosepius-core",
            "--bin",
            "kana-recognize",
            "--",
            str(samples),
        ),
        cwd=ROOT,
        env=environment,
        check=True,
        stdout=subprocess.PIPE,
        text=True,
    )
    return json.loads(completed.stdout)


def self_test() -> None:
    background = 15
    values = [background] * (WIDTH * HEIGHT)
    for x in range(18, 54):
        values[38 * WIDTH + x] = 0
    payload = bytes((values[index] << 4) | values[index + 1] for index in range(0, len(values), 2))
    record = bytearray(IMAGE_OFFSET) + payload
    record[9] = 0xB1  # halfwidth katakana A
    assert decode_label(record, "etl4") == "あ"
    assert decode_label(record, "etl5") == "ア"
    strokes = vectorize(record)
    assert strokes and sum(len(stroke) for stroke in strokes) >= 2
    assert otsu_threshold(grayscale(record)) < background
    small = [[255] * 7 for _ in range(5)]
    for x in range(1, 6):
        small[2][x] = 0
    assert vectorize_image(small)
    assert foreground([]) == set()
    loop = {(1, 1), (2, 1), (3, 1), (3, 2), (3, 3), (2, 3), (1, 3), (1, 2)}
    assert trace_skeleton(loop)
    assert trace_skeleton({(2, 2)}) == [[[1.5, 2.0], [2.5, 2.0]]]


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--etl4", type=Path, help="uncompressed official ETL4 C-type data file")
    parser.add_argument("--etl5", type=Path, help="uncompressed official ETL5 C-type data file")
    parser.add_argument("--samples-per-character", type=int, default=24)
    parser.add_argument("--self-test", action="store_true")
    args = parser.parse_args()
    if args.self_test:
        self_test()
        print("ETL parser and vectorizer self-test passed")
        return 0
    if args.etl4 is None and args.etl5 is None:
        parser.error("provide --etl4 and/or --etl5, or use --self-test")
    if args.samples_per_character < 1:
        parser.error("--samples-per-character must be positive")

    samples = OUTPUT / "samples"
    if samples.exists():
        shutil.rmtree(samples)
    sources = {}
    try:
        if args.etl4 is not None:
            sources["etl4"] = generate(args.etl4, "etl4", args.samples_per_character, samples)
        if args.etl5 is not None:
            sources["etl5"] = generate(args.etl5, "etl5", args.samples_per_character, samples)
        report = evaluate(samples)
        atomic_json(
            OUTPUT / "summary.json",
            {
                "gate": "idiosepius-kana-etl-v1",
                "note": "Offline handwriting skeleton proxy; not online trajectory or stroke-order evidence.",
                "terms": "ETL data is not redistributed; obtain it from AIST after accepting its terms.",
                "samples_per_character": args.samples_per_character,
                "tool_sha256": hashlib.sha256(Path(__file__).read_bytes()).hexdigest(),
                "sources": sources,
                "report": report,
            },
        )
    except (OSError, ValueError, subprocess.CalledProcessError, json.JSONDecodeError) as error:
        print(f"error: {error}", file=sys.stderr)
        return 1

    print(f"report: {(OUTPUT / 'summary.json').relative_to(ROOT)}")
    for group in report["directory_groups"]:
        print(
            f"{group['name']}: top1={group['top1_rate']:.2%} "
            f"accepted-wrong={group['accepted_wrong_rate']:.2%} "
            f"rejected={group['valid_rejection_rate']:.2%}"
        )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
