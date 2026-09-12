#!/usr/bin/env python3
"""Verify offline build updates and failed-install recovery in an isolated copy.

No user browser data is opened. The service-worker source stays identical while
an unused package asset changes, proving the generated import triggers updates.
"""

import argparse
import importlib.util
import json
import shutil
import tempfile
import threading
import time
from functools import partial
from http.server import SimpleHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path

from playwright.sync_api import sync_playwright

ROOT = Path(__file__).resolve().parent.parent
spec = importlib.util.spec_from_file_location(
    "manifest", Path(__file__).with_name("write-web-manifest.py")
)
manifest = importlib.util.module_from_spec(spec)
spec.loader.exec_module(manifest)


class QuietHandler(SimpleHTTPRequestHandler):
    def do_GET(self):
        if self.path == "/slow-update-probe":
            self.server.slow_started.set()
            self.server.slow_release.wait(90)
            self.send_response(200)
            self.send_header("Content-Type", "text/plain")
            self.end_headers()
            self.wfile.write(b"slow response")
        else:
            super().do_GET()

    def log_message(self, *args):
        pass

    def copyfile(self, source, outputfile):
        try:
            super().copyfile(source, outputfile)
        except (BrokenPipeError, ConnectionResetError):
            pass  # the offline transition deliberately cancels network requests


def wait_cache(page, version):
    deadline = time.monotonic() + 90
    prefix = page.evaluate(
        "'idiosepius-offline-' + encodeURIComponent(new URL('./', location.href).href) + '-'"
    )
    expected = prefix + version
    while time.monotonic() < deadline:
        names = page.evaluate(
            "async prefix => (await caches.keys()).filter(k => k.startsWith(prefix))",
            prefix,
        )
        if names == [expected]:
            return expected
        page.wait_for_timeout(100)
    raise AssertionError(f"offline cache did not become {expected}: {names}")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--chromium", default="/run/current-system/sw/bin/google-chrome"
    )
    args = parser.parse_args()
    output = ROOT / "target/kana-diagnostics/web-update-v1" / str(time.time_ns())
    output.mkdir(parents=True)
    with tempfile.TemporaryDirectory(prefix="idiosepius-web-update-") as directory:
        web = Path(directory) / "web"
        shutil.copytree(ROOT / "web", web)
        probe = web / "pkg/update-probe.txt"
        probe.write_text("build one")
        first = manifest.generate(web)
        worker_source = (web / "service-worker.js").read_bytes()
        server = ThreadingHTTPServer(
            ("127.0.0.1", 0), partial(QuietHandler, directory=str(web))
        )
        server.slow_started = threading.Event()
        server.slow_release = threading.Event()
        threading.Thread(target=server.serve_forever, daemon=True).start()
        try:
            with sync_playwright() as p:
                browser = p.chromium.launch(
                    executable_path=args.chromium, headless=True
                )
                context = browser.new_context()
                page = context.new_page()
                page.goto(f"http://127.0.0.1:{server.server_port}/kana.html")
                page.wait_for_function(
                    "navigator.serviceWorker.controller !== null", timeout=90000
                )
                wait_cache(page, first["version"])
                # A second deployment under the same origin owns its own cache.
                preview = web / "preview"
                shutil.copytree(ROOT / "web", preview)
                preview_inventory = manifest.generate(preview)
                preview_page = context.new_page()
                preview_page.goto(
                    f"http://127.0.0.1:{server.server_port}/preview/kana.html"
                )
                preview_page.wait_for_function(
                    "navigator.serviceWorker.controller !== null", timeout=90000
                )
                preview_cache = wait_cache(preview_page, preview_inventory["version"])
                assert (
                    page.evaluate(
                        "async () => (await caches.match('./pkg/update-probe.txt')).text()"
                    )
                    == "build one"
                )
                lingering_page = context.new_page()
                lingering_page.goto(f"http://127.0.0.1:{server.server_port}/kana.html")
                lingering_page.wait_for_function(
                    "navigator.serviceWorker.controller !== null"
                )
                lingering_page.evaluate(
                    "void fetch('./slow-update-probe').then(r => r.text()).then(text => window.slowResult = text)"
                )
                assert server.slow_started.wait(5), (
                    "slow request never reached the server"
                )
                probe.write_text("build two")
                second = manifest.generate(web)
                assert (web / "service-worker.js").read_bytes() == worker_source
                assert second["version"] != first["version"]
                # Ordinary route entry re-registers, including a network check of
                # the generated imported script; do not manually call update().
                page.reload()
                second_cache = page.evaluate(
                    "version => 'idiosepius-offline-' + encodeURIComponent(new URL('./', location.href).href) + '-' + version",
                    second["version"],
                )
                # Activation may wait for the old worker's in-flight event. Wait
                # only for installation to cache the new asset before releasing it.
                deadline = time.monotonic() + 90
                while time.monotonic() < deadline:
                    value = page.evaluate(
                        "async name => { const r = await caches.match('./pkg/update-probe.txt', {cacheName:name}); return r ? await r.text() : null; }",
                        second_cache,
                    )
                    if value == "build two":
                        break
                    page.wait_for_timeout(100)
                else:
                    raise AssertionError(
                        "new build was not cached while old request was pending"
                    )
                server.slow_release.set()
                lingering_page.wait_for_function(
                    "window.slowResult === 'slow response'"
                )
                second_cache = wait_cache(page, second["version"])

                state = page.evaluate("""async () => {
                  const result = {};
                  for (const name of await caches.keys()) {
                    const r = await (await caches.open(name)).match('./pkg/update-probe.txt');
                    result[name] = r ? await r.text() : null;
                  }
                  return result;
                }""")
                assert state.get(second_cache) == "build two", state
                lingering_page.wait_for_function(
                    "window.slowResult === 'slow response'"
                )
                # Finishing an old fetch must not recreate an obsolete cache.
                wait_cache(page, second["version"])
                lingering_page.close()
                assert page.evaluate(
                    "async name => (await caches.keys()).includes(name)", preview_cache
                )
                context.set_offline(True)
                preview_page.reload()
                preview_page.wait_for_function(
                    '!document.getElementById("start").disabled', timeout=90000
                )
                page.reload()
                page.wait_for_function(
                    '!document.getElementById("start").disabled', timeout=90000
                )
                assert (
                    page.evaluate(
                        "async () => (await fetch('./pkg/update-probe.txt')).text()"
                    )
                    == "build two"
                )
                context.set_offline(False)
                probe.write_text("incomplete build three")
                third = manifest.generate(web)
                probe.unlink()  # simulate an incomplete deployment after inventory publication
                page.evaluate("""async () => {
                  const r = await navigator.serviceWorker.getRegistration();
                  window.failedInstall = false;
                  r.addEventListener('updatefound', () => {
                    const worker = r.installing;
                    worker.addEventListener('statechange', () => {
                      if (worker.state === 'redundant') window.failedInstall = true;
                    });
                  });
                  await r.update();
                }""")
                page.wait_for_function("window.failedInstall", timeout=90000)
                second_cache = wait_cache(page, second["version"])
                assert page.evaluate(
                    "async name => (await caches.keys()).includes(name)", preview_cache
                )
                context.set_offline(True)
                preview_page.reload()
                preview_page.wait_for_function(
                    '!document.getElementById("start").disabled', timeout=90000
                )
                page.reload()
                page.wait_for_function(
                    '!document.getElementById("start").disabled', timeout=90000
                )
                assert (
                    page.evaluate(
                        "async () => (await fetch('./pkg/update-probe.txt')).text()"
                    )
                    == "build two"
                )
                browser.close()
            (output / "summary.json").write_text(
                json.dumps(
                    {
                        "passed": True,
                        "first_version": first["version"],
                        "second_version": second["version"],
                        "failed_version": third["version"],
                        "checks": [
                            "unchanged-worker-source-update",
                            "unused-asset-refresh",
                            "offline-new-build",
                            "failed-install-retains-old-cache",
                            "offline-after-failed-install",
                            "independent-deployment-scopes",
                            "old-inflight-request-does-not-recreate-cache",
                        ],
                    },
                    indent=2,
                )
                + "\n"
            )
            print(f"Web update checks passed: {output.relative_to(ROOT)}")
        finally:
            server.slow_release.set()
            server.shutdown()


if __name__ == "__main__":
    main()
