#!/usr/bin/env bash
# The multi-user conformance suite: Last.fm answers saved during a live comparison for ten users
# (fixtures-multi/), replayed against lfm-mirror serving a copy of the database that comparison
# ran on. Each fixtures-multi/<user>.<format>/ group runs against its own server start, with
# the clock frozen at the group's first request (its `now` file).
#
#   scripts/conformance-multi.sh prepare            # once: build data/conformance-multi.db
#   scripts/conformance-multi.sh                    # every case
#   scripts/conformance-multi.sh --only user.getTopAlbums   # extra args go to the harness
#   scripts/conformance-multi.sh --expect conformance/expected-multi.txt
#
# `prepare` copies lfm.db out of the Docker volume $LFM_MULTI_VOLUME (default lfm-load-data)
# without starting any container that uses it, then removes the scrobbles that reached Last.fm
# after the saved answers (fixtures-multi/cutoffs.txt). The copy is not committed.
#
# Reports go to $CONFORMANCE_REPORT_DIR (default target/conformance-multi), one per group.
set -euo pipefail
cd "$(dirname "$0")/.."

db=data/conformance-multi.db
export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-target/conformance}"

if [ "${1:-}" = "prepare" ]; then
  volume="${LFM_MULTI_VOLUME:-lfm-load-data}"
  mkdir -p data
  rm -f "$db" "$db-wal" "$db-shm"
  docker run --rm -v "$volume:/d:ro" -v "$PWD/data:/o" alpine sh -c \
    "cp /d/lfm.db /o/conformance-multi.db \
     && for s in -wal -shm; do if [ -f /d/lfm.db\$s ]; then cp /d/lfm.db\$s /o/conformance-multi.db\$s; fi; done \
     && chown $(id -u):$(id -g) /o/conformance-multi.db*"
  cargo run --quiet -p lfm-mirror --example trim_history -- "$db" fixtures-multi/cutoffs.txt
  echo "prepared $db"
  exit 0
fi

if [ ! -f "$db" ]; then
  echo "conformance-multi: $db is missing. Run: scripts/conformance-multi.sh prepare" >&2
  exit 2
fi

reports="${CONFORMANCE_REPORT_DIR:-target/conformance-multi}"
cargo build --quiet -p lfm-mirror -p lfm-conformance
rm -rf "$reports"
mkdir -p "$reports"

work=$(mktemp -d)
server_pid=""
stop_server() {
  if [ -n "$server_pid" ]; then
    kill "$server_pid" 2>/dev/null || true
    wait "$server_pid" 2>/dev/null || true
    server_pid=""
  fi
}
trap 'stop_server; rm -rf "$work"' EXIT

# One copy for the whole run; groups go in clock order, as a live server would see the times.
for suffix in "" -wal -shm; do
  if [ -f "$db$suffix" ]; then cp "$db$suffix" "$work/lfm.db$suffix"; fi
done

key=$(od -An -tx1 -N16 /dev/urandom | tr -d ' \n')
status=0
groups=$(for dir in fixtures-multi/*/; do echo "$(cat "$dir/now") $dir"; done | sort -n)
while read -r now dir; do
  port=$(python3 -c 'import socket; s=socket.socket(); s.bind(("127.0.0.1", 0)); print(s.getsockname()[1])')
  LFM_BIND="127.0.0.1:$port" LFM_DATA_DIR="$work" LFM_DB="$work/lfm.db" \
    LFM_NOW="$now" LFM_DEV_API_KEY="$key" RUST_LOG="${RUST_LOG:-warn}" \
    "$CARGO_TARGET_DIR/debug/lfm-mirror" >>"$work/server.log" 2>&1 &
  server_pid=$!
  until curl -sf "http://127.0.0.1:$port/healthz" >/dev/null; do
    if ! kill -0 "$server_pid" 2>/dev/null; then
      echo "conformance-multi: lfm-mirror exited during start-up:" >&2
      tail -n 20 "$work/server.log" >&2
      exit 2
    fi
    sleep 0.1
  done
  group=$(basename "$dir")
  code=0
  LFM_CONFORMANCE_API_KEY="$key" "$CARGO_TARGET_DIR/debug/lfm-conformance" run \
    --base-url "http://127.0.0.1:$port/2.0/" --fixtures "$dir" \
    --ignore conformance/ignore-multi.toml --report "$reports/$group.json" "$@" \
    >"$work/out.txt" 2>&1 || code=$?
  if grep -q 'no fixture matches' "$work/out.txt"; then
    stop_server
    continue
  fi
  echo "== $group (LFM_NOW=$now): $(grep -E '^  [0-9]+ cases' "$work/out.txt" | sed 's/^ *//')"
  grep -E '^(baseline|  regression)' "$work/out.txt" || true
  if [ "$code" -eq 2 ]; then cat "$work/out.txt" >&2; exit 2; fi
  if [ "$code" -ne 0 ]; then status=1; fi
  stop_server
done <<<"$groups"

python3 - "$reports" <<'EOF'
import json, pathlib, sys
cases = [c for p in sorted(pathlib.Path(sys.argv[1]).glob("*.json"))
         for c in json.loads(p.read_text())["cases"]]
count = lambda outcome: sum(c["outcome"] == outcome for c in cases)
print(f"all groups: {len(cases)} cases, {count('exact')} exact, "
      f"{count('ignored')} ignored, {count('failed')} failed")
EOF
exit "$status"
