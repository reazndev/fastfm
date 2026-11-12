#!/usr/bin/env bash
# Every check CI runs. Run this before each commit.
set -euo pipefail
cd "$(dirname "$0")/.."

scripts/check-file-size.sh
cargo fmt --all --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features

if [ -f web/package.json ]; then
  npm --prefix web run lint
  npm --prefix web run format:check
  npm --prefix web run test -- --watch=false
  npm --prefix web run build
fi
