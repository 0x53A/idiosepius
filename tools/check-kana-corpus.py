#!/usr/bin/env python3
"""Replay a captured corpus through both classifiers and report labeled outcomes.

Exact duplicate ink is counted once, including overlapping cumulative exports.
Expected labels are annotations; they never select classifier candidates.
Scores and agreement are diagnostics, not an acceptance or quality policy.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import subprocess
from collections import Counter, defaultdict
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
INVENTORIES = {
    "hiragana": "あいうえおかきくけこさしすせそたちつてとなにぬねのはひふへほまみむめもやゆよらりるれろわをん",
    "katakana": "アイウエオカキクケコサシスセソタチツテトナニヌネノハヒフヘホマミムメモヤユヨラリルレロワヲン",
}


def writing_validity(doc):
    value = doc.get("writing_validity", "invalid" if doc.get("expected_invalid") else "unknown")
    if value not in ("unknown", "valid", "invalid"):
        raise ValueError("invalid writing_validity annotation")
    if doc.get("expected_invalid") and value != "invalid":
        raise ValueError("non-kana annotation conflicts with writing_validity")
    return value


def unique_samples(documents):
    unique = {}
    duplicates = 0
    for name, doc in documents:
        script = doc.get("script", doc.get("recognizer", {}).get("script_scope"))
        if script not in INVENTORIES:
            raise ValueError(f"{name}: missing or invalid script scope")
        expected = doc.get("expected")
        invalid = doc.get("expected_invalid", False)
        validity = writing_validity(doc)
        if not isinstance(invalid, bool) or (
            expected is not None
            and (
                not isinstance(expected, str)
                or len(expected) != 1
                or expected not in INVENTORIES[script]
            )
        ):
            raise ValueError(f"{name}: invalid expected annotation")
        if expected is not None and invalid:
            raise ValueError(f"{name}: both a kana label and invalid-ink annotation")
        ink = doc.get("strokes")
        if not isinstance(ink, list):
            raise TypeError(f"{name}: missing strokes")
        key = hashlib.sha256(
            json.dumps(
                [script, ink], sort_keys=True, allow_nan=False, separators=(",", ":")
            ).encode()
        ).hexdigest()
        if key in unique:
            old_name, old = unique[key]
            old_label = (old.get("expected"), old.get("expected_invalid", False), writing_validity(old))
            if old_label != (expected, invalid, validity):
                raise ValueError(
                    f"conflicting annotations for identical ink: {old_name}, {name}"
                )
            duplicates += 1
        else:
            unique[key] = (name, {**doc, "script": script})
    return list(unique.values()), duplicates


def fraction(numerator, denominator):
    return numerator / denominator if denominator else None


def summarize(documents, outputs, duplicates=0):
    if len(documents) != len(outputs):
        raise ValueError("replay output count mismatch")
    fingerprints = {out["vision"]["model_fingerprint"] for out in outputs}
    if len(fingerprints) != 1:
        raise ValueError("expected exactly one replay model")
    report = {
        "report_version": 2,
        "model_fingerprint": next(iter(fingerprints)),
        "unique_samples": len(documents),
        "duplicate_samples_skipped": duplicates,
        "note": "Expected kana labels represent intended identity, not verified correct strokes. Legacy kana-labelled captures default to unknown writing validity. Validity values are supplied annotations, never inferred from classifier agreement. No independent writer accuracy or calibrated acceptance claim.",
        "scripts": {},
    }
    for script, inventory in INVENTORIES.items():
        rows = [
            (name, doc, out)
            for (name, doc), out in zip(documents, outputs, strict=True)
            if doc["script"] == script
        ]
        if not rows:
            continue
        counts = Counter()
        per_class = defaultdict(Counter)
        bins = [Counter() for _ in range(10)]
        failures = []
        by_validity = defaultdict(Counter)
        for name, doc, out in rows:
            if out["script"] != script:
                raise ValueError(f"{name}: replay script mismatch")
            expected = doc.get("expected")
            invalid = doc.get("expected_invalid", False)
            validity = writing_validity(doc)
            counts["samples"] += 1
            counts["writing_validity_" + validity] += 1
            if expected is None and not invalid:
                counts["unlabeled"] += 1
                continue
            counts["labeled"] += 1
            g, v = out["geometry"], out["vision"]
            gc = [c["character"] for c in g["candidates"]]
            vc = [c["character"] for c in v["candidates"]]
            accepted = g["state"] == "recognized"
            agreed = bool(gc and vc and gc[0] == vc[0])
            g_correct = bool(expected and gc and gc[0] == expected)
            v_correct = bool(expected and vc and vc[0] == expected)
            counts["geometry_accepted"] += accepted
            counts["geometry_accepted_correct"] += accepted and g_correct
            counts["agreement"] += agreed
            counts["agreement_correct"] += agreed and v_correct
            counts["vision_errors"] += v["error"] is not None
            if invalid:
                counts["invalid"] += 1
                counts["invalid_geometry_rejected"] += not accepted
                counts["invalid_vision_score_at_least_0_9"] += bool(
                    vc and v["candidates"][0]["score"] >= 0.9
                )
            else:
                counts["intent_labeled"] += 1
                by_validity[validity]["samples"] += 1
                by_validity[validity]["vision_intent_matches"] += v_correct
                by_validity[validity]["geometry_intent_matches"] += g_correct
                counts["vision_top1"] += v_correct
                counts["vision_top5"] += expected in vc[:5]
                counts["geometry_top1"] += g_correct
                counts["geometry_top5"] += expected in gc[:5]
                counts["only_vision_correct"] += v_correct and not g_correct
                counts["only_geometry_correct"] += g_correct and not v_correct
                per_class[expected]["samples"] += 1
                per_class[expected]["vision_correct"] += v_correct
                per_class[expected]["geometry_correct"] += g_correct
                if vc:
                    score = v["candidates"][0]["score"]
                    if not 0 <= score <= 1:
                        raise ValueError(f"{name}: invalid vision score")
                    bucket = bins[min(int(score * 10), 9)]
                    bucket["samples"] += 1
                    bucket["correct"] += v_correct
                    bucket["score_sum"] += score
            if (invalid and accepted) or (
                expected and (not v_correct or not g_correct)
            ):
                failures.append(
                    {
                        "file": name,
                        "expected": expected,
                        "expected_invalid": invalid,
                        "writing_validity": validity,
                        "geometry_top1": gc[0] if gc else None,
                        "geometry_accepted": accepted,
                        "vision_top1": vc[0] if vc else None,
                        "vision_score": v["candidates"][0]["score"] if vc else None,
                    }
                )
        report["scripts"][script] = {
            "intent_recovery_by_writing_validity": {k: dict(v) for k, v in sorted(by_validity.items())},
            "counts": dict(sorted(counts.items())),
            "class_coverage": len(per_class),
            "missing_labels": "".join(c for c in inventory if c not in per_class),
            "vision_intended_top1_rate": fraction(counts["vision_top1"], counts["intent_labeled"]),
            "vision_intended_top5_rate": fraction(counts["vision_top5"], counts["intent_labeled"]),
            "geometry_intended_top1_rate": fraction(counts["geometry_top1"], counts["intent_labeled"]),
            "geometry_intended_top5_rate": fraction(counts["geometry_top5"], counts["intent_labeled"]),
            "geometry_accepted_intent_match_rate_including_non_kana": fraction(
                counts["geometry_accepted_correct"], counts["geometry_accepted"]
            ),
            "agreement_intent_match_rate_including_non_kana": fraction(
                counts["agreement_correct"], counts["agreement"]
            ),
            "macro_vision_intent_match_observed_classes_only": fraction(
                sum(c["vision_correct"] / c["samples"] for c in per_class.values()),
                len(per_class),
            ),
            "per_class": dict(sorted(per_class.items())),
            "intent_label_score_bins": [
                {
                    "lower": i / 10,
                    "upper": (i + 1) / 10,
                    "samples": b["samples"],
                    "intent_match_rate": fraction(b["correct"], b["samples"]),
                    "mean_score": fraction(b["score_sum"], b["samples"]),
                }
                for i, b in enumerate(bins)
            ],
            "failures": failures,
        }
    return report


def self_test():
    def sample(label=None, invalid=False, x=0):
        return {
            "script": "hiragana",
            "expected": label,
            "expected_invalid": invalid,
            "strokes": [[{"x": x, "y": 0}, {"x": x + 1, "y": 1}]],
        }

    docs = [
        ("correct", sample("あ")),
        ("wrong", sample("い", x=1)),
        ("invalid", sample(invalid=True, x=2)),
        ("unlabeled", sample(x=3)),
    ]
    unique, duplicates = unique_samples(docs + [docs[0]])
    assert len(unique) == 4 and duplicates == 1
    out = {
        "script": "hiragana",
        "geometry": {"state": "recognized", "candidates": [{"character": "あ"}]},
        "vision": {
            "model_fingerprint": "test",
            "error": None,
            "candidates": [{"character": "あ", "score": 0.99}],
        },
    }
    report = summarize(unique, [out] * 4, duplicates)["scripts"]["hiragana"]
    assert report["vision_intended_top1_rate"] == 0.5
    assert report["agreement_intent_match_rate_including_non_kana"] == 1 / 3
    assert report["geometry_accepted_intent_match_rate_including_non_kana"] == 1 / 3
    assert report["intent_label_score_bins"][9]["samples"] == 2
    assert report["intent_label_score_bins"][9]["intent_match_rate"] == 0.5
    assert report["counts"]["invalid_vision_score_at_least_0_9"] == 1
    assert report["counts"]["unlabeled"] == 1
    assert "valid" not in report["counts"]
    assert report["counts"]["writing_validity_unknown"] == 3
    malformed = [("malformed", {**sample("あ"), "writing_validity": "invalid"})]
    result = summarize(malformed, [out])["scripts"]["hiragana"]
    assert result["vision_intended_top1_rate"] == 1
    assert result["intent_recovery_by_writing_validity"]["invalid"]["vision_intent_matches"] == 1
    assert writing_validity(sample("あ")) == "unknown"
    for bad in (
        docs + [("conflict", sample("い"))],
        [("bad", sample("が"))],
        [("both", sample("あ", True))],
        [("bad-quality", {**sample("あ"), "writing_validity": "perfect"})],
        docs + [("quality-conflict", {**sample("あ"), "writing_validity": "valid"})],
    ):
        try:
            unique_samples(bad)
        except ValueError:
            pass
        else:
            raise AssertionError("accepted conflicting or invalid annotation")
    print("Paired corpus accounting tests passed")


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument("corpus", nargs="*", type=Path)
    p.add_argument("--model", type=Path)
    p.add_argument("--output", type=Path)
    p.add_argument("--self-test", action="store_true")
    a = p.parse_args()
    if a.self_test:
        self_test()
        return
    if not a.corpus:
        p.error("provide one or more extracted corpus directories or sample JSON files")
    paths = sorted(
        {
            f
            for path in a.corpus
            for f in (path.rglob("*.json") if path.is_dir() else [path])
        }
    )
    documents, duplicates = unique_samples(
        [(str(path), json.loads(path.read_text())) for path in paths]
    )
    if not documents:
        p.error("empty corpus")
    outputs = json.loads(
        subprocess.check_output(
            [
                str(ROOT / "target/release/kana-vision"),
                *(["--model", str(a.model)] if a.model else []),
            ],
            input=json.dumps([doc for _, doc in documents]).encode(),
        )
    )
    report = summarize(documents, outputs, duplicates)
    encoded = json.dumps(report, ensure_ascii=False, indent=2) + "\n"
    if a.output:
        a.output.write_text(encoded)
    print(encoded, end="")


if __name__ == "__main__":
    main()
