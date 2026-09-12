#!/usr/bin/env python3
"""Project a verified audit onto an exact subset corpus without rerunning inference."""
import argparse
import hashlib
import importlib.util
import json
from pathlib import Path

spec=importlib.util.spec_from_file_location("audit",Path(__file__).with_name("check-kana-beginner.py"))
audit=importlib.util.module_from_spec(spec)
spec.loader.exec_module(audit)


def corpus(path):
    raw=(path/"samples.jsonl").read_bytes()
    manifest=json.loads((path/"manifest.json").read_text())
    assert hashlib.sha256(raw).hexdigest()==manifest["samples_sha256"]
    rows=[json.loads(line) for line in raw.splitlines()]
    assert len(rows)==manifest["samples"] and len({r["id"] for r in rows})==len(rows)
    return manifest,rows


def main():
    p=argparse.ArgumentParser(description=__doc__)
    p.add_argument("--source-corpus",type=Path,required=True)
    p.add_argument("--source-audit",type=Path,required=True)
    p.add_argument("--subset-corpus",type=Path,required=True)
    p.add_argument("--output",type=Path,required=True)
    a=p.parse_args()
    if a.output.exists():
        p.error("Never overwrite an audit")
    full_manifest,full=corpus(a.source_corpus)
    subset_manifest,subset=corpus(a.subset_corpus)
    by_id={r["id"]:r for r in full}
    assert all(by_id.get(r["id"])==r for r in subset),"Subset ink or metadata differs"
    source_summary=json.loads((a.source_audit/"summary.json").read_text())
    assert source_summary["corpus_sha256"]==full_manifest["samples_sha256"]
    raw=(a.source_audit/"predictions.jsonl").read_bytes()
    predictions=[json.loads(line) for line in raw.splitlines()]
    assert len(predictions)==len(full) and len({r["id"] for r in predictions})==len(full)
    for r in predictions:
        original=by_id[r["id"]]
        assert all(r[k]==v for k,v in original.items() if k!="strokes")
    index={r["id"]:r for r in predictions}
    selected=[index[r["id"]] for r in subset]
    summary={k:v for k,v in source_summary.items() if k not in ["groups","per_character","corpus_sha256"]}
    summary.update(corpus_sha256=subset_manifest["samples_sha256"],groups=audit.summarize(selected),
        per_character={c:audit.summarize([r for r in selected if r["parent_character"]==c])
                       for c in sorted({r["parent_character"] for r in selected if r["parent_character"]})},
        projection=dict(source_corpus_sha256=full_manifest["samples_sha256"],
                        source_predictions_sha256=hashlib.sha256(raw).hexdigest(),
                        script_sha256=hashlib.sha256(Path(__file__).read_bytes()).hexdigest()))
    a.output.mkdir(parents=True)
    (a.output/"predictions.jsonl").write_text("".join(json.dumps(r,ensure_ascii=False,separators=(",",":"))+"\n" for r in selected))
    (a.output/"summary.json").write_text(json.dumps(summary,ensure_ascii=False,indent=2)+"\n")
    (a.output/"projection.py").write_bytes(Path(__file__).read_bytes())
    print(f"Projected {len(selected)} exact samples from {len(full)} audited rows")


if __name__=="__main__":
    main()
