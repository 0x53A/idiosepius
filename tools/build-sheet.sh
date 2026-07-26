#!/usr/bin/env bash
# Build a module's formula sheet PDF from its formula facts.
#
#   tools/build-sheet.sh cs          # -> content/control-systems/cs-formula-sheet.pdf
#   tools/build-sheet.sh cs --terse  # -> ...-terse.pdf
#   tools/build-sheet.sh             # every module that has a formulas pack
#
# The argument is a module prefix, matching the `<prefix>-00-formulas.json`
# naming every module follows. The sheet is written next to the pack it came
# from, inside that module's own repository.
#
# LaTeX is not in the development shell — it is a large dependency and only
# this script needs it — so this re-enters `tools/sheet-shell.nix` when
# tectonic is not already on PATH.
set -euo pipefail

repo_dir=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)
cd "$repo_dir"

if ! command -v tectonic >/dev/null 2>&1; then
  exec nix-shell tools/sheet-shell.nix --run "tools/build-sheet.sh $*"
fi

terse=
suffix=
args=()
for arg in "$@"; do
  if [[ $arg == --terse ]]; then
    terse=--terse
    suffix=-terse
  else
    args+=("$arg")
  fi
done

shopt -s nullglob

# No module named: do the lot, so adding a module needs no change here.
if ((${#args[@]} == 0)); then
  for pack in content/*/*-00-formulas.json; do
    base=${pack##*/}
    args+=("${base%%-00-formulas.json}")
  done
  if ((${#args[@]} == 0)); then
    echo "no <module>-00-formulas.json found under $repo_dir/content" >&2
    exit 1
  fi
fi

build=$(mktemp -d)
trap 'rm -rf "$build"' EXIT

for module in "${args[@]}"; do
  # One directory per module under content/, each a repository of its own.
  packs=(content/*/"$module"-00-formulas.json)
  if ((${#packs[@]} == 0)); then
    echo "no content/*/$module-00-formulas.json" >&2
    exit 1
  fi
  pack=${packs[0]}
  # The sheet belongs beside its source, in the module's own repository.
  out_dir=$(dirname "$pack")
  name="$module-formula-sheet$suffix"

  python3 tools/formula-sheet.py "$pack" $terse -o "$build/$name.tex"
  # Tectonic is chatty on stderr even when it succeeds; only failure matters.
  tectonic -X compile "$build/$name.tex" --outdir "$build" >/dev/null
  mv "$build/$name.pdf" "$out_dir/$name.pdf"
  echo "wrote $out_dir/$name.pdf"
done
