#!/usr/bin/env python3
"""Check frozen trial gates and native exports; never installs a candidate."""
import argparse
import hashlib
import importlib.util
import json
from pathlib import Path
import subprocess

ROOT = Path(__file__).resolve().parent.parent
spec = importlib.util.spec_from_file_location("promotion", Path(__file__).with_name("check-kana-promotion.py"))
promotion = importlib.util.module_from_spec(spec)
spec.loader.exec_module(promotion)


def logits(inputs, model=None):
    command = [str(ROOT / "target/release/kana-rasterize"), "--logits"]
    if model:
        command += ["--model", str(model)]
    return json.loads(subprocess.check_output(command, input=json.dumps(inputs).encode()))


def canonical_metrics(inputs, model=None):
    scores = logits(inputs, model)
    both = sum(max(range(92), key=lambda j: row[j]) == i for i, row in enumerate(scores))
    known = sum(max(range(0 if i < 46 else 46, 46 if i < 46 else 92), key=lambda j: row[j]) == i for i, row in enumerate(scores))
    return dict(canonical_known_top1=known, canonical_both_top1=both)


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument("run", type=Path)
    p.add_argument("--baseline-validation", type=Path, required=True)
    p.add_argument("--baseline-model", type=Path, required=True,
                   help="Explicit frozen baseline binary; never silently use newly promoted weights")
    p.add_argument("--baseline-audit", type=Path, required=True)
    p.add_argument("--candidate-audit", type=Path, required=True)
    a = p.parse_args()
    output = a.run / "gates.json"
    if output.exists():
        p.error("Never overwrite a gate report")
    report = json.loads((a.run / "report.json").read_text())
    model = a.run / "model.bin"
    assert hashlib.sha256(model.read_bytes()).hexdigest() == report["model_sha256"]
    golden = json.loads((a.run / "synthetic-golden.json").read_text())
    actual = logits(golden["input"], model)
    delta = max(abs(x-y) for row, reference in zip(actual, golden["logits"], strict=True) for x,y in zip(row, reference, strict=True))
    assert delta < .0002
    canonical = json.loads(subprocess.check_output([str(ROOT / "target/release/kana-vision"), "--canonical", "--rasters"]))
    inputs = [row["raster"] for row in canonical]
    baseline_reference, candidate_reference = canonical_metrics(inputs, a.baseline_model), canonical_metrics(inputs, model)
    baseline = json.loads(a.baseline_validation.read_text())
    def view(values, preparation, reference):
        return dict(results={"proxy_images": {key.split(":")[1]: value for key, value in values.items() if key.startswith(preparation + ":")}}, **reference)
    gates = promotion.assess(view(baseline, "clean", baseline_reference), view(baseline, "original", baseline_reference),
                             view(report["validation"], "clean", candidate_reference), view(report["validation"], "original", candidate_reference))
    old, new = [json.loads((directory / "summary.json").read_text()) for directory in [a.baseline_audit, a.candidate_audit]]
    assert old["corpus_sha256"] == new["corpus_sha256"]
    assert old["model_sha256"] == hashlib.sha256(a.baseline_model.read_bytes()).hexdigest()
    assert new["model_sha256"] == report["model_sha256"]
    stroke_failures = []
    for source in ["embedded", "kanjivg"]:
        if new["groups"][source + ":valid_control"]["vision"]["parent_top1"] < .99:
            stroke_failures.append(f"{source} positive-control top1 below 99 percent")
    negative = "procedural:non_kana"
    if new["groups"][negative]["vision"]["high_confidence_rates"]["0.99"] > old["groups"][negative]["vision"]["high_confidence_rates"]["0.99"]:
        stroke_failures.append("More >=99-percent-softmax non-kana probes")
    result = dict(model_sha256=report["model_sha256"], baseline_model_sha256=old["model_sha256"],
        etl_gates=gates, stroke_failures=stroke_failures, positive_controls={s: new["groups"][s + ":valid_control"]["vision"]["parent_top1"] for s in ["embedded", "kanjivg"]},
        baseline_reference=baseline_reference, candidate_reference=candidate_reference,
        native_logit_delta=delta, model_bytes=model.stat().st_size,
        eligible_for_confirmation=gates["validation_gates_passed"] and not stroke_failures and model.stat().st_size < 10_000_000,
        note="Eligibility for another seed and further checks only; not a modern pen accuracy or validity claim. No deployment.")
    output.write_text(json.dumps(result, indent=2) + "\n")
    print(json.dumps(result, indent=2))


if __name__ == "__main__":
    main()
