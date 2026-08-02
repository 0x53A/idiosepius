#!/usr/bin/env bash
# Test the whole workspace, not just the default member.
#
# The workspace default-members is crates/app, so a bare `cargo test` skips
# core (schema, scheduler, session logging) entirely. This runs everything.
#
#   ./tools/run-all-tests.sh [extra cargo test args...]
#
set -euo pipefail

cd "$(dirname "$0")/.."

test_log=$(mktemp)
trap 'rm -f "$test_log"' EXIT

if cargo test --workspace --all-targets "$@" >"$test_log" 2>&1; then
  echo "all workspace tests passed"
else
  cat "$test_log" >&2
  exit 1
fi
