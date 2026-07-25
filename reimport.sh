#!/usr/bin/env bash
# Re-import the separately versioned Control Systems packs.
#
# Usage:
#   ./reimport.sh                    # ~/idiosepius/study.db
#   ./reimport.sh path/to/study.db
set -euo pipefail

repo_dir=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
cd "$repo_dir"

db_path=${1:-"$HOME/idiosepius/study.db"}
if (($# > 1)); then
  echo "usage: $0 [study.db]" >&2
  exit 2
fi

shopt -s nullglob
packs=(content/cs-0*.json)
if ((${#packs[@]} == 0)); then
  echo "no Control Systems packs found under $repo_dir/content" >&2
  exit 1
fi

mkdir -p -- "$(dirname -- "$db_path")"

echo "re-importing ${#packs[@]} packs into $db_path"
cargo run -q -p idiosepius-core --bin idiodb -- \
  "$db_path" import "${packs[@]}"
