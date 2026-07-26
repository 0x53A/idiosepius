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

# wasm-bindgen can emit hashed snippet modules in addition to the predictable
# loader and .wasm names. Give the service worker a complete, deterministic
# package list so a first successful visit really is sufficient for offline use.
python3 - "$repo_root/web/pkg" <<'PY'
import json
import sys
from pathlib import Path

package_dir = Path(sys.argv[1])
manifest_path = package_dir / "asset-manifest.json"
assets = sorted(
    f"./pkg/{path.relative_to(package_dir).as_posix()}"
    for path in package_dir.rglob("*")
    if path.is_file() and path.name not in {".gitignore", manifest_path.name}
)
manifest_path.write_text(json.dumps(assets, indent=2) + "\n", encoding="utf-8")
PY
