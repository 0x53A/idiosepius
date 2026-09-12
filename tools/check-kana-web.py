#!/usr/bin/env python3
"""Exercise the built canvas with Chromium's pen input and the real Wasm worker.

Build with tools/build-web.sh first, then run:
  uv run --with playwright python tools/check-kana-web.py
No browser profile, study database, or existing sample collection is opened.
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


def records(page):
    return page.evaluate("""() => new Promise((resolve, reject) => {
      const r = indexedDB.open('idiosepius-kana', 1);
      r.onsuccess = () => {
        const db = r.result;
        const q = db.transaction('samples').objectStore('samples').getAll();
        q.onsuccess = () => { db.close(); resolve(q.result); };
        q.onerror = () => reject(q.error);
      };
      r.onerror = () => reject(r.error);
    })""")


def pen_stroke(page, cdp, points, stationary=False, during=None):
    page.locator('#ink').scroll_into_view_if_needed()
    rect = page.locator('#ink').bounding_box()
    def send(kind, point, pressure):
        cdp.send('Input.dispatchMouseEvent', {
            'type': kind, 'x': rect['x'] + 1 + point[0] / 1024 * (rect['width'] - 2),
            'y': rect['y'] + 1 + point[1] / 1024 * (rect['height'] - 2),
            'button': 'left', 'buttons': 0 if kind == 'mouseReleased' else 1,
            'clickCount': 1, 'pointerType': 'pen', 'force': pressure,
        })
    send('mousePressed', points[0], .2)
    if during:
        during()
    if stationary:
        send('mouseMoved', points[0], .8)
    for point in points[1:]:
        send('mouseMoved', point, .6)
    send('mouseReleased', points[-1], 0.)


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--chromium', default='/run/current-system/sw/bin/google-chrome')
    args = parser.parse_args()
    output = ROOT / 'target/kana-diagnostics/browser-v1' / str(time.time_ns())
    output.mkdir(parents=True)
    server = ThreadingHTTPServer(('127.0.0.1', 0), partial(QuietHandler, directory=str(ROOT / 'web')))
    threading.Thread(target=server.serve_forever, daemon=True).start()
    url = f'http://127.0.0.1:{server.server_port}/kana.html'
    errors = []
    try:
        with sync_playwright() as p:
            browser = p.chromium.launch(executable_path=args.chromium, headless=True)
            context = browser.new_context(viewport={'width': 1024, 'height': 1366}, device_scale_factor=2)
            page = context.new_page()
            page.on('pageerror', lambda error: errors.append(str(error)))
            page.goto(url)
            page.evaluate("""document.getElementById('ink').addEventListener('pointerdown', e => {
                if(e.pointerType === 'pen') window.lastPenId = e.pointerId;
            })""")
            page.wait_for_function('!document.getElementById("start").disabled', timeout=90000)
            page.wait_for_function('document.getElementById("storage").textContent.startsWith("0 samples")')
            assert page.locator('#pen-only').is_checked()
            box = page.locator('#ink').bounding_box()
            page.mouse.move(box['x'] + 50, box['y'] + 50)
            page.mouse.down()
            page.mouse.move(box['x'] + 150, box['y'] + 150)
            page.mouse.up()
            assert page.locator('#undo').is_disabled(), 'pen-only accepted mouse ink'
            page.select_option('#script', 'katakana')
            page.wait_for_function('!document.getElementById("start").disabled')
            page.fill('#expected', 'フ')
            cdp = context.new_cdp_session(page)
            # Independent SVG coordinates: not derived from embedded templates.
            fu = [[163,306],[241,318],[691,218],[774,252],[566,568],[246,847]]
            pen_stroke(page, cdp, fu, stationary=True)
            page.wait_for_function('!document.getElementById("save").disabled')
            assert page.locator('#verdict').inner_text() == 'Recognized: フ'
            page.screenshot(path=str(output / 'ipad-portrait.png'), full_page=True)
            page.click('#save')
            page.wait_for_function('document.getElementById("storage").textContent.startsWith("1 samples")')
            saved = records(page)[0]
            assert saved['expected'] == 'フ'
            assert saved['result']['state'] == 'recognized'
            assert saved['vision']['experimental'] is True
            assert len(saved['vision']['model_fingerprint']) == 16
            assert len(saved['vision']['candidates']) == 5
            assert saved['reference']['character'] == saved['vision']['candidates'][0]['character']
            assert saved['vision']['error'] is None
            assert saved['capture']['point_time_ms_clock'] == 'dom_event_timestamp'
            assert all(p['pointer'] == 'pen' for p in saved['strokes'][0])
            assert any(p['pressure'] > .75 for p in saved['strokes'][0]), 'stationary pressure lost'
            assert saved['strokes'][0][-1]['pressure'] == 0

            def inject_coalesced_and_foreign_pointer():
                page.evaluate("""() => {
                  const c = document.getElementById('ink'), r = c.getBoundingClientRect();
                  const foreign = {pointerId: 99, pointerType: 'touch', clientX:r.left+5,clientY:r.top+5};
                  c.dispatchEvent(new PointerEvent('pointerdown', foreign));
                  c.dispatchEvent(new PointerEvent('pointermove', foreign));
                  c.dispatchEvent(new PointerEvent('pointerup', foreign));
                  const sample = (x, pressure) => new PointerEvent('pointermove', {
                    pointerId: window.lastPenId, pointerType:'pen', pressure,
                    clientX:r.left+x, clientY:r.top+100,
                  });
                  const parent = sample(300,.9);
                  Object.defineProperty(parent, 'getCoalescedEvents', {
                    value: () => [sample(100,.31),sample(120,.47)]
                  });
                  c.dispatchEvent(parent);
                }""")
            def cancel():
                page.evaluate("""document.getElementById('ink').dispatchEvent(new PointerEvent(
                    'pointercancel', {pointerId:window.lastPenId,pointerType:'pen'}))""")
            pen_stroke(page, cdp, fu, during=inject_coalesced_and_foreign_pointer)
            page.wait_for_function('!document.getElementById("save").disabled')
            # Inspect the raw record via export storage, then retain it as a
            # deliberately invalid probe rather than claiming it is handwriting.
            pen_stroke(page, cdp, fu, during=cancel)
            page.wait_for_function('!document.getElementById("undo").disabled')
            page.click('#undo')
            page.wait_for_function('!document.getElementById("save").disabled')
            page.fill('#expected', '-')
            page.click('#save')
            page.wait_for_function('document.getElementById("storage").textContent.startsWith("2 samples")')
            injected = records(page)[1]
            assert len(injected['strokes']) == 1
            assert all(p['pointer'] == 'pen' for p in injected['strokes'][0])
            pressures = [p['pressure'] for p in injected['strokes'][0]]
            assert any(abs(p-.31) < .001 for p in pressures)
            assert any(abs(p-.47) < .001 for p in pressures)
            assert not any(abs(p-.9) < .001 for p in pressures), 'aggregate event duplicated'
            assert injected['expected_invalid']
            assert injected['capture']['cancelled_strokes'] == 0
            assert injected['capture']['cancelled_stroke_indices'] == []

            pen_stroke(page, cdp, fu, during=cancel)
            page.wait_for_function('!document.getElementById("clear").disabled')
            assert 'interrupted' in page.locator('#notice').inner_text()
            page.click('#undo')
            assert page.locator('#undo').is_disabled()
            # Removing an interrupted stroke removes its cancellation metadata.
            # A failed write must keep the drawing and must not advance a prompt.

            # A guided save advances; a skip records the missing position.
            page.click('#start')
            prompt = page.locator('#expected').input_value()
            pen_stroke(page, cdp, fu)
            page.wait_for_function('!document.getElementById("save").disabled')
            page.evaluate("""() => {
              const add = IDBObjectStore.prototype.add;
              IDBObjectStore.prototype.add = function(...args) {
                IDBObjectStore.prototype.add = add;
                const request = add.apply(this, args);
                this.transaction.abort();
                return request;
              };
            }""")
            page.click('#save')
            page.wait_for_function('document.getElementById("notice").textContent.startsWith("Could not save:")')
            assert page.locator('#progress').inner_text().startswith('1 / 46')
            assert not page.locator('#save').is_disabled()
            assert len(records(page)) == 2
            # A count refresh failure after a successful commit must not claim
            # the save failed or invite saving the same drawing twice.
            page.evaluate("""() => {
              const count = IDBObjectStore.prototype.count;
              IDBObjectStore.prototype.count = function(...args) {
                IDBObjectStore.prototype.count = count;
                throw new Error('injected count failure');
              };
            }""")
            page.click('#save')
            page.wait_for_function('document.getElementById("notice").textContent.startsWith("Sample saved. Could not refresh")')
            page.wait_for_function('document.getElementById("progress").textContent.startsWith("2 / 46")')
            page.click('#skip')
            assert page.locator('#progress').inner_text().startswith('3 / 46')
            saved = records(page)
            assert saved[2]['capture']['cancelled_strokes'] == 0
            assert saved[2]['capture']['cancelled_stroke_indices'] == []
            assert saved[2]['expected'] == prompt
            assert saved[2]['expected_source'] == 'prompted'
            assert saved[2]['writing_validity'] == 'unknown'
            assert saved[2]['prompt']['position'] == 1
            assert len(set(saved[2]['prompt']['sequence'])) == 46

            # Reload keeps committed data, without silently resuming an old run.
            page.reload()
            page.wait_for_function('!document.getElementById("start").disabled')
            page.wait_for_function('document.getElementById("storage").textContent.startsWith("3 samples")')
            page.click('#export')
            page.wait_for_selector('#download:visible')
            with page.expect_download() as download:
                page.click('#download')
            archive = output / 'samples.zip'
            download.value.save_as(str(archive))
            with zipfile.ZipFile(archive) as z:
                assert len(z.namelist()) == 3
                exported = [json.loads(z.read(name)) for name in z.namelist()]
                assert exported == records(page)
                z.extractall(output / 'corpus')

            # The installed service worker must load the actual worker offline.
            page.evaluate('navigator.serviceWorker.ready')
            page.wait_for_function('navigator.serviceWorker.controller !== null', timeout=90000)
            context.set_offline(True)
            page.reload()
            page.wait_for_function('!document.getElementById("start").disabled', timeout=90000)
            page.select_option('#script', 'katakana')
            page.wait_for_function('!document.getElementById("start").disabled')
            pen_stroke(page, cdp, fu)
            page.wait_for_function('!document.getElementById("save").disabled')
            assert page.locator('#verdict').inner_text() == 'Recognized: フ'
            def assert_predictions_hidden_while_drawing():
                assert page.locator('#candidates li').count() == 0
                assert page.locator('#vision-candidates li').count() == 0
                assert page.locator('#save').is_disabled()
            pen_stroke(page, cdp, [[100,100],[200,200]], during=assert_predictions_hidden_while_drawing)
            page.wait_for_function('!document.getElementById("save").disabled')
            page.click('#undo')
            page.wait_for_function('!document.getElementById("save").disabled')
            assert page.locator('#verdict').inner_text() == 'Recognized: フ'
            context.set_offline(False)
            page.set_viewport_size({'width': 1180, 'height': 820})
            page.screenshot(path=str(output / 'ipad-landscape.png'), full_page=True)
            save_box = page.locator('#save').bounding_box()
            assert save_box['y'] + save_box['height'] <= 820
            page.set_viewport_size({'width': 390, 'height': 844})
            assert page.evaluate('document.documentElement.scrollWidth <= innerWidth')
            page.screenshot(path=str(output / 'narrow.png'), full_page=True)
            assert not errors, errors
            browser.close()
        (output / 'summary.json').write_text(json.dumps({'passed': True, 'saved_samples': 3,
            'checks': ['pen-only', 'stationary-pressure', 'upright-katakana', 'prompt-save-skip',
                       'coalesced-events', 'pointer-ownership', 'cancellation',
                       'reload-persistence', 'zip-roundtrip', 'offline-worker', 'narrow-layout',
                       'landscape-controls', 'vision-provenance', 'identity-reference-comparison', 'stale-predictions-hidden', 'undo-cancellation-metadata', 'failed-save-retains-ink', 'committed-save-count-failure']}, indent=2) + '\n')
        print(f'Browser checks passed: {output.relative_to(ROOT)}')
    finally:
        server.shutdown()


if __name__ == '__main__':
    main()
