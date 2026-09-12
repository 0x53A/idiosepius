#!/usr/bin/env python3
"""Validate hash-bound review sidecars and summarize supplied judgments.

This never edits the corpus, trains a model, or certifies reviewer expertise.
Unknown and ambiguous rows are excluded from definite-validity denominators.
"""
import argparse
from collections import Counter
import hashlib
import json
from pathlib import Path
import unicodedata


def encode(value):
    return json.dumps(value,ensure_ascii=False,sort_keys=True,separators=(",",":"),allow_nan=False)


def digest(value):
    return hashlib.sha256(encode(value).encode()).hexdigest()


def validate(tasks, queue_hash, document):
    if digest(tasks)!=queue_hash:
        raise ValueError("Queue contents do not match queue hash")
    index={task["id"]:task for task in tasks}
    if len(index)!=len(tasks):
        raise ValueError("Duplicate task ids")
    for task in tasks:
        if digest(dict(script=task["script"],strokes=task["strokes"]))!=task["ink_sha256"]:
            raise ValueError("Task ink hash mismatch")
    if document.get("format")!="idiosepius-kana-review" or document.get("format_version")!=1:
        raise ValueError("Unsupported review format")
    if document.get("queue_sha256")!=queue_hash:
        raise ValueError("Review belongs to a different queue")
    if not isinstance(document.get("reviews"),list):
        raise ValueError("Missing reviews array")
    result=[];seen=set()
    for row in document["reviews"]:
        key=row.get("id")
        if key not in index or key in seen:
            raise ValueError("Unknown or repeated reviewed task")
        seen.add(key);task=index[key]
        if row.get("ink_sha256")!=task["ink_sha256"]:
            raise ValueError("Review ink differs from queued ink")
        validity=row.get("writing_validity")
        if validity not in ("unknown","ambiguous","valid","invalid"):
            raise ValueError("Unsupported writing validity")
        reviewer=row.get("reviewer")
        if reviewer is not None and not isinstance(reviewer,str):
            raise ValueError("Reviewer must be an alias string")
        if validity!="unknown" and not (reviewer and reviewer.strip()):
            raise ValueError("Judgments require a reviewer identifier")
        if not isinstance(row.get("hypothesis_revealed"),bool):
            raise ValueError("Missing hypothesis reveal provenance")
        reason=row.get("reason","")
        if not isinstance(reason,str) or (validity=="invalid" and not reason.strip()):
            raise ValueError("Invalid judgments require a reason")
        normalized=dict(row)
        for field in ("written_character","intended_character"):
            value=row.get(field)
            if value is not None:
                if not isinstance(value,str):
                    raise ValueError(f"{field} must be one character or null")
                value=unicodedata.normalize("NFC",value.strip())
                if len(value)!=1:
                    raise ValueError(f"{field} must be one character or null; leave ambiguous identity unset")
            normalized[field]=value
        normalized["source_alias_ids"]=task.get("source_alias_ids",[key])
        result.append(normalized)
    return result


def summarize(tasks, reviews):
    index={task["id"]:task for task in tasks}
    counts=Counter();strata={"blind":Counter(),"hypothesis_seen":Counter()}
    for row in reviews:
        validity=row["writing_validity"]
        counts[validity]+=1
        bucket=strata["hypothesis_seen" if row["hypothesis_revealed"] else "blind"]
        bucket[validity]+=1
        if validity in ("valid","invalid"):
            bucket["flagged_"+validity]+=bool(index[row["id"]]["hypothesis"]["flagged"])
    return dict(queue_samples=len(tasks),exported_rows=len(reviews),missing_rows=len(tasks)-len(reviews),
        supplied_validity_counts=dict(counts),by_reveal_status={k:dict(v) for k,v in strata.items()},
        note="Selected diagnostic queue and supplied judgments, not population accuracy or certified ground truth. Unknown, ambiguous and missing rows are not valid/invalid labels. Reveal status is supplied provenance, not proof of reviewer blinding.")


def self_test():
    import copy
    task=dict(id="fixture",script="hiragana",strokes=[[dict(x=0,y=0),dict(x=1,y=1)]],hypothesis=dict(flagged=True))
    task["ink_sha256"]=digest(dict(script=task["script"],strokes=task["strokes"]))
    tasks=[task];qh=digest(tasks)
    row=dict(id="fixture",ink_sha256=task["ink_sha256"],writing_validity="ambiguous",reviewer="fixture",
             reason="",hypothesis_revealed=False,written_character=None,intended_character=None)
    doc=dict(format="idiosepius-kana-review",format_version=1,queue_sha256=qh,reviews=[row])
    result=summarize(tasks,validate(tasks,qh,doc))
    assert result["by_reveal_status"]["blind"]==dict(ambiguous=1)
    for mutate in [lambda d:d.update(queue_sha256="wrong"),
                   lambda d:d["reviews"][0].update(ink_sha256="wrong"),
                   lambda d:d["reviews"].append(dict(row)),
                   lambda d:d["reviews"][0].update(reviewer=None),
                   lambda d:d["reviews"][0].update(writing_validity="invalid",reason=""),
                   lambda d:d["reviews"][0].update(written_character="あい")]:
        bad=copy.deepcopy(doc);mutate(bad)
        try:validate(tasks,qh,bad)
        except ValueError:pass
        else:raise AssertionError("Accepted invalid review binding/provenance")
    doc["reviews"][0].update(writing_validity="invalid",reason="missing stroke")
    assert summarize(tasks,validate(tasks,qh,doc))["by_reveal_status"]["blind"]==dict(invalid=1,flagged_invalid=1)
    print("Review checks passed: hash binding, duplicates, provenance, reason, single identity and ambiguity exclusion")


def main():
    p=argparse.ArgumentParser(description=__doc__)
    p.add_argument("--queue",type=Path)
    p.add_argument("--reviews",type=Path)
    p.add_argument("--output",type=Path)
    p.add_argument("--self-test",action="store_true")
    a=p.parse_args()
    if a.self_test:self_test();return
    if not all((a.queue,a.reviews,a.output)) or a.output.exists():
        p.error("Provide --queue, --reviews and a new --output directory")
    manifest=json.loads((a.queue/"manifest.json").read_text())
    tasks=json.loads((a.queue/"tasks.json").read_text())
    assert len(tasks)==manifest["tasks"]
    raw=a.reviews.read_bytes();doc=json.loads(raw)
    normalized=validate(tasks,manifest["queue_sha256"],doc)
    report=summarize(tasks,normalized)
    report.update(queue_sha256=manifest["queue_sha256"],review_sha256=hashlib.sha256(raw).hexdigest())
    a.output.mkdir(parents=True)
    (a.output/"summary.json").write_text(json.dumps(report,indent=2)+"\n")
    (a.output/"annotations.json").write_text(json.dumps(normalized,ensure_ascii=False,indent=2)+"\n")
    (a.output/"original-review.json").write_bytes(raw)
    (a.output/"validator.py").write_bytes(Path(__file__).read_bytes())
    print(json.dumps(report,indent=2))


if __name__=="__main__":
    main()
