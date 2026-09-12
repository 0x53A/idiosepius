#!/usr/bin/env python3
"""Replay all 92 canonical references through real browser Wasm and native Rust.

Uses a fresh profile and public reference strokes. It measures arithmetic and
worker latency, not real handwriting accuracy or Apple Pencil behavior.
"""

import argparse
import hashlib
import json
import statistics
import subprocess
import threading
import time
from functools import partial
from http.server import SimpleHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path

from playwright.sync_api import sync_playwright

ROOT = Path(__file__).resolve().parent.parent


class QuietHandler(SimpleHTTPRequestHandler):
    def log_message(self, *args):
        pass


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--executable", default="/run/current-system/sw/bin/google-chrome"
    )
    args = parser.parse_args()
    templates = json.loads(
        (ROOT / "crates/core/assets/kana_templates.json").read_text()
    )["templates"]
    samples = {}
    for template in templates:
        samples.setdefault(
            template["label"],
            {
                "expected": template["label"],
                "script": template["script"],
                "strokes": [
                    [{"x": x, "y": y} for x, y in stroke]
                    for stroke in template["strokes"]
                ],
            },
        )
    samples = list(samples.values())
    assert len(samples) == 92
    native = json.loads(
        subprocess.check_output(
            [str(ROOT / "target/release/kana-vision")],
            input=json.dumps(samples).encode(),
        )
    )
    server = ThreadingHTTPServer(
        ("127.0.0.1", 0), partial(QuietHandler, directory=str(ROOT / "web"))
    )
    threading.Thread(target=server.serve_forever, daemon=True).start()
    try:
        with sync_playwright() as playwright:
            browser = playwright.chromium.launch(
                executable_path=args.executable, headless=True
            )
            page = browser.new_page()
            page.goto(f"http://127.0.0.1:{server.server_port}/kana.html")
            outputs = page.evaluate(
                """async samples => {
                const worker = new Worker('./kana-worker.js', {type: 'module'});
                let next = 0;
                const query = data => new Promise((resolve, reject) => {
                    const id = ++next;
                    const timer = setTimeout(() => reject(new Error('worker timed out')), 30000);
                    worker.onmessage = ({data: result}) => {
                        clearTimeout(timer);
                        if (result.id !== id) reject(new Error('worker response id mismatch'));
                        else if (result.error) reject(new Error(result.error));
                        else resolve(result.value);
                    };
                    worker.onerror = event => {clearTimeout(timer); reject(new Error(event.message));};
                    worker.postMessage({id, ...data});
                });
                try {
                    await query({action:'init', script:'hiragana'});
                    const results = [];
                    for (const {script, strokes} of samples) {
                        const start = performance.now();
                        const value = await query({action:'recognize', script, strokes});
                        results.push({value, ms:performance.now()-start});
                    }
                    return results;
                } finally {worker.terminate();}
            }""",
                samples,
            )
            version = browser.version
            browser.close()
    finally:
        server.shutdown()
        server.server_close()
    maximum = 0.0
    for a, b in zip(native, outputs, strict=True):
        av, bv = a["vision"], b["value"]["vision"]
        assert av["model_fingerprint"] == bv["model_fingerprint"]
        assert av["error"] == bv["error"] is None
        assert [r["character"] for r in av["candidates"]] == [
            r["character"] for r in bv["candidates"]
        ]
        maximum = max(
            maximum,
            *(
                abs(x["score"] - y["score"])
                for x, y in zip(av["candidates"], bv["candidates"], strict=True)
            ),
        )
    assert maximum < 0.00002, maximum
    times = sorted(r["ms"] for r in outputs)
    result = {
        "passed": True,
        "samples": len(samples),
        "model_fingerprint": native[0]["vision"]["model_fingerprint"],
        "maximum_score_delta": maximum,
        "browser_version": version,
        "worker_roundtrip_ms": {
            "median": statistics.median(times),
            "p95": times[int(0.95 * (len(times) - 1))],
            "maximum": max(times),
        },
        "template_sha256": hashlib.sha256(
            (ROOT / "crates/core/assets/kana_templates.json").read_bytes()
        ).hexdigest(),
        "note": "Public canonical references, Chromium desktop; not a handwriting accuracy or iPad latency estimate.",
    }
    out = ROOT / "target/kana-diagnostics/worker-parity-v1" / str(time.time_ns())
    out.mkdir(parents=True)
    (out / "summary.json").write_text(json.dumps(result, indent=2) + "\n")
    print(json.dumps(result, indent=2))
    print(f"Worker parity checks passed: {out.relative_to(ROOT)}")


if __name__ == "__main__":
    main()
