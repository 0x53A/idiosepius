const $ = (id) => document.getElementById(id);
const canvas = $("ink");
const context = canvas.getContext("2d");
const worker = new Worker(new URL("./kana-worker.js", import.meta.url), { type: "module" });
const pending = new Map();
let requestId = 0;
function request(action, extra = {}) {
  return new Promise((resolve, reject) => {
    const id = ++requestId;
    const timer = setTimeout(() => {
      pending.delete(id);
      reject(new Error("Recognition worker timed out. Reload to try again; saved samples are retained."));
    }, 60000);
    pending.set(id, { resolve, reject, timer });
    worker.postMessage({ id, action, ...extra });
  });
}
worker.onmessage = ({ data }) => {
  const job = pending.get(data.id);
  if (!job) return;
  clearTimeout(job.timer);
  pending.delete(data.id);
  if (data.error) job.reject(new Error(data.error));
  else job.resolve(data.value);
};
worker.onerror = () => {
  for (const job of pending.values()) {
    clearTimeout(job.timer);
    job.reject(new Error("Could not run the recognition worker. Reload to try again."));
  }
  pending.clear();
};

let db, inventory = "", metadata, strokes = [], active = null, result = null, vision = null, reference = null;
let revision = 0, busy = false, run = null, savedCount = 0, downloadUrl;
const cancelledStrokes = new Set();
let nextRunTime = 0;
const script = () => $("script").value;
function notice(message, error = false) {
  $("notice").textContent = message;
  $("notice").className = error ? "wrong" : "";
}
function updateControls() {
  const locked = busy || active !== null || !metadata;
  for (const id of ["script", "start", "expected", "pen-only"]) $(id).disabled = locked;
  for (const id of ["clear", "undo"]) $(id).disabled = locked || !strokes.length;
  $("save").disabled = locked || !db || !result || !strokes.length;
  $("skip").disabled = locked || !run;
  $("export").disabled = busy || !db || savedCount === 0;
}
function draw() {
  context.clearRect(0, 0, 1024, 1024);
  context.strokeStyle = "#1e3037";
  context.lineWidth = 1;
  context.setLineDash([8, 12]);
  context.beginPath(); context.moveTo(512, 0); context.lineTo(512, 1024);
  context.moveTo(0, 512); context.lineTo(1024, 512); context.stroke();
  context.setLineDash([]);
  context.strokeStyle = "#d8e6ea";
  context.lineWidth = 5;
  context.lineCap = "square";
  context.lineJoin = "miter";
  for (const stroke of strokes) {
    if (!stroke.length) continue;
    context.beginPath(); context.moveTo(stroke[0].x, stroke[0].y);
    for (const point of stroke.slice(1)) context.lineTo(point.x, point.y);
    context.stroke();
    if (stroke.length === 1) context.fillRect(stroke[0].x - 2, stroke[0].y - 2, 4, 4);
  }
}
let drawQueued = false;
function scheduleDraw() {
  if (drawQueued) return;
  drawQueued = true;
  requestAnimationFrame(() => { drawQueued = false; draw(); });
}
function renderResult() {
  const verdict = $("verdict");
  verdict.className = "";
  $("candidates").replaceChildren();
  $("vision-candidates").replaceChildren();
  $("vision-verdict").textContent = "Draw to compare.";
  $("reference").textContent = "";
  if (vision && result) {
    $("vision-verdict").textContent = vision.error ? `Unavailable: ${vision.error}` : `Prediction: ${vision.candidates[0]?.character ?? "—"}`;
    for (const candidate of vision.candidates) {
      const row = document.createElement("li");
      row.textContent = candidate.character;
      const score = document.createElement("span");
      score.textContent = candidate.score.toFixed(3);
      row.append(score); $("vision-candidates").append(row);
    }
    if (reference) $("reference").textContent = `Compared with ${reference.character}: distance ${reference.symmetric_distance.toFixed(4)}, path length ${reference.path_length_ratio.toFixed(2)}× reference. This is similarity, not a quality grade.`;
  }
  if (!result) { verdict.textContent = active ? "Drawing…" : "Draw one kana in the square."; return; }
  const recognized = result.state === "recognized";
  const expected = $("expected").value.trim();
  const top = result.candidates[0]?.character;
  if (recognized) {
    verdict.textContent = `Recognized: ${top}`;
    if (expected) verdict.className = expected === top ? "correct" : "wrong";
  } else {
    const reason = result.state.unrecognized.reason.replaceAll("_", " ");
    verdict.textContent = `Not recognized: ${reason}.`;
  }
  for (const candidate of result.candidates.slice(0, 5)) {
    const row = document.createElement("li");
    row.textContent = candidate.character;
    const score = document.createElement("span");
    score.textContent = candidate.distance.toFixed(4);
    row.append(score); $("candidates").append(row);
  }
}
async function recognize() {
  result = null; vision = null; reference = null;
  renderResult();
  const current = ++revision;
  updateControls();
  if (!strokes.length) { renderResult(); return; }
  $("verdict").className = "";
  $("verdict").textContent = "Recognizing…";
  $("candidates").replaceChildren();
  try {
    const value = await request("recognize", { script: script(), strokes });
    if (current !== revision) return;
    result = value.geometry; vision = value.vision; reference = value.reference;
    renderResult(); updateControls();
  } catch (error) { if (current === revision) { notice(error.message, true); updateControls(); } }
}
function clear() {
  strokes = []; result = null; vision = null; reference = null; cancelledStrokes.clear(); ++revision;
  draw(); renderResult(); updateControls();
}
function append(event) {
  const rect = active.rect;
  strokes.at(-1).push({
    x: (event.clientX - rect.left) / rect.width * 1024,
    y: (event.clientY - rect.top) / rect.height * 1024,
    time_ms: event.timeStamp,
    // A mouse's standardized synthetic pressure is not a pressure measurement.
    pressure: event.pointerType === "mouse" ? null : event.pressure,
    pointer: ["pen", "touch", "mouse"].includes(event.pointerType) ? event.pointerType : "unknown",
  });
}
canvas.addEventListener("pointerdown", (event) => {
  if (active || busy || !metadata || event.button !== 0) return;
  if ($("pen-only").checked && event.pointerType !== "pen") return;
  const rect = canvas.getBoundingClientRect();
  active = { id: event.pointerId, rect: {
    left: rect.left + canvas.clientLeft, top: rect.top + canvas.clientTop,
    width: canvas.clientWidth, height: canvas.clientHeight,
  }};
  canvas.setPointerCapture(event.pointerId);
  strokes.push([]); ++revision; result = null; vision = null; reference = null;
  append(event); scheduleDraw(); renderResult(); updateControls();
  event.preventDefault();
});
canvas.addEventListener("pointermove", (event) => {
  if (active?.id !== event.pointerId) return;
  const samples = event.getCoalescedEvents?.() || [];
  // The coalesced list replaces the aggregate event, never duplicates it.
  for (const sample of samples.length ? samples : [event]) append(sample);
  scheduleDraw(); event.preventDefault();
});
function finish(event, cancelled) {
  if (active?.id !== event.pointerId) return;
  if (!cancelled) append(event);
  else cancelledStrokes.add(strokes.length - 1);
  active = null;
  if (cancelled) notice("The stroke was interrupted. Undo it if it is incomplete.");
  scheduleDraw(); recognize();
}
canvas.addEventListener("pointerup", (event) => finish(event, false));
canvas.addEventListener("pointercancel", (event) => finish(event, true));
canvas.addEventListener("lostpointercapture", (event) => finish(event, true));
canvas.addEventListener("contextmenu", (event) => event.preventDefault());

function showPrompt() {
  $("prompt").textContent = run ? run.sequence[run.position - 1] : "Free drawing";
  $("progress").textContent = run ? `${run.position} / ${run.sequence.length} · ${run.saved_before} saved` : "";
  if (run) $("expected").value = run.sequence[run.position - 1];
  updateControls();
}
function advance(saved) {
  if (!run) return;
  if (saved) run.saved_before++;
  run.position++;
  if (run.position > run.sequence.length) {
    notice(`Set complete: ${run.saved_before} saved, ${run.sequence.length - run.saved_before} skipped.`);
    run = null; $("expected").value = "";
  }
  clear(); showPrompt();
}
$("start").onclick = () => {
  const sequence = [...inventory];
  for (let i = sequence.length - 1; i > 0; --i) {
    const j = crypto.getRandomValues(new Uint32Array(1))[0] % (i + 1);
    [sequence[i], sequence[j]] = [sequence[j], sequence[i]];
  }
  nextRunTime = Math.max(Date.now(), nextRunTime + 1);
  run = { run_started_at_unix_ms: nextRunTime, position: 1, saved_before: 0, sequence };
  clear(); showPrompt(); notice("Write the displayed kana. Save advances to the next one.");
};
$("skip").onclick = () => advance(false);
$("clear").onclick = clear;
$("undo").onclick = () => { cancelledStrokes.delete(strokes.length - 1); strokes.pop(); scheduleDraw(); recognize(); };
$("expected").oninput = () => { run = null; showPrompt(); renderResult(); };
async function selectScript() {
  busy = true; metadata = null; run = null; $("expected").value = "";
  clear(); showPrompt();
  try {
    const value = await request("init", { script: script() });
    inventory = value.inventory; metadata = value.metadata;
  } catch (error) { notice(error.message, true); }
  finally { busy = false; updateControls(); }
}
$("script").onchange = selectScript;

function openStorage() {
  return new Promise((resolve, reject) => {
    const opening = indexedDB.open("idiosepius-kana", 1);
    opening.onupgradeneeded = () => opening.result.createObjectStore("samples", { autoIncrement: true });
    opening.onsuccess = () => resolve(opening.result);
    opening.onerror = () => reject(opening.error);
    opening.onblocked = () => reject(new Error("Close other kana tabs to open sample storage."));
  });
}
function transaction(mode, operation) {
  return new Promise((resolve, reject) => {
    const tx = db.transaction("samples", mode);
    const req = operation(tx.objectStore("samples"));
    tx.oncomplete = () => resolve(req.result);
    tx.onabort = tx.onerror = () => reject(tx.error || new Error("Sample storage failed"));
  });
}
async function storageCount() {
  savedCount = await transaction("readonly", (store) => store.count());
  $("storage").textContent = `${savedCount} samples saved in this browser.`;
  updateControls();
}
$("save").onclick = async () => {
  if (busy || active || !result || !db || !strokes.length) return;
  const expected = $("expected").value.trim();
  const invalid = expected === "-" || expected === "−";
  if (expected && !invalid && ([...expected].length !== 1 || !inventory.includes(expected))) {
    notice("Enter one kana from the selected script, leave blank, or use − for invalid ink.", true); return;
  }
  const points = strokes.flat();
  const times = points.map((p) => p.time_ms);
  const sample = {
    format: "idiosepius-kana-sample", format_version: 1, saved_at_unix_ms: Date.now(),
    ...(invalid ? { expected_invalid: true } : expected ? { expected } : {}),
    ...(expected ? { expected_source: run ? "prompted" : "manual" } : {}),
    writing_validity: invalid ? "invalid" : "unknown",
    ...(run ? { prompt: { ...run, total: run.sequence.length, sequence: run.sequence.join("") } } : {}),
    capture: {
      stroke_count: strokes.length, point_count: points.length, coordinate_extent: [1024, 1024],
      coordinate_origin: "top_left", point_time_ms_clock: "dom_event_timestamp",
      duration_ms: times.reduce((a, b) => Math.max(a, b), -Infinity) - times.reduce((a, b) => Math.min(a, b), Infinity),
      cancelled_strokes: cancelledStrokes.size, cancelled_stroke_indices: [...cancelledStrokes].sort((a, b) => a - b), source: "web_pointer_events", pen_only: $("pen-only").checked,
    },
    recognizer: metadata, strokes, result, vision, reference,
  };
  busy = true; updateControls();
  try {
    await transaction("readwrite", (store) => store.add(sample));
    notice("Sample saved.");
    savedCount++;
    $("storage").textContent = `${savedCount} samples saved in this browser.`;
    if (run) advance(true); else clear();
    try { await storageCount(); }
    catch (error) { notice(`Sample saved. Could not refresh the saved count: ${error.message}`, true); }
  } catch (error) { notice(`Could not save: ${error.message}. Your drawing is still here.`, true); }
  finally { busy = false; updateControls(); }
};
$("export").onclick = async () => {
  busy = true; updateControls();
  try {
    const samples = await transaction("readonly", (store) => store.getAll());
    const bytes = await request("export", { samples });
    if (downloadUrl) URL.revokeObjectURL(downloadUrl);
    downloadUrl = URL.createObjectURL(new Blob([bytes], { type: "application/zip" }));
    $("download").href = downloadUrl;
    $("download").download = `kana-samples-${Date.now()}.zip`;
    $("download").textContent = `Download ${samples.length} samples (.zip)`;
    $("download").hidden = false;
    notice("Archive ready. Tap the download link to keep it in Files.");
  } catch (error) { notice(`Could not export: ${error.message}`, true); }
  finally { busy = false; updateControls(); }
};
document.addEventListener("keydown", (event) => {
  if (["INPUT", "SELECT", "TEXTAREA"].includes(document.activeElement.tagName) || event.ctrlKey || event.metaKey || event.altKey) return;
  const id = { c: "clear", u: "undo", s: "save", n: "skip" }[event.key.toLowerCase()];
  if (id && !$(id).disabled) { event.preventDefault(); $(id).click(); }
});
draw();
selectScript();
openStorage().then(async (storage) => {
  db = storage;
  db.onversionchange = () => { db.close(); db = null; updateControls(); notice("Sample storage changed in another tab. Reload to continue.", true); };
  await storageCount();
}).catch((error) => { $("storage").textContent = `Sample storage unavailable: ${error.message}`; });
if ("serviceWorker" in navigator) {
  navigator.serviceWorker.register("./service-worker.js", { updateViaCache: "none" }).catch((error) => notice(`Offline cache unavailable: ${error.message}`, true));
}
