#!/usr/bin/env python3
"""Evaluate hiragana recognition against the Kuzushiji-49 train/test split.

K49 contains 28x28 offline images of historical, cursive hiragana.  They are a
deliberately difficult identity-geometry proxy, not evidence about modern pen
trajectories.  The official training split is development data.  The official
test split is an explicit holdout and must not be inspected while tuning.

The tool does not download data.  It verifies the official archives byte for
byte, parses their small NumPy container format using the standard library,
and writes derived diagnostic samples only below ``target/``.
"""

from __future__ import annotations

import argparse
import ast
import csv
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import shutil
import struct
import subprocess
import sys
import tempfile
import zipfile


ROOT = Path(__file__).resolve().parent.parent
OUTPUT = ROOT / "target" / "kana-diagnostics" / "kana-k49-v1"
HIRAGANA = "あいうえおかきくけこさしすせそたちつてとなにぬねのはひふへほまみむめもやゆよらりるれろわをん"
EXPECTED = {
    "development": {
        "records": 232365,
        "images_sha256": "1c42adc463ed8efe598cf002f6ecafbca6d8c38f0551f7a35e0b23755fca1c8d",
        "labels_sha256": "fbfef4750ce9aa70b6072f0bca7daa9e80e2d79b379d5ae923265a63396260e4",
    },
    "holdout": {
        "records": 38547,
        "images_sha256": "1de2476bb29ed1a12a2424e6724349f7519f6f39d112848aea6aa9c21eeaf594",
        "labels_sha256": "299be30ea32c2cc69318f5a5a579ffbffb0a6a1ccff134fb0ca834cd525ad6af",
    },
}
CLASSMAP_SHA256 = "95f5a2cfbfba1721f566059cf77c22808898b63044a5f9a08de9d8a2950eed84"
SOURCE_COMMIT = "eb78dc13345cb00a9ced461e9be277f845534dff"
SOURCE_URL = "https://github.com/rois-codh/kmnist"
DATASET_URL = "https://codh.rois.ac.jp/kmnist/"


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as source:
        while block := source.read(1024 * 1024):
            digest.update(block)
    return digest.hexdigest()


def require_hash(path: Path, expected: str, name: str) -> str:
    actual = sha256(path)
    if actual != expected:
        raise ValueError(f"{name} SHA-256 is {actual}, expected {expected}")
    return actual


def atomic_json(path: Path, document: object) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    data = json.dumps(document, ensure_ascii=False, indent=2).encode() + b"\n"
    with tempfile.NamedTemporaryFile(dir=path.parent, prefix=f".{path.name}.", delete=False) as pending:
        pending.write(data)
        pending.flush()
        os.fsync(pending.fileno())
        pending_path = Path(pending.name)
    os.replace(pending_path, path)


def npy_from_npz(path: Path, expected_shape: tuple[int, ...]) -> bytes:
    with zipfile.ZipFile(path) as archive:
        if archive.namelist() != ["arr_0.npy"]:
            raise ValueError(f"{path} must contain only arr_0.npy")
        document = archive.read("arr_0.npy")
    if document[:6] != b"\x93NUMPY":
        raise ValueError(f"{path} does not contain a NumPy array")
    version = tuple(document[6:8])
    if version == (1, 0):
        header_length = struct.unpack("<H", document[8:10])[0]
        header_start = 10
    elif version in {(2, 0), (3, 0)}:
        header_length = struct.unpack("<I", document[8:12])[0]
        header_start = 12
    else:
        raise ValueError(f"{path} uses unsupported NPY version {version}")
    header_end = header_start + header_length
    encoding = "utf-8" if version == (3, 0) else "latin1"
    try:
        header = ast.literal_eval(document[header_start:header_end].decode(encoding))
    except (SyntaxError, ValueError, UnicodeDecodeError) as error:
        raise ValueError(f"{path} has an invalid NPY header") from error
    expected_header = {"descr": "|u1", "fortran_order": False, "shape": expected_shape}
    if header != expected_header:
        raise ValueError(f"{path} array is {header}, expected {expected_header}")
    payload = document[header_end:]
    expected_bytes = 1
    for extent in expected_shape:
        expected_bytes *= extent
    if len(payload) != expected_bytes:
        raise ValueError(f"{path} contains {len(payload)} data bytes, expected {expected_bytes}")
    return payload


def classmap(path: Path) -> dict[int, str]:
    require_hash(path, CLASSMAP_SHA256, "K49 class map")
    with path.open(encoding="utf-8", newline="") as source:
        rows = list(csv.DictReader(source))
    mapping = {}
    for row in rows:
        index = int(row["index"])
        character = row["char"]
        if row["codepoint"] != f"U+{ord(character):04X}":
            raise ValueError(f"class {index} has inconsistent character metadata")
        mapping[index] = character
    if set(mapping) != set(range(49)):
        raise ValueError("K49 class map does not contain exactly classes 0 through 48")
    if "".join(mapping[index] for index in range(49) if mapping[index] in HIRAGANA) != HIRAGANA:
        raise ValueError("K49 class map no longer matches the basic hiragana inventory")
    return mapping


def vectorizer():
    path = ROOT / "tools" / "check-kana-etl.py"
    spec = importlib.util.spec_from_file_location("idiosepius_kana_bitmap", path)
    if spec is None or spec.loader is None:
        raise ValueError(f"cannot load bitmap vectorizer from {path}")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module.vectorize_image


def selection_key(index: int) -> bytes:
    return hashlib.sha256(f"idiosepius-k49-v1:{index}".encode()).digest()


def selected_indices(labels: bytes, mapping: dict[int, str], limit: int) -> dict[str, list[int]]:
    by_character = {character: [] for character in HIRAGANA}
    for index, label in enumerate(labels):
        character = mapping.get(label)
        if character in by_character:
            by_character[character].append(index)
    result = {}
    for character, indices in by_character.items():
        indices.sort(key=selection_key)
        result[character] = sorted(indices[:limit])
    return result


def sample(character: str, strokes: list[list[list[float]]], role: str, index: int, hashes: dict) -> dict:
    return {
        "format": "idiosepius-kana-sample",
        "format_version": 1,
        "strokes": strokes,
        "expected": character,
        "expected_source": "manual",
        "recognizer": {"script_scope": "hiragana"},
        "external_source": {
            "name": "Kuzushiji-49",
            "role": role,
            "record_index": index,
            "images_sha256": hashes["images"],
            "labels_sha256": hashes["labels"],
            "classmap_sha256": CLASSMAP_SHA256,
            "source_commit": SOURCE_COMMIT,
            "license": "CC BY-SA 4.0",
            "vectorizer": "otsu-zhang-suen-graph-v1",
            "limitations": "historical offline bitmap; no stroke order, direction, timing, or pressure",
        },
    }


def generate(images_path: Path, labels_path: Path, map_path: Path, role: str, limit: int) -> tuple[Path, dict]:
    expected = EXPECTED[role]
    hashes = {
        "images": require_hash(images_path, expected["images_sha256"], f"K49 {role} images"),
        "labels": require_hash(labels_path, expected["labels_sha256"], f"K49 {role} labels"),
    }
    mapping = classmap(map_path)
    records = expected["records"]
    images = npy_from_npz(images_path, (records, 28, 28))
    labels = npy_from_npz(labels_path, (records,))
    chosen = selected_indices(labels, mapping, limit)
    output = OUTPUT / role / "samples"
    if output.exists():
        shutil.rmtree(output)
    convert = vectorizer()
    counts = {}
    skipped_empty = 0
    for character, indices in chosen.items():
        count = 0
        for index in indices:
            offset = index * 28 * 28
            pixels = images[offset : offset + 28 * 28]
            image = [list(pixels[row * 28 : (row + 1) * 28]) for row in range(28)]
            strokes = convert(image)
            if not strokes:
                skipped_empty += 1
                continue
            count += 1
            atomic_json(
                output / "k49" / f"u{ord(character):04x}-{count:03d}-r{index:06d}.json",
                sample(character, strokes, role, index, hashes),
            )
        counts[character] = count
    return output, {
        "records": records,
        "selected": sum(counts.values()),
        "per_character": counts,
        "missing": "".join(character for character in HIRAGANA if not counts[character]),
        "skipped_empty": skipped_empty,
        "images": {"path": str(images_path), "sha256": hashes["images"]},
        "labels": {"path": str(labels_path), "sha256": hashes["labels"]},
        "classmap": {"path": str(map_path), "sha256": CLASSMAP_SHA256},
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


def balanced_rates(report: dict) -> dict:
    characters = report["characters"]
    if not characters:
        return {"top1": None, "accepted_correct": None, "accepted_wrong": None}
    return {
        key: sum(row[field] for row in characters) / len(characters)
        for key, field in (
            ("top1", "top1_rate"),
            ("accepted_correct", "accepted_correct_rate"),
            ("accepted_wrong", "accepted_wrong_rate"),
        )
    }


def npy_document(payload: bytes, shape: tuple[int, ...]) -> bytes:
    header = repr({"descr": "|u1", "fortran_order": False, "shape": shape}).encode("latin1")
    padding = (64 - ((10 + len(header) + 1) % 64)) % 64
    header += b" " * padding + b"\n"
    return b"\x93NUMPY\x01\x00" + struct.pack("<H", len(header)) + header + payload


def self_test() -> None:
    with tempfile.TemporaryDirectory() as temporary:
        archive_path = Path(temporary) / "tiny.npz"
        payload = bytes(range(12))
        with zipfile.ZipFile(archive_path, "w", compression=zipfile.ZIP_DEFLATED) as archive:
            archive.writestr("arr_0.npy", npy_document(payload, (3, 2, 2)))
        assert npy_from_npz(archive_path, (3, 2, 2)) == payload
    mapping = {index: character for index, character in enumerate(HIRAGANA)}
    labels = bytes([2, 1, 0, 1, 2, 0])
    chosen = selected_indices(labels, mapping, 1)
    assert all(len(chosen[character]) == (1 if character in HIRAGANA[:3] else 0) for character in HIRAGANA)
    convert = vectorizer()
    image = [[255] * 7 for _ in range(7)]
    for coordinate in range(1, 6):
        image[coordinate][coordinate] = 0
    assert convert(image)


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--images", type=Path)
    parser.add_argument("--labels", type=Path)
    parser.add_argument("--classmap", type=Path)
    parser.add_argument("--role", choices=tuple(EXPECTED), default="development")
    parser.add_argument("--samples-per-character", type=int, default=24)
    parser.add_argument("--self-test", action="store_true")
    args = parser.parse_args()
    if args.self_test:
        self_test()
        print("K49 container and bitmap-vectorizer self-test passed")
        return 0
    if any(path is None for path in (args.images, args.labels, args.classmap)):
        parser.error("provide --images, --labels, and --classmap, or use --self-test")
    if args.samples_per_character < 1:
        parser.error("--samples-per-character must be positive")

    try:
        samples, source = generate(
            args.images,
            args.labels,
            args.classmap,
            args.role,
            args.samples_per_character,
        )
        report = evaluate(samples)
        summary = {
            "gate": "idiosepius-kana-k49-v1",
            "role": args.role,
            "holdout_policy": (
                "Development split; results may guide changes."
                if args.role == "development"
                else "Official test split; run only after freezing the recognizer."
            ),
            "note": "Historical offline bitmap skeleton proxy; not modern online handwriting evidence.",
            "license": "CC BY-SA 4.0",
            "attribution": "Clanuwat et al., Deep Learning for Classical Japanese Literature, arXiv:1812.01718 (2018)",
            "source_url": SOURCE_URL,
            "dataset_url": DATASET_URL,
            "source_commit": SOURCE_COMMIT,
            "samples_per_character": args.samples_per_character,
            "tool_sha256": hashlib.sha256(Path(__file__).read_bytes()).hexdigest(),
            "source": source,
            "balanced_rates": balanced_rates(report),
            "report": report,
        }
        destination = OUTPUT / args.role / "summary.json"
        atomic_json(destination, summary)
    except (OSError, ValueError, KeyError, subprocess.CalledProcessError, json.JSONDecodeError) as error:
        print(f"error: {error}", file=sys.stderr)
        return 1

    print(f"report: {destination.relative_to(ROOT)}")
    print(
        f"{args.role}: top1={report['top1_rate']:.2%} "
        f"accepted-wrong={report['accepted_wrong_rate']:.2%} "
        f"rejected={report['valid_rejection_rate']:.2%}"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
