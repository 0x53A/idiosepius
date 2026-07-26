#!/usr/bin/env bash
# Render the UI headlessly and write PNGs, so the look can be checked without
# a desktop session (and diffed between changes).
#
#   nix-shell --run ./tools/shot.sh            # module cs
#   nix-shell --run "./tools/shot.sh -m ma"    # module ma
#
# Output lands in target/shots/ (target/shots/<module>/ for anything but cs,
# so a second module's captures do not overwrite the first's).
set -euo pipefail

cd "$(dirname "$0")/.."

module=cs
while [[ ${1:-} == -* ]]; do
  case $1 in
    -m | --module)
      module=${2:?missing module prefix}
      shift 2
      ;;
    *)
      echo "usage: $0 [-m MODULE]" >&2
      exit 2
      ;;
  esac
done

out=target/shots
[[ $module == cs ]] || out=target/shots/$module
db=$out/shot.db
mkdir -p "$out"

cargo build -p idiosepius-app

shopt -s nullglob
# One directory per module under content/, each a repository of its own.
# The same glob as reimport.sh: two digits, so a tenth topic file is not
# silently left out.
packs=(content/*/"$module"-[0-9][0-9]-*.json)
if ((${#packs[@]} == 0)); then
  echo "no '$module' packs found under content/" >&2
  exit 1
fi

# A throwaway database so screenshots never touch real study history.
rm -f "$db" "$db-wal" "$db-shm"
cargo run -q -p idiosepius-core --bin idiodb -- "$db" import "${packs[@]}" >/dev/null

# Cards to capture. The `cs` set is curated — a choice card, a true/false one
# and a derivation that cites formula facts — so those screenshots stay
# comparable across changes. Any other module picks the same *shapes* by uid
# order, which is arbitrary but reproducible, and can be curated here later.
if [[ $module == cs ]]; then
  choice=cs-sta-036
  truefalse=cs-sta-009
  swipe_no=cs-mod-002
  deep_calc=cs-ide-006
else
  pick() { # pick <kind> <offset>
    sqlite3 "$db" \
      "SELECT uid FROM question WHERE kind = '$1' AND active = 1 ORDER BY uid LIMIT 1 OFFSET $2"
  }
  choice=$(pick multiple_choice 0)
  truefalse=$(pick true_false 0)
  swipe_no=$(pick true_false 1)
  deep_calc=$(pick multiple_choice 1)
  : "${truefalse:=$choice}" "${swipe_no:=$truefalse}" "${deep_calc:=$choice}"
  if [[ -z $choice ]]; then
    echo "module '$module' imported no questions" >&2
    exit 1
  fi
fi

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

shoot math      --screen math
shoot plots     --screen plots
shoot plot-zoom --screen plot-zoom
shoot decks     --screen decks
shoot course    --screen course
shoot lessons   --screen lessons
shoot lesson    --screen lesson
shoot questions --screen questions
shoot questions-scroll    --screen questions-scroll
shoot questions-collapsed --screen questions-collapsed
shoot progress  --screen progress
shoot choice    --screen study --card "$choice"
shoot truefalse --screen study --card "$truefalse"
shoot hover     --screen hover --card "$truefalse"
shoot swipe-yes --screen study --card "$truefalse" --drag 95
shoot swipe-no  --screen study --card "$swipe_no" --drag -95
shoot feedback  --screen feedback --card "$truefalse"
shoot explain   --screen explain --card "$truefalse"
shoot deep      --screen deep --card "$truefalse"
# A derivation that cites formula facts, so the formula block is diffable too.
shoot deep-calc --screen deep --card "$deep_calc"
shoot review    --screen review --card "$truefalse"
# Option notes: answered, so only the note under the option that was picked;
# and the same card in review, where every note is shown.
shoot notes-picked --screen feedback --card "$choice"
shoot notes-all    --screen review --card "$choice"

# A short viewport with enough decks to force the home list to scroll. Add
# these only after every content-specific capture, so the first deck selected
# by the screenshot routes stays the real imported module.
for n in 2 3 4 5 6; do
  sqlite3 "$db" \
    "INSERT INTO deck (slug, title, description, exam_at, created_at)
     VALUES ('shot-$n', 'Screenshot Deck $n', NULL, NULL, $n)"
done
env -u WAYLAND_DISPLAY xvfb-run -a -s "-screen 0 620x420x24" \
  env LIBGL_ALWAYS_SOFTWARE=1 \
  ./target/debug/idiosepius-app "$db" --shot "$out/decks-small.pam" --screen decks 2>/dev/null
if command -v magick >/dev/null; then
  magick "$out/decks-small.pam" "$out/decks-small.png" && rm -f "$out/decks-small.pam"
  echo "  $out/decks-small.png"
fi

echo "done"
