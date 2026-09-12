#!/usr/bin/env python3
"""Run the deterministic kana recognizer safety gate.

Every generated artifact stays below target/kana-diagnostics/kana-gate-v1.
This is synthetic regression evidence, not a substitute for held-out writers.
"""

from __future__ import annotations

import hashlib
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import time


ROOT = Path(__file__).resolve().parent.parent
OUTPUT = ROOT / "target" / "kana-diagnostics" / "kana-gate-v1"
SEEDS = (0x11112222, 0x33334444, 0x55556666, 0x72616C7068)
SAMPLES_PER_CHARACTER = 12
SEED_TIMEOUT_SECONDS = 600
FINGERPRINT_FILES = (
    Path("crates/core/src/kana.rs"),
    Path("crates/core/src/bin/kana-recognize.rs"),
    Path("crates/core/assets/kana_templates.json"),
    Path("tools/build-kana-templates.py"),
    Path("tools/kana_geometry.py"),
    Path("tools/check-kana.py"),
)
LIMITS = {
    "minimum_top1_rate": 0.99,
    "minimum_top5_rate": 1.0,
    "maximum_accepted_wrong_rate": 0.005,
    "maximum_valid_rejection_rate": 0.06,
    "minimum_invalid_rejection_rate": 0.99,
}


def atomic_json(path: Path, document: object) -> None:
    data = json.dumps(document, ensure_ascii=False, indent=2).encode() + b"\n"
    with tempfile.NamedTemporaryFile(dir=path.parent, prefix=f".{path.name}.", delete=False) as pending:
        pending.write(data)
        pending.flush()
        os.fsync(pending.fileno())
        pending_path = Path(pending.name)
    os.replace(pending_path, path)


def source_fingerprint() -> dict:
    digest = hashlib.sha256()
    for relative in FINGERPRINT_FILES:
        path_bytes = relative.as_posix().encode()
        contents = (ROOT / relative).read_bytes()
        digest.update(len(path_bytes).to_bytes(8, "big"))
        digest.update(path_bytes)
        digest.update(len(contents).to_bytes(8, "big"))
        digest.update(contents)
    return {
        "algorithm": "sha256-path-and-bytes-v1",
        "value": digest.hexdigest(),
        "files": [path.as_posix() for path in FINGERPRINT_FILES],
    }


def evaluate(seed: int) -> dict:
    environment = os.environ.copy()
    environment["CARGO_TARGET_DIR"] = str(ROOT / "target")
    command = (
        environment.get("CARGO", "cargo"),
        "run",
        "--quiet",
        "--release",
        "-p",
        "idiosepius-core",
        "--bin",
        "kana-recognize",
        "--",
        "--synthetic-scripted",
        str(SAMPLES_PER_CHARACTER),
        hex(seed),
    )
    completed = subprocess.run(
        command,
        cwd=ROOT,
        env=environment,
        check=True,
        stdout=subprocess.PIPE,
        text=True,
        timeout=SEED_TIMEOUT_SECONDS,
    )
    return json.loads(completed.stdout)


def check(document: dict, seed: int) -> list[str]:
    report = document["report"]
    problems = []
    expected_samples = report["character_count"] * SAMPLES_PER_CHARACTER
    if document["generator"] != "kana-synthetic-v3":
        problems.append(f"unexpected generator {document['generator']!r}")
    if report["generator"] != document["generator"]:
        problems.append("wrapper and report generator provenance differ")
    if report["recognizer"] != document["recognizer"]:
        problems.append("wrapper and report recognizer provenance differ")
    if document["scope"] != "per_script":
        problems.append(f"unexpected scope {document['scope']!r}")
    if document["seed"] != seed or report["seed"] != seed:
        problems.append(
            f"seed provenance is {document['seed']!r}/{report['seed']!r}, expected {seed}"
        )
    if not report["scripted"]:
        problems.append("report did not use per-script recognition")
    if report["samples_per_character"] != SAMPLES_PER_CHARACTER:
        problems.append(
            f"report has {report['samples_per_character']} variants per character, "
            f"expected {SAMPLES_PER_CHARACTER}"
        )
    if report["samples"] != expected_samples:
        problems.append(f"evaluated {report['samples']} valid samples, expected {expected_samples}")
    verdict_total = report["accepted_correct"] + report["accepted_wrong"] + report["rejected"]
    if verdict_total != report["samples"]:
        problems.append(f"valid verdict partition totals {verdict_total}, expected {report['samples']}")
    if document["top1_rate"] < LIMITS["minimum_top1_rate"]:
        problems.append(f"top-1 rate {document['top1_rate']:.4%} is below the gate")
    if document["top5_rate"] < LIMITS["minimum_top5_rate"]:
        problems.append(f"top-5 rate {document['top5_rate']:.4%} is below the gate")
    if document["accepted_wrong_rate"] > LIMITS["maximum_accepted_wrong_rate"]:
        problems.append(
            f"accepted-wrong rate {document['accepted_wrong_rate']:.4%} is above the gate"
        )
    if document["rejection_rate"] > LIMITS["maximum_valid_rejection_rate"]:
        problems.append(f"valid rejection rate {document['rejection_rate']:.4%} is above the gate")
    if document["invalid_rejection_rate"] < LIMITS["minimum_invalid_rejection_rate"]:
        problems.append(
            f"invalid rejection rate {document['invalid_rejection_rate']:.4%} is below the gate"
        )
    for character in report["characters"]:
        character_total = (
            character["accepted_correct"] + character["accepted_wrong"] + character["rejected"]
        )
        if character_total != character["samples"]:
            problems.append(f"{character['character']} verdict partition is inconsistent")
    return problems


def main() -> int:
    OUTPUT.mkdir(parents=True, exist_ok=True)
    summary_path = OUTPUT / "summary.json"
    runs = []
    started_at_unix_ms = time.time_ns() // 1_000_000
    summary = {
        "gate": "idiosepius-kana-synthetic-gate-v1",
        "note": "Synthetic regression evidence; not real-handwriting accuracy.",
        "samples_per_character": SAMPLES_PER_CHARACTER,
        "timeout_seconds_per_seed": SEED_TIMEOUT_SECONDS,
        "source_fingerprint": source_fingerprint(),
        "started_at_unix_ms": started_at_unix_ms,
        "limits": LIMITS,
        "status": "running",
        "passed": None,
        "runs": runs,
    }
    atomic_json(summary_path, summary)
    for seed in SEEDS:
        try:
            document = evaluate(seed)
        except (
            subprocess.CalledProcessError,
            subprocess.TimeoutExpired,
            json.JSONDecodeError,
            OSError,
        ) as error:
            summary["status"] = "failed"
            summary["passed"] = False
            summary["error"] = f"seed {seed:#x}: {error}"
            summary["completed_at_unix_ms"] = time.time_ns() // 1_000_000
            atomic_json(summary_path, summary)
            print(summary["error"], file=sys.stderr)
            print(f"summary: {summary_path.relative_to(ROOT)}")
            return 1
        path = OUTPUT / f"seed-{seed:010x}.json"
        atomic_json(path, document)
        try:
            problems = check(document, seed)
        except (KeyError, TypeError, ValueError) as error:
            summary["status"] = "failed"
            summary["passed"] = False
            summary["error"] = f"seed {seed:#x} produced an invalid report: {error}"
            summary["completed_at_unix_ms"] = time.time_ns() // 1_000_000
            atomic_json(summary_path, summary)
            print(summary["error"], file=sys.stderr)
            print(f"summary: {summary_path.relative_to(ROOT)}")
            return 1
        runs.append(
            {
                "seed": seed,
                "file": path.relative_to(ROOT).as_posix(),
                "top1_rate": document["top1_rate"],
                "top5_rate": document["top5_rate"],
                "accepted_correct_rate": document["accepted_correct_rate"],
                "accepted_wrong_rate": document["accepted_wrong_rate"],
                "valid_rejection_rate": document["rejection_rate"],
                "invalid_rejection_rate": document["invalid_rejection_rate"],
                "mean_recognition_ms": document["mean_recognition_ms"],
                "elapsed_ms": document["elapsed_ms"],
                "problems": problems,
            }
        )
        state = "PASS" if not problems else "FAIL"
        print(
            f"{state} {seed:#014x}  top1={document['top1_rate']:.2%}  "
            f"accepted-wrong={document['accepted_wrong_rate']:.2%}  "
            f"valid-rejected={document['rejection_rate']:.2%}  "
            f"invalid-rejected={document['invalid_rejection_rate']:.2%}"
        )
        for problem in problems:
            print(f"  {problem}", file=sys.stderr)

    passed = all(not run["problems"] for run in runs)
    summary["recognizer"] = document["recognizer"]
    summary["status"] = "passed" if passed else "failed"
    summary["passed"] = passed
    summary["completed_at_unix_ms"] = time.time_ns() // 1_000_000
    atomic_json(summary_path, summary)
    print(f"summary: {summary_path.relative_to(ROOT)}")
    return 0 if passed else 1


if __name__ == "__main__":
    raise SystemExit(main())
