#!/usr/bin/env bash
# Re-import a module's packs from the separately versioned content checkout.
#
# Usage:
#   ./reimport.sh                          # module cs -> ~/idiosepius/study.db
#   ./reimport.sh -m ma                    # module ma -> the same database
#   ./reimport.sh -m ma path/to/other.db   # or a separate one
#
# One invocation imports one module, because packs are merged by deck slug and
# the importer rejects a set describing more than one deck. That is a limit on
# the *call*, not on the database: run this once per module against the same
# path and both decks live there side by side, each with its own exam date and
# scheduling state. Pass a path only when you actually want them apart.
set -euo pipefail

repo_dir=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
cd "$repo_dir"

module=cs
while [[ ${1:-} == -* ]]; do
  case $1 in
    -m | --module)
      module=${2:?missing module prefix}
      shift 2
      ;;
    *)
      echo "usage: $0 [-m MODULE] [study.db]" >&2
      exit 2
      ;;
  esac
done

if (($# > 1)); then
  echo "usage: $0 [-m MODULE] [study.db]" >&2
  exit 2
fi

db_path=${1:-"$HOME/idiosepius/study.db"}

shopt -s nullglob
# One directory per module under content/, each a repository of its own.
# Two digits, not "-0*": a module with more than nine topic files would
# otherwise be imported silently truncated at 09.
packs=(content/*/"$module"-[0-9][0-9]-*.json)
if ((${#packs[@]} == 0)); then
  echo "no '$module' packs found under $repo_dir/content" >&2
  exit 1
fi

mkdir -p -- "$(dirname -- "$db_path")"

echo "re-importing ${#packs[@]} '$module' packs into $db_path"
cargo run -q -p idiosepius-core --bin idiodb -- \
  "$db_path" import "${packs[@]}"
