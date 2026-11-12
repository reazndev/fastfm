#!/usr/bin/env bash
# Runs the conformance harness against lfm-mirror serving a copy of data/conformance.db, with the
# clock frozen at the fixture recording time and a throwaway dev key. Prints the summary and exits
# with the harness's code (0 pass, 1 failures or regressions, 2 the run could not happen).
#
#   scripts/conformance.sh                                  # every fixture
#   scripts/conformance.sh --only user.getInfo              # extra args go to the harness
#   scripts/conformance.sh --expect conformance/expected.txt
#
# The JSON report goes to $CONFORMANCE_REPORT (default target/conformance-report.json).
set -euo pipefail
cd "$(dirname "$0")/.."

db=data/conformance.db
if [ ! -f "$db" ]; then
  echo "conformance: $db is missing. Run the import into it first (crates/sync)." >&2
  exit 2
fi

export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-target/conformance}"
report="${CONFORMANCE_REPORT:-target/conformance-report.json}"
cargo build --quiet -p lfm-mirror -p lfm-conformance

# The earliest Date header among the user.getInfo fixtures, as Unix seconds.
now=$(grep -h '"date":' fixtures/user.getInfo/*.meta.json \
  | sed 's/.*"date": "\(.*\)".*/\1/' \
  | while read -r stamp; do date -u -d "$stamp" +%s; done \
  | sort -n | head -1)
if [ -z "$now" ]; then
  echo "conformance: no Date header in fixtures/user.getInfo/*.meta.json" >&2
  exit 2
fi

work=$(mktemp -d)
server_pid=""
cleanup() {
  if [ -n "$server_pid" ]; then
    kill "$server_pid" 2>/dev/null || true
    wait "$server_pid" 2>/dev/null || true
  fi
  rm -rf "$work"
}
trap cleanup EXIT

# The server may write (migrations, background jobs); keep the imported database untouched.
for suffix in "" -wal -shm; do
  if [ -f "$db$suffix" ]; then cp "$db$suffix" "$work/conformance.db$suffix"; fi
done

port=$(python3 -c 'import socket; s=socket.socket(); s.bind(("127.0.0.1", 0)); print(s.getsockname()[1])')
key=$(od -An -tx1 -N16 /dev/urandom | tr -d ' \n')

LFM_BIND="127.0.0.1:$port" LFM_DATA_DIR="$work" LFM_DB="$work/conformance.db" \
  LFM_NOW="$now" LFM_DEV_API_KEY="$key" RUST_LOG="${RUST_LOG:-warn}" \
  "$CARGO_TARGET_DIR/debug/lfm-mirror" >"$work/server.log" 2>&1 &
server_pid=$!

for _ in $(seq 1 100); do
  if curl -sf "http://127.0.0.1:$port/healthz" >/dev/null; then break; fi
  if ! kill -0 "$server_pid" 2>/dev/null; then
    echo "conformance: lfm-mirror exited during start-up:" >&2
    tail -n 20 "$work/server.log" >&2
    exit 2
  fi
  sleep 0.1
done

echo "lfm-mirror on port $port, LFM_NOW=$now ($(date -u -d "@$now"))"
status=0
LFM_CONFORMANCE_API_KEY="$key" "$CARGO_TARGET_DIR/debug/lfm-conformance" run \
  --base-url "http://127.0.0.1:$port/2.0/" --report "$report" "$@" || status=$?
echo "report: $report"
exit "$status"
