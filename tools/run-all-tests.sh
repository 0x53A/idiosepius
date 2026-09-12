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

if ! {
  python3 tools/check-kana-cross-source.py --self-test
  python3 tools/check-kana-etl.py --self-test
  python3 tools/write-web-manifest.py --self-test
  python3 tools/kana_etl7.py
  python3 tools/check-kana-corpus.py --self-test
  python3 tools/check-kana-reviews.py --self-test
  python3 tools/check-kana-preprocessing.py --self-test
  python3 tools/check-kana-k49.py --self-test
  python3 tools/build-kana-k49-variants.py --self-test
} >"$test_log" 2>&1; then
  cat "$test_log" >&2
  exit 1
fi

if cargo test --workspace --all-targets "$@" >"$test_log" 2>&1; then
  echo "all workspace tests passed"
else
  cat "$test_log" >&2
  exit 1
fi
