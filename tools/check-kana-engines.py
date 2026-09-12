#!/usr/bin/env python3
"""Run real-worker/storage/offline checks in WebKit or Firefox on a fresh profile.

This uses mouse PointerEvents with pen-only disabled. Chromium's separate pen
suite exercises pressure/coalescing; Linux WebKit is not an Apple Pencil test.
Use a Playwright version matching the installed browser binaries.
"""

import argparse
import json
import threading
import time
import zipfile
from functools import partial
from http.server import SimpleHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path

from playwright.sync_api import sync_playwright

ROOT = Path(__file__).resolve().parent.parent


class QuietHandler(SimpleHTTPRequestHandler):
    def log_message(self, *args):
        pass


def draw_fu(page):
    page.locator("#ink").scroll_into_view_if_needed()
    rect = page.locator("#ink").bounding_box()
    points = [(163, 306), (241, 318), (691, 218), (774, 252), (566, 568), (246, 847)]
    for i, (x, y) in enumerate(points):
        page.mouse.move(
            rect["x"] + 1 + x / 1024 * (rect["width"] - 2),
            rect["y"] + 1 + y / 1024 * (rect["height"] - 2),
        )
        if i == 0:
            page.mouse.down()
    page.mouse.up()
    page.wait_for_function('!document.getElementById("save").disabled', timeout=90000)
    assert page.locator("#verdict").inner_text() == "Recognized: フ"
    assert page.locator("#vision-candidates li").count() == 5


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--engine", choices=["webkit", "firefox"], default="webkit")
    parser.add_argument("--executable")
    parser.add_argument(
        "--offline-mode",
        choices=["server", "browser"],
        default="server",
        help="Stop the HTTP server (default), or ask Playwright to emulate offline",
    )
    args = parser.parse_args()
    output = (
        ROOT / "target/kana-diagnostics" / f"{args.engine}-v1" / str(time.time_ns())
    )
    output.mkdir(parents=True)
    server = ThreadingHTTPServer(
        ("127.0.0.1", 0), partial(QuietHandler, directory=str(ROOT / "web"))
    )
    threading.Thread(target=server.serve_forever, daemon=True).start()
    errors = []
    try:
        with sync_playwright() as playwright:
            browser = getattr(playwright, args.engine).launch(
                headless=True,
                **({"executable_path": args.executable} if args.executable else {}),
            )
            context = browser.new_context(
                viewport={"width": 1024, "height": 1366}, device_scale_factor=2
            )
            page = context.new_page()
            page.on("pageerror", lambda error: errors.append(str(error)))
            page.goto(f"http://127.0.0.1:{server.server_port}/kana.html")
            page.wait_for_function(
                '!document.getElementById("start").disabled', timeout=90000
            )
            page.wait_for_function(
                'document.getElementById("storage").textContent.startsWith("0 samples")'
            )
            page.select_option("#script", "katakana")
            page.wait_for_function('!document.getElementById("start").disabled')
            page.uncheck("#pen-only")
            page.fill("#expected", "フ")
            draw_fu(page)
            page.click("#save")
            page.wait_for_function(
                'document.getElementById("storage").textContent.startsWith("1 samples")'
            )
            page.click("#export")
            page.wait_for_selector("#download:visible")
            with page.expect_download() as download:
                page.click("#download")
            archive = output / "samples.zip"
            download.value.save_as(str(archive))
            with zipfile.ZipFile(archive) as z:
                assert len(z.namelist()) == 1
                saved = json.loads(z.read(z.namelist()[0]))
                assert saved["expected"] == "フ"
                assert all(
                    point["pressure"] is None and point["pointer"] == "mouse"
                    for stroke in saved["strokes"]
                    for point in stroke
                )
                assert saved["vision"]["error"] is None
                z.extractall(output / "corpus")
            page.evaluate("navigator.serviceWorker.ready")
            page.wait_for_function(
                "navigator.serviceWorker.controller !== null", timeout=90000
            )
            if args.offline_mode == "browser":
                context.set_offline(True)
            else:
                server.shutdown()
                server.server_close()
            response = page.reload()
            assert response.from_service_worker, (
                "offline navigation did not come from the service worker"
            )
            page.wait_for_function(
                '!document.getElementById("start").disabled', timeout=90000
            )
            page.wait_for_function(
                'document.getElementById("storage").textContent.startsWith("1 samples")'
            )
            page.select_option("#script", "katakana")
            page.wait_for_function('!document.getElementById("start").disabled')
            page.uncheck("#pen-only")
            draw_fu(page)
            page.screenshot(path=str(output / "offline-portrait.png"), full_page=True)
            assert not errors, errors
            version = browser.version
            browser.close()
        (output / "summary.json").write_text(
            json.dumps(
                {
                    "passed": True,
                    "engine": args.engine,
                    "browser_version": version,
                    "offline_mode": args.offline_mode,
                    "model_fingerprint": saved["vision"]["model_fingerprint"],
                    "checks": [
                        "mouse-pointer-events",
                        "worker-wasm",
                        "indexeddb-save",
                        "zip-export",
                        "reload-persistence",
                        "offline-worker",
                    ],
                    "limitation": "Linux engine check, not Safari or Apple Pencil hardware",
                },
                indent=2,
            )
            + "\n"
        )
        print(f"{args.engine} checks passed: {output.relative_to(ROOT)}")
    finally:
        server.shutdown()


if __name__ == "__main__":
    main()
