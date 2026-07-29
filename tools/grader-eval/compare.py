#!/usr/bin/env python3
"""Print a compact comparison table for grader-eval JSON reports."""

from __future__ import annotations

import argparse
import json
import math
import statistics
from pathlib import Path


def percentile(values: list[float], fraction: float) -> float:
    """Nearest-rank percentile, suitable for the benchmark's small samples."""
    if not values:
        return 0.0
    ordered = sorted(values)
    return ordered[max(0, math.ceil(fraction * len(ordered)) - 1)]


def load_report(path: Path) -> dict:
    try:
        report = json.loads(path.read_text())
    except (OSError, json.JSONDecodeError) as error:
        raise SystemExit(f"{path}: {error}") from error
    if not isinstance(report.get("results"), list) or not report["results"]:
        raise SystemExit(f"{path}: report has no results")
    return report


def metrics(path: Path) -> dict:
    report = load_report(path)
    results = report["results"]
    # Reports come in two shapes. The API harness records one `latency_ms` per
    # case; the older in-process llama.cpp harness split the same wall time
    # into `prefill_ms` + `generation_ms`. Read either so a stored baseline
    # still lines up in the table against a fresh run.
    latencies = [
        float(
            result.get(
                "latency_ms",
                float(result.get("prefill_ms", 0.0))
                + float(result.get("generation_ms", 0.0)),
            )
        )
        for result in results
    ]
    total_ms = sum(latencies)
    generated_tokens = sum(int(result.get("generated_tokens", 0)) for result in results)
    exact = sum(bool(result["exact"]) for result in results)
    peak_rss_kib = report.get("peak_rss_kib")
    model_bytes = report.get("model_bytes")
    return {
        "model": str(report["model_id"]),
        "backend": str(report["backend"]),
        "threads": report.get("threads", "n/a"),
        "evaluations": len(results),
        "accuracy": exact / len(results),
        "false_accepts": sum(
            result["expected"] == "incorrect" and result.get("predicted") == "correct"
            for result in results
        ),
        "false_rejects": sum(
            result["expected"] == "correct" and result.get("predicted") == "incorrect"
            for result in results
        ),
        "wrong_uncertain": sum(
            result["expected"] != "uncertain" and result.get("predicted") == "uncertain"
            for result in results
        ),
        "missed_uncertain": sum(
            result["expected"] == "uncertain" and result.get("predicted") != "uncertain"
            for result in results
        ),
        "parse_failures": sum(result.get("predicted") is None for result in results),
        "median_ms": statistics.median(latencies),
        "p95_ms": percentile(latencies, 0.95),
        "tokens_per_second": (
            generated_tokens * 1000.0 / total_ms if total_ms else 0.0
        ),
        # Both are properties of an in-process model. An API candidate has
        # neither, so they print as "n/a" rather than being faked as zero.
        "model_gib": (
            int(model_bytes) / 1024**3 if model_bytes is not None else None
        ),
        "peak_rss_mib": (
            int(peak_rss_kib) / 1024 if peak_rss_kib is not None else None
        ),
        "path": path,
    }


def cell(value: object) -> str:
    return str(value).replace("|", r"\|")


def main() -> None:
    parser = argparse.ArgumentParser(
        description="Compare local answer-grader benchmark reports"
    )
    parser.add_argument("reports", type=Path, nargs="+")
    args = parser.parse_args()

    rows = sorted(
        (metrics(path) for path in args.reports),
        key=lambda row: (-row["accuracy"], row["median_ms"]),
    )
    headings = [
        "model",
        "backend",
        "threads",
        "N",
        "accuracy",
        "FA",
        "FR",
        "wrong ?",
        "missed ?",
        "parse",
        "p50 ms",
        "p95 ms",
        "tok/s",
        "GGUF GiB",
        "RSS MiB",
    ]
    print("| " + " | ".join(headings) + " |")
    print("| " + " | ".join(["---"] * len(headings)) + " |")
    for row in rows:
        values = [
            row["model"],
            row["backend"],
            row["threads"],
            row["evaluations"],
            f'{row["accuracy"]:.1%}',
            row["false_accepts"],
            row["false_rejects"],
            row["wrong_uncertain"],
            row["missed_uncertain"],
            row["parse_failures"],
            f'{row["median_ms"]:.0f}',
            f'{row["p95_ms"]:.0f}',
            f'{row["tokens_per_second"]:.2f}',
            (f'{row["model_gib"]:.2f}' if row["model_gib"] is not None else "n/a"),
            (
                f'{row["peak_rss_mib"]:.0f}'
                if row["peak_rss_mib"] is not None
                else "n/a"
            ),
        ]
        print("| " + " | ".join(cell(value) for value in values) + " |")


if __name__ == "__main__":
    main()
