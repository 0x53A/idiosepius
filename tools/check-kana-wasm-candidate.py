#!/usr/bin/env python3
"""Compare a local candidate's browser Wasm and native results on 92 references.

Uses a fresh Chromium profile and temporary local server. No embedded model,
service worker, study database, or user's browser profile is changed.
"""

import argparse
import hashlib
import json
import statistics
import subprocess
import threading
from functools import partial
from http.server import SimpleHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path

from playwright.sync_api import sync_playwright

ROOT = Path(__file__).resolve().parent.parent


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("model", type=Path)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument(
        "--executable", default="/run/current-system/sw/bin/google-chrome"
    )
    args = parser.parse_args()
    binary = args.model.read_bytes()
    samples = {}
    for row in json.loads(
        (ROOT / "crates/core/assets/kana_templates.json").read_text()
    )["templates"]:
        samples.setdefault(
            row["label"],
            {
                "expected": row["label"],
                "script": row["script"],
                "strokes": [
                    [{"x": x, "y": y} for x, y in stroke] for stroke in row["strokes"]
                ],
            },
        )
    samples = list(samples.values())
    assert len(samples) == 92
    native = json.loads(
        subprocess.check_output(
            [str(ROOT / "target/release/kana-vision"), "--model", str(args.model)],
            input=json.dumps(samples).encode(),
        )
    )

    class Handler(SimpleHTTPRequestHandler):
        def log_message(self, *args):
            pass

        def do_GET(self):
            if self.path == "/candidate.bin":
                self.send_response(200)
                self.send_header("Content-Type", "application/octet-stream")
                self.send_header("Content-Length", str(len(binary)))
                self.end_headers()
                self.wfile.write(binary)
            else:
                super().do_GET()

    server = ThreadingHTTPServer(
        ("127.0.0.1", 0), partial(Handler, directory=str(ROOT / "web"))
    )
    threading.Thread(target=server.serve_forever, daemon=True).start()
    try:
        with sync_playwright() as playwright:
            browser = playwright.chromium.launch(
                executable_path=args.executable, headless=True
            )
            try:
                page = browser.new_page(service_workers="block")
                page.goto(
                    f"http://127.0.0.1:{server.server_port}/pkg/asset-manifest.json"
                )
                # Labels are deliberately omitted from the inference input.
                actual = page.evaluate(
                    """async samples => {
                    const {default: init, KanaEngine} = await import('/pkg/kana/idiosepius_kana_web.js');
                    await init();
                    const bytes = new Uint8Array(await (await fetch('/candidate.bin')).arrayBuffer());
                    const engine = KanaEngine.with_model(bytes);
                    try {
                        return samples.map(sample => {
                            const start = performance.now();
                            const result = JSON.parse(engine.recognize_pair(JSON.stringify(sample.strokes), sample.script));
                            return {vision: result.vision, milliseconds: performance.now() - start};
                        });
                    } finally {engine.free();}
                }""",
                    [
                        {"script": row["script"], "strokes": row["strokes"]}
                        for row in samples
                    ],
                )
                version = browser.version
            finally:
                browser.close()
    finally:
        server.shutdown()
        server.server_close()
    maximum = 0
    for rust, wasm in zip(native, actual, strict=True):
        a, b = rust["vision"], wasm["vision"]
        assert a["model_fingerprint"] == b["model_fingerprint"]
        assert a["error"] == b["error"]
        assert [x["character"] for x in a["candidates"]] == [
            x["character"] for x in b["candidates"]
        ]
        maximum = max(
            maximum,
            max(
                abs(x["score"] - y["score"])
                for x, y in zip(a["candidates"], b["candidates"], strict=True)
            ),
        )
    assert maximum < 2e-5, maximum
    times = sorted(row["milliseconds"] for row in actual)
    result = {
        "passed": True,
        "samples": len(samples),
        "browser": version,
        "model_sha256": hashlib.sha256(binary).hexdigest(),
        "maximum_score_delta": maximum,
        "milliseconds": {
            "median": statistics.median(times),
            "p95": times[int(0.95 * (len(times) - 1))],
            "maximum": max(times),
        },
        "note": "Desktop Wasm module replay of public references, both classifiers; no worker scheduling, independent handwriting accuracy, or iPad latency claim.",
    }
    args.output.write_text(json.dumps(result, indent=2) + "\n")
    print(json.dumps(result, indent=2))


if __name__ == "__main__":
    main()
