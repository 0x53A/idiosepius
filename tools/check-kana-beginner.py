#!/usr/bin/env python3
"""Audit both deployed classifiers on a frozen beginner corpus, without labels as inputs."""
import argparse
from collections import defaultdict
import hashlib
import json
from pathlib import Path
import subprocess

ROOT = Path(__file__).resolve().parent.parent


def summarize(rows):
    groups = defaultdict(list)
    for row in rows:
        groups[f"{row['source']}:{row['kind']}"].append(row)
    output = {}
    for key, values in groups.items():
        count = len(values)
        entry = {"samples": count}
        for engine in ["vision", "geometry"]:
            predictions = [v[engine] for v in values]
            entry[engine] = {
                "parent_top1": sum(bool(p["candidates"]) and p["candidates"][0]["character"] == v["parent_character"] for p, v in zip(predictions, values)) / count if values[0]["parent_character"] else None,
                "parent_top5": sum(v["parent_character"] in [c["character"] for c in p["candidates"][:5]] for p, v in zip(predictions, values)) / count if values[0]["parent_character"] else None,
            }
            if engine == "geometry":
                accepted = [p["state"] == "recognized" for p in predictions]
                entry[engine]["acceptance_rate"] = sum(accepted) / count
                entry[engine]["accepted_other_than_parent"] = sum(a and p["candidates"][0]["character"] != v["parent_character"] for a, p, v in zip(accepted, predictions, values)) / count if values[0]["parent_character"] else None
            else:
                entry[engine]["high_confidence_rates"] = {str(t): sum(bool(p["candidates"]) and p["candidates"][0]["score"] >= t for p in predictions) / count for t in [.5, .9, .99, .999]}
            if "requested_answer" in values[0]:
                returns_requested = [bool(p["candidates"]) and p["candidates"][0]["character"] == v["requested_answer"] for p, v in zip(predictions, values)]
                entry[engine]["returned_requested_instead_of_drawn"] = sum(returns_requested)
                if engine == "geometry":
                    entry[engine]["accepted_requested_instead_of_drawn"] = sum(a and b for a, b in zip(accepted, returns_requested))
                else:
                    entry[engine]["requested_with_score_at_least_0_99"] = sum(b and p["candidates"][0]["score"] >= .99 for b, p in zip(returns_requested, predictions))
        output[key] = entry
    return output


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument("corpus", type=Path)
    p.add_argument("--model", type=Path)
    p.add_argument("--output", type=Path, required=True)
    a = p.parse_args()
    if a.output.exists():
        p.error("Never overwrite an evaluation")
    manifest = json.loads((a.corpus / "manifest.json").read_text())
    raw = (a.corpus / "samples.jsonl").read_bytes()
    assert hashlib.sha256(raw).hexdigest() == manifest["samples_sha256"]
    samples = [json.loads(line) for line in raw.splitlines()]
    assert len(samples) == manifest["samples"]
    a.output.mkdir(parents=True)
    command = [str(ROOT / "target/release/kana-vision")]
    if a.model:
        command.extend(["--model", str(a.model)])
    output = []
    with (a.output / "predictions.jsonl").open("w") as stream:
        for start in range(0, len(samples), 64):
            batch = samples[start:start + 64]
            # Deliberately omit expected, parent, mutation and validity metadata.
            payload = [{"strokes": r["strokes"], "script": r["script"]} for r in batch]
            predictions = json.loads(subprocess.check_output(command, input=json.dumps(payload).encode()))
            for sample, prediction in zip(batch, predictions, strict=True):
                assert prediction["expected"] is None
                row = {k: v for k, v in sample.items() if k != "strokes"}
                row.update({k: prediction[k] for k in ["vision", "geometry", "reference"]})
                stream.write(json.dumps(row, ensure_ascii=False, separators=(",", ":")) + "\n")
                output.append(row)
            print(f"Evaluated {len(output)}/{len(samples)}", flush=True)
    model = a.model or ROOT / "crates/core/assets/kana_vision.bin"
    summary = dict(corpus_sha256=manifest["samples_sha256"], model_sha256=hashlib.sha256(model.read_bytes()).hexdigest(),
        evaluator_sha256=hashlib.sha256(Path(__file__).read_bytes()).hexdigest(),
        executable_sha256=hashlib.sha256(Path(command[0]).read_bytes()).hexdigest(),
        groups=summarize(output),
        per_character={c: summarize([r for r in output if r["parent_character"] == c]) for c in sorted({r["parent_character"] for r in output if r["parent_character"]})},
        note="Parent recall on corruption probes is not recognition accuracy: the mutation may form another valid kana. CNN confidence thresholds are diagnostic sweeps, not acceptance rules. Only non_kana rows have asserted invalid labels.")
    (a.output / "summary.json").write_text(json.dumps(summary, ensure_ascii=False, indent=2) + "\n")
    print(json.dumps(summary["groups"], indent=2))


if __name__ == "__main__":
    main()
