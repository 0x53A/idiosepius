#!/usr/bin/env python3
"""Build an offline, initially blind annotation queue for repair hypotheses.

Reviews are downloaded separately, bound to exact ink hashes. No annotations
are invented and no source corpus, study database or model is changed.
"""
import argparse
import hashlib
import json
from pathlib import Path
import random


def encode(value):
    return json.dumps(value,ensure_ascii=False,sort_keys=True,separators=(",",":"),allow_nan=False)


def main():
    p=argparse.ArgumentParser(description=__doc__)
    p.add_argument("--corpus",type=Path,required=True)
    p.add_argument("--repairs",type=Path,required=True)
    p.add_argument("--output",type=Path,required=True)
    a=p.parse_args()
    if a.output.exists():
        p.error("Never overwrite a review queue")
    raw=(a.corpus/"samples.jsonl").read_bytes()
    manifest=json.loads((a.corpus/"manifest.json").read_text())
    assert hashlib.sha256(raw).hexdigest()==manifest["samples_sha256"]
    rows={r["id"]:r for r in map(json.loads,raw.splitlines())}
    repair_raw=(a.repairs/"predictions.jsonl").read_bytes()
    repair_summary=json.loads((a.repairs/"summary.json").read_text())
    assert repair_summary["corpus_sha256"]==manifest["samples_sha256"]
    predictions=list(map(json.loads,repair_raw.splitlines()))
    assert len(predictions)==len(rows)
    disagreements=[r for r in predictions if r["possible_missing_stroke"] and r["repair_candidates"][0]["character"]!=r["parent_character"]]
    same_parent=[r for r in predictions if r["source"]!="embedded" and r["possible_missing_stroke"] and r["repair_candidates"][0]["character"]==r["parent_character"]]
    controls=[r for r in predictions if r["source"]!="embedded" and r["kind"]=="valid_control" and r["operation"]=="canonical"]
    rng=random.Random(20260916)
    rng.shuffle(same_parent);rng.shuffle(controls)
    hashes={key:hashlib.sha256(encode(dict(script=r["script"],strokes=r["strokes"])).encode()).hexdigest() for key,r in rows.items()}
    aliases={}
    for key,value in hashes.items():
        aliases.setdefault(value,[]).append(key)
    seen=set()
    def unique_group(candidates,limit):
        result=[]
        for candidate in candidates:
            digest=hashes[candidate["id"]]
            if digest in seen:
                continue
            seen.add(digest)
            result.append(candidate)
            if len(result)==limit:
                break
        return result
    unique_disagreements=unique_group(disagreements,len(disagreements))
    selected_same=unique_group(same_parent,20)
    selected_controls=unique_group(controls,20)
    selected=unique_disagreements+selected_same+selected_controls
    rng.shuffle(selected)
    tasks=[]
    for prediction in selected:
        row=rows[prediction["id"]]
        ink_hash=hashes[row["id"]]
        tasks.append(dict(id=row["id"],ink_sha256=ink_hash,strokes=row["strokes"],script=row["script"],
            source_alias_ids=aliases[ink_hash],
            hypothesis=dict(parent=row["parent_character"],operation=row["operation"],
                proposed=prediction["repair_candidates"][0],flagged=prediction["possible_missing_stroke"],
                source=row["source"],kind=row["kind"])))
    payload=encode(tasks)
    queue_hash=hashlib.sha256(payload.encode()).hexdigest()
    html=PAGE.replace("__TASKS__",payload.replace("<","\\u003c")).replace("__QUEUE_HASH__",queue_hash)
    a.output.mkdir(parents=True)
    (a.output/"index.html").write_text(html)
    (a.output/"tasks.json").write_text(payload+"\n")
    (a.output/"manifest.json").write_text(json.dumps(dict(queue_sha256=queue_hash,tasks=len(tasks),
        source_corpus_sha256=manifest["samples_sha256"],repair_predictions_sha256=hashlib.sha256(repair_raw).hexdigest(),
        raw_flagged_disagreement_count=len(disagreements),
        deduplication="Exact script plus raw stroke coordinates; all source aliases retained. Different point sampling or equivalent rasters are not merged.",
        sampled_groups=dict(flagged_disagreements=len(unique_disagreements),flagged_same_parent=len(selected_same),canonical_controls=len(selected_controls)),
        note="Review queue, not an accuracy sample. Hypotheses hidden until requested. No prefilled quality labels. Reviewer judgments need provenance and may remain ambiguous."),indent=2)+"\n")
    (a.output/"generator.py").write_bytes(Path(__file__).read_bytes())
    print(f"Wrote {len(tasks)} tasks to {a.output}/index.html")


PAGE='''<!doctype html><meta charset="utf-8"><meta name="viewport" content="width=device-width">
<title>Kana writing review</title>
<style>
body{background:#060a0d;color:#d8e6ea;font:15px monospace;max-width:760px;margin:32px auto;padding:16px}
button,input,select,textarea{font:inherit;color:inherit;background:#0b1216;border:1px solid #74919a;padding:8px;border-radius:0}
button{cursor:pointer}button:disabled{opacity:.4}label{display:block;margin:14px 0}input,textarea{max-width:95%}
svg{display:block;border:1px solid #74919a;background:#0b1216;width:280px;height:280px;margin:16px 0}
small,p{color:#9bb4bd;line-height:1.5}nav{display:flex;gap:12px;margin:16px 0}textarea{width:95%;height:65px}
</style>
<h1>W R I T I N G · R E V I E W</h1>
<p>Judge the ink before revealing the source or proposed repair. A different valid kana is not malformed writing. Leave uncertain cases ambiguous. This queue mixes controls and repair probes; it is not a population accuracy sample.</p>
<label>Reviewer identifier <input id="reviewer" placeholder="An alias is sufficient"></label>
<div id="position"></div><svg id="ink" viewBox="0 0 100 100" aria-label="Sample ink"></svg>
<label>Writing form <select id="validity"><option value="unknown">Unreviewed</option><option value="valid">Valid kana form</option><option value="invalid">Malformed form</option><option value="ambiguous">Ambiguous / cannot judge</option></select></label>
<label>Drawn kana, if identifiable <input id="written" autocomplete="off"></label>
<label>Possible intended kana, if inferable <input id="intended" autocomplete="off"></label>
<label>Reason / missing or altered feature <textarea id="reason"></textarea></label>
<button id="reveal">Reveal source and hypothesis</button><pre id="hypothesis" hidden></pre>
<nav><button id="previous">Previous</button><button id="next">Next</button></nav>
<p id="storage"></p><button id="export">Download review JSON</button>
<p>Progress stays in this browser when local storage is available. Download before moving the review elsewhere. Original ink and corpora are never edited.</p>
<script>
const tasks=__TASKS__,queueHash="__QUEUE_HASH__",key="kana-review:"+queueHash;
const $=id=>document.getElementById(id);let position=0,reviews={},storageAvailable=true;
try{reviews=JSON.parse(localStorage.getItem(key)||"{}");if(!reviews||typeof reviews!=="object"||Array.isArray(reviews))reviews={}}catch{storageAvailable=false}
function persist(){try{localStorage.setItem(key,JSON.stringify(reviews))}catch{storageAvailable=false}
  $("storage").textContent=storageAvailable?"Progress saved locally.":"Local storage unavailable. Download to retain your review."}
function save(){const task=tasks[position],old=reviews[task.id]||{};
  reviews[task.id]={id:task.id,ink_sha256:task.ink_sha256,writing_validity:$("validity").value,
    written_character:$("written").value.trim()||null,intended_character:$("intended").value.trim()||null,
    reason:$("reason").value,reviewer:$("reviewer").value.trim()||null,hypothesis_revealed:!!old.hypothesis_revealed};persist()}
function show(){const task=tasks[position],r=reviews[task.id]||{};
  $("position").textContent=`${position+1} / ${tasks.length} · ${task.script}`;
  $("ink").replaceChildren();for(const stroke of task.strokes){const line=document.createElementNS("http://www.w3.org/2000/svg","polyline");
    line.setAttribute("points",stroke.map(p=>`${p.x},${p.y}`).join(" "));line.setAttribute("fill","none");line.setAttribute("stroke","#d8e6ea");line.setAttribute("stroke-width","1.6");$("ink").append(line)}
  $("reviewer").value=r.reviewer||$("reviewer").value||"";
  $("validity").value=r.writing_validity||"unknown";$("written").value=r.written_character||"";$("intended").value=r.intended_character||"";$("reason").value=r.reason||"";
  $("hypothesis").hidden=!r.hypothesis_revealed;$("hypothesis").textContent=r.hypothesis_revealed?JSON.stringify(task.hypothesis,null,2):"";
  $("previous").disabled=position===0;$("next").disabled=position===tasks.length-1}
for(const id of ["reviewer","validity","written","intended","reason"])$(id).oninput=save;
$("previous").onclick=()=>{save();position--;show()};$("next").onclick=()=>{save();position++;show()};
$("reveal").onclick=()=>{save();reviews[tasks[position].id].hypothesis_revealed=true;persist();show()};
$("export").onclick=()=>{save();const data={format:"idiosepius-kana-review",format_version:1,queue_sha256:queueHash,
  note:"Supplied judgments, not automatically verified labels. Unknown and ambiguous are not valid/invalid training labels.",reviews:Object.values(reviews)};
  const url=URL.createObjectURL(new Blob([JSON.stringify(data,null,2)],{type:"application/json"})),a=document.createElement("a");a.href=url;a.download="kana-review.json";a.click();setTimeout(()=>URL.revokeObjectURL(url),1000)};
show();
</script>'''


if __name__=="__main__":
    main()
