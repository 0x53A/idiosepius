#!/usr/bin/env bash
# Test the whole workspace, not just the default member.
#
# The workspace default-members is crates/app, so a bare `cargo test` skips
# core (schema, scheduler, session logging) entirely. This runs everything.
#
#   ./tools/run-all-tests.sh [extra cargo test args...]
#
# No nix-shell needed: rusqlite is bundled and nothing here opens a window.
set -euo pipefail

cd "$(dirname "$0")/.."

cargo test --workspace --all-targets "$@"
