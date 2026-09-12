#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$repo_root"

mkdir -p target
exec 9>target/build-web.lock
if ! flock -n 9; then
    echo "Another web build is already running." >&2
    exit 1
fi

wasm-pack build crates/app \
    --target web \
    --out-dir ../../web/pkg \
    --out-name idiosepius_app \
    --no-pack \
    --no-typescript \
    "$@"

# The diagnostic canvas loads only its recognizer worker, not egui, the study
# database or the audio engine. Its generated files join the same manifest.
wasm-pack build crates/kana-web \
    --target web \
    --out-dir ../../web/pkg/kana \
    --out-name idiosepius_kana_web \
    --no-pack \
    --no-typescript \
    "$@"

# Include hashed wasm-bindgen snippets and content-version the offline cache.
python3 tools/write-web-manifest.py
