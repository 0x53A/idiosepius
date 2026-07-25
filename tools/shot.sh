#!/usr/bin/env bash
# Render the UI headlessly and write PNGs, so the look can be checked without
# a desktop session (and diffed between changes).
#
#   nix-shell --run ./tools/shot.sh
#
# Output lands in target/shots/.
set -euo pipefail

cd "$(dirname "$0")/.."

out=target/shots
db=$out/shot.db
mkdir -p "$out"

cargo build -p idiosepius-app

# A throwaway database so screenshots never touch real study history.
rm -f "$db" "$db-wal" "$db-shm"
cargo run -q -p idiosepius-core --bin idiodb -- "$db" import content/cs-0*.json >/dev/null

# shoot <name> [extra idio args...]
shoot() {
  local name=$1
  shift
  # winit prefers Wayland when WAYLAND_DISPLAY is set; force X11 for Xvfb.
  env -u WAYLAND_DISPLAY xvfb-run -a -s "-screen 0 1000x760x24" \
    env LIBGL_ALWAYS_SOFTWARE=1 \
    ./target/debug/idiosepius-app "$db" --shot "$out/$name.pam" "$@" 2>/dev/null

  if command -v magick >/dev/null; then
    magick "$out/$name.pam" "$out/$name.png" && rm -f "$out/$name.pam"
    echo "  $out/$name.png"
  fi
}

shoot decks     --screen decks
shoot choice    --screen study --card cs-sta-036
shoot truefalse --screen study --card cs-sta-009
shoot swipe-yes --screen study --card cs-sta-009 --drag 95
shoot swipe-no  --screen study --card cs-mod-002 --drag -95

echo "done"
