#!/usr/bin/env python3
"""Evaluate the kana recognizer against independent public vector sources.

The source repositories are supplied explicitly and must be checked out at the
commits pinned below. Generated samples and reports stay below
``target/kana-diagnostics/kana-cross-source-v1``; no third-party data is copied
into the application.

KanjiVG is the actual holdout because neither half of the embedded template
library comes from it. AnimCJK hiragana is retained as a harsher diagnostic,
but AnimCJK katakana is excluded because those trajectories are the embedded
katakana templates and would only measure self-recognition.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import math
import os
from pathlib import Path
import re
import shutil
import subprocess
import sys
import tempfile
from kana_geometry import animcjk_screen_strokes, self_test as geometry_self_test
import xml.etree.ElementTree as ET


ROOT = Path(__file__).resolve().parent.parent
OUTPUT = ROOT / "target" / "kana-diagnostics" / "kana-cross-source-v1"
KANJIVG_COMMIT = "61e39cfc29724132a6f8823b166296932985a0ff"
ANIMCJK_COMMIT = "ec5e17cca76c87587790bcbce5ea0b4d4fb753d6"
HIRAGANA = "あいうえおかきくけこさしすせそたちつてとなにぬねのはひふへほまみむめもやゆよらりるれろわをん"
KATAKANA = "アイウエオカキクケコサシスセソタチツテトナニヌネノハヒフヘホマミムメモヤユヨラリルレロワヲン"
TOKEN = re.compile(r"[A-Za-z]|[-+]?(?:\d+(?:\.\d*)?|\.\d+)(?:[eE][-+]?\d+)?")
COMMAND_SIZES = {
    "M": 2,
    "L": 2,
    "H": 1,
    "V": 1,
    "C": 6,
    "S": 4,
    "Q": 4,
    "T": 2,
    "Z": 0,
}


def atomic_json(path: Path, document: object) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    data = json.dumps(document, ensure_ascii=False, indent=2).encode() + b"\n"
    with tempfile.NamedTemporaryFile(dir=path.parent, prefix=f".{path.name}.", delete=False) as pending:
        pending.write(data)
        pending.flush()
        os.fsync(pending.fileno())
        pending_path = Path(pending.name)
    os.replace(pending_path, path)


def repository_commit(path: Path) -> str:
    completed = subprocess.run(
        ("git", "-C", str(path), "rev-parse", "HEAD"),
        check=True,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        text=True,
    )
    return completed.stdout.strip()


def require_repository(path: Path, expected: str, name: str) -> None:
    actual = repository_commit(path)
    if actual != expected:
        raise ValueError(f"{name} is at {actual}, expected pinned commit {expected}")


def point_line_distance(point: tuple[float, float], start: tuple[float, float], end: tuple[float, float]) -> float:
    vx = end[0] - start[0]
    vy = end[1] - start[1]
    denominator = vx * vx + vy * vy
    if denominator == 0:
        return math.hypot(point[0] - start[0], point[1] - start[1])
    t = ((point[0] - start[0]) * vx + (point[1] - start[1]) * vy) / denominator
    t = min(1.0, max(0.0, t))
    return math.hypot(point[0] - start[0] - t * vx, point[1] - start[1] - t * vy)


def midpoint(left: tuple[float, float], right: tuple[float, float]) -> tuple[float, float]:
    return ((left[0] + right[0]) * 0.5, (left[1] + right[1]) * 0.5)


def flatten_cubic(
    start: tuple[float, float],
    control1: tuple[float, float],
    control2: tuple[float, float],
    end: tuple[float, float],
    tolerance: float = 0.25,
) -> list[tuple[float, float]]:
    if max(point_line_distance(control1, start, end), point_line_distance(control2, start, end)) <= tolerance:
        return [end]
    a = midpoint(start, control1)
    b = midpoint(control1, control2)
    c = midpoint(control2, end)
    d = midpoint(a, b)
    e = midpoint(b, c)
    middle = midpoint(d, e)
    return flatten_cubic(start, a, d, middle, tolerance) + flatten_cubic(
        middle, e, c, end, tolerance
    )


def flatten_quadratic(
    start: tuple[float, float],
    control: tuple[float, float],
    end: tuple[float, float],
    tolerance: float = 0.25,
) -> list[tuple[float, float]]:
    cubic1 = (
        start[0] + (control[0] - start[0]) * 2.0 / 3.0,
        start[1] + (control[1] - start[1]) * 2.0 / 3.0,
    )
    cubic2 = (
        end[0] + (control[0] - end[0]) * 2.0 / 3.0,
        end[1] + (control[1] - end[1]) * 2.0 / 3.0,
    )
    return flatten_cubic(start, cubic1, cubic2, end, tolerance)


def svg_path_points(data: str) -> list[tuple[float, float]]:
    tokens = TOKEN.findall(data.replace(",", " "))
    points: list[tuple[float, float]] = []
    current = (0.0, 0.0)
    subpath_start = current
    command: str | None = None
    previous_command: str | None = None
    cubic_control: tuple[float, float] | None = None
    quadratic_control: tuple[float, float] | None = None
    index = 0

    def absolute(pair: tuple[float, float], relative: bool) -> tuple[float, float]:
        return (current[0] + pair[0], current[1] + pair[1]) if relative else pair

    while index < len(tokens):
        if tokens[index].isalpha():
            command = tokens[index]
            index += 1
        if command is None:
            raise ValueError("SVG path data starts without a command")
        upper = command.upper()
        if upper not in COMMAND_SIZES:
            raise ValueError(f"unsupported SVG path command {command!r}")
        if upper == "Z":
            if current != subpath_start:
                points.append(subpath_start)
            current = subpath_start
            previous_command = command
            command = None
            cubic_control = None
            quadratic_control = None
            continue
        count = COMMAND_SIZES[upper]
        if index + count > len(tokens) or any(token.isalpha() for token in tokens[index : index + count]):
            raise ValueError(f"SVG path command {command!r} lacks parameters")
        values = [float(token) for token in tokens[index : index + count]]
        index += count
        relative = command.islower()

        if upper == "M":
            current = absolute((values[0], values[1]), relative)
            subpath_start = current
            points.append(current)
            command = "l" if relative else "L"
        elif upper == "L":
            current = absolute((values[0], values[1]), relative)
            points.append(current)
        elif upper == "H":
            current = (current[0] + values[0], current[1]) if relative else (values[0], current[1])
            points.append(current)
        elif upper == "V":
            current = (current[0], current[1] + values[0]) if relative else (current[0], values[0])
            points.append(current)
        elif upper == "C":
            control1 = absolute((values[0], values[1]), relative)
            control2 = absolute((values[2], values[3]), relative)
            end = absolute((values[4], values[5]), relative)
            points.extend(flatten_cubic(current, control1, control2, end))
            current = end
            cubic_control = control2
        elif upper == "S":
            control1 = (
                (2 * current[0] - cubic_control[0], 2 * current[1] - cubic_control[1])
                if previous_command is not None
                and previous_command.upper() in {"C", "S"}
                and cubic_control is not None
                else current
            )
            control2 = absolute((values[0], values[1]), relative)
            end = absolute((values[2], values[3]), relative)
            points.extend(flatten_cubic(current, control1, control2, end))
            current = end
            cubic_control = control2
        elif upper == "Q":
            control = absolute((values[0], values[1]), relative)
            end = absolute((values[2], values[3]), relative)
            points.extend(flatten_quadratic(current, control, end))
            current = end
            quadratic_control = control
        elif upper == "T":
            control = (
                (2 * current[0] - quadratic_control[0], 2 * current[1] - quadratic_control[1])
                if previous_command is not None
                and previous_command.upper() in {"Q", "T"}
                and quadratic_control is not None
                else current
            )
            end = absolute((values[0], values[1]), relative)
            points.extend(flatten_quadratic(current, control, end))
            current = end
            quadratic_control = control
        previous_command = command
        if upper not in {"C", "S"}:
            cubic_control = None
        if upper not in {"Q", "T"}:
            quadratic_control = None
    return points


def kanjivg_strokes(root: Path, character: str) -> list[list[list[float]]]:
    source = root / "kanji" / f"{ord(character):05x}.svg"
    document = ET.parse(source).getroot()
    namespace = {"svg": "http://www.w3.org/2000/svg"}
    paths = document.findall(".//svg:path", namespace)
    strokes = []
    for path in paths:
        identifier = path.attrib.get("id", "")
        if "-s" not in identifier:
            continue
        points = svg_path_points(path.attrib["d"])
        if len(points) < 2:
            raise ValueError(f"{source}: stroke {identifier} has insufficient geometry")
        strokes.append([[round(x, 5), round(y, 5)] for x, y in points])
    if not strokes:
        raise ValueError(f"{source}: no stroke paths found")
    return strokes


def animcjk_rows(root: Path) -> dict[str, dict]:
    rows = {}
    with (root / "graphicsJaKana.txt").open(encoding="utf-8") as source:
        for line in source:
            row = json.loads(line)
            rows[row["character"]] = row
    return rows


def sample(character: str, script: str, strokes: list[list[list[float]]], source: dict) -> dict:
    return {
        "format": "idiosepius-kana-sample",
        "format_version": 1,
        "strokes": strokes,
        "expected": character,
        "expected_source": "manual",
        "recognizer": {"script_scope": script},
        "external_source": source,
    }


def generate_samples(kanjivg: Path, animcjk: Path, output: Path) -> None:
    if output.exists():
        shutil.rmtree(output)
    for character, script in ((character, "hiragana") for character in HIRAGANA):
        atomic_json(
            output / "kanjivg-hiragana" / f"u{ord(character):04x}.json",
            sample(
                character,
                script,
                kanjivg_strokes(kanjivg, character),
                {
                    "name": "KanjiVG",
                    "commit": KANJIVG_COMMIT,
                    "license": "CC-BY-SA-3.0",
                    "url": "https://github.com/KanjiVG/kanjivg",
                },
            ),
        )
    for character, script in ((character, "katakana") for character in KATAKANA):
        atomic_json(
            output / "kanjivg-katakana" / f"u{ord(character):04x}.json",
            sample(
                character,
                script,
                kanjivg_strokes(kanjivg, character),
                {
                    "name": "KanjiVG",
                    "commit": KANJIVG_COMMIT,
                    "license": "CC-BY-SA-3.0",
                    "url": "https://github.com/KanjiVG/kanjivg",
                },
            ),
        )

    rows = animcjk_rows(animcjk)
    for character in HIRAGANA:
        atomic_json(
            output / "animcjk-hiragana" / f"u{ord(character):04x}.json",
            sample(
                character,
                "hiragana",
                animcjk_screen_strokes(rows[character]["medians"]),
                {
                    "name": "AnimCJK",
                    "commit": ANIMCJK_COMMIT,
                    "license": "LGPL-3.0-or-later",
                    "url": "https://github.com/parsimonhi/animCJK",
                },
            ),
        )


def source_fingerprint(samples: Path) -> str:
    digest = hashlib.sha256()
    for path in sorted(samples.rglob("*.json")):
        relative = path.relative_to(samples).as_posix().encode()
        contents = path.read_bytes()
        digest.update(len(relative).to_bytes(8, "big"))
        digest.update(relative)
        digest.update(len(contents).to_bytes(8, "big"))
        digest.update(contents)
    return digest.hexdigest()


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
    geometry_self_test()
    points = svg_path_points("M0,0 C0,10 10,10 10,0 s10,-10 10,0")
    assert points[0] == (0.0, 0.0)
    assert points[-1] == (20.0, 0.0)
    assert max(y for _, y in points) > 0.0
    relative = svg_path_points("m1,2 3,4 l5,6 h2 v3")
    assert relative == [(1.0, 2.0), (4.0, 6.0), (9.0, 12.0), (11.0, 12.0), (11.0, 15.0)]
    with tempfile.TemporaryDirectory() as directory:
        root = Path(directory)
        (root / "kanji").mkdir()
        (root / "kanji" / "03042.svg").write_text(
            """<svg xmlns="http://www.w3.org/2000/svg">
            <g id="outer"><g id="inner">
            <path id="kvg:03042-s1" d="M0,0 C0,1 1,1 1,0"/>
            </g></g></svg>""",
            encoding="utf-8",
        )
        strokes = kanjivg_strokes(root, "あ")
        assert len(strokes) == 1
        assert strokes[0][0] == [0.0, 0.0]
        assert strokes[0][-1] == [1.0, 0.0]


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--kanjivg", type=Path)
    parser.add_argument("--animcjk", type=Path)
    parser.add_argument("--self-test", action="store_true")
    args = parser.parse_args()
    if args.self_test:
        self_test()
        print("cross-source SVG parser self-test passed")
        return 0
    if args.kanjivg is None or args.animcjk is None:
        parser.error("provide both --kanjivg and --animcjk, or use --self-test")
    try:
        require_repository(args.kanjivg, KANJIVG_COMMIT, "KanjiVG")
        require_repository(args.animcjk, ANIMCJK_COMMIT, "AnimCJK")
        samples = OUTPUT / "samples"
        generate_samples(args.kanjivg, args.animcjk, samples)
        report = evaluate(samples)
        summary = {
            "gate": "idiosepius-kana-cross-source-v1",
            "note": "Public canonical vectors are cross-source evidence, not real handwriting.",
            "sources": {
                "kanjivg": {"commit": KANJIVG_COMMIT, "role": "holdout"},
                "animcjk-hiragana": {"commit": ANIMCJK_COMMIT, "role": "diagnostic"},
            },
            "generated_sample_fingerprint": {
                "algorithm": "sha256-path-and-bytes-v1",
                "value": source_fingerprint(samples),
            },
            "tool_sha256": hashlib.sha256(Path(__file__).read_bytes()).hexdigest(),
            "report": report,
        }
        atomic_json(OUTPUT / "summary.json", summary)
    except (OSError, ValueError, ET.ParseError, subprocess.CalledProcessError, json.JSONDecodeError) as error:
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
