#!/usr/bin/env bash
# Render the UI headlessly and write PNGs, so the look can be checked without
# a desktop session (and diffed between changes).
#
#   nix-shell --run ./tools/shot.sh            # module cs
#   nix-shell --run "./tools/shot.sh -m ma"    # module ma
#   nix-shell --run "./tools/shot.sh formulas-study formulas-narrow"
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
      echo "usage: $0 [-m MODULE] [SHOT ...]" >&2
      exit 2
      ;;
  esac
done

requested=("$@")
wanted() {
  ((${#requested[@]} == 0)) && return 0
  local candidate
  for candidate in "${requested[@]}"; do
    [[ $candidate == "$1" ]] && return 0
  done
  return 1
}
captured=0

out=target/shots
[[ $module == cs ]] || out=target/shots/$module
db=$out/shot.db
mkdir -p "$out"

cargo build -p idiosepius-app

shopt -s nullglob
# One directory per module under content/, each a repository of its own.
# The same glob as reimport.sh: two digits, so a tenth topic file is not
# silently left out.
pack_candidates=(content/*/"$module"-[0-9][0-9]-*.json)
packs=()
for pack in "${pack_candidates[@]}"; do
  [[ $pack == *.sheet.json ]] || packs+=("$pack")
done
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

# shoot_at <width> <height> <name> [extra idio args...]
shoot_at() {
  local width=$1 height=$2 name=$3
  shift 3
  wanted "$name" || return 0
  captured=$((captured + 1))
  # winit prefers Wayland when WAYLAND_DISPLAY is set; force X11 for Xvfb.
  env -u WAYLAND_DISPLAY xvfb-run -a -s "-screen 0 ${width}x${height}x24" \
    env LIBGL_ALWAYS_SOFTWARE=1 \
    ./target/debug/idiosepius-app "$db" --shot "$out/$name.pam" "$@" 2>/dev/null

  if command -v magick >/dev/null; then
    magick "$out/$name.pam" "$out/$name.png" && rm -f "$out/$name.pam"
    echo "  $out/$name.png"
  fi
}

# shoot <name> [extra idio args...]
shoot() {
  local name=$1
  shift
  shoot_at 1000 760 "$name" "$@"
}

shoot math      --screen math
shoot formulas  --screen formulas
# The sheet beside a live card is the arrangement worth diffing.
shoot formulas-study --screen formulas-study --card "$truefalse"
shoot plots     --screen plots
shoot plot-zoom --screen plot-zoom
shoot decks     --screen decks
shoot settings  --screen settings
shoot settings-open --screen settings-open
shoot soundscape --screen soundscape
shoot course    --screen course
shoot lessons   --screen lessons
shoot lesson    --screen lesson
# The foot of a reading: the automatic symbol glossary, the read marker and the
# practice row. `--card` pins a lesson by uid the way it pins a question, and
# on a lesson `--drag` is a scroll offset, for a capture of a figure mid-body.
shoot lesson-end --screen lesson-end
shoot lesson-questions --screen lesson-questions
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

# Below the docking threshold the same sheet remains the exclusive modal.
shoot_at 620 760 formulas-narrow \
  --screen formulas-study --card "$truefalse"

# A short viewport with enough decks to force the home list to scroll. Add
# these only after every content-specific capture, so the first deck selected
# by the screenshot routes stays the real imported module.
if wanted decks-small || wanted decks-small-end; then
  for n in 2 3 4 5 6; do
    sqlite3 "$db" \
      "INSERT INTO deck (slug, title, description, exam_at, created_at)
       VALUES ('shot-$n', 'Screenshot Deck $n', NULL, NULL, $n)"
  done
  shoot_at 620 420 decks-small --screen decks
  shoot_at 620 420 decks-small-end --screen decks-scroll
fi

if ((${#requested[@]} > 0 && captured == 0)); then
  echo "none of the requested shots are known: ${requested[*]}" >&2
  exit 2
fi

echo "done"
