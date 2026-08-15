#!/usr/bin/env bash
# Verifies the supported Unix local-host vertical slice without a provider process.
set -euo pipefail

data_dir="$(mktemp -d "${TMPDIR:-/tmp}/gent-smoke.XXXXXX")"
daemon_pid=""

cleanup() {
  if [[ -n "$daemon_pid" ]]; then
    kill -TERM "$daemon_pid" 2>/dev/null || true
    wait "$daemon_pid" 2>/dev/null || true
  fi
  rm -rf "$data_dir"
}
trap cleanup EXIT

cargo run --quiet -p gentd -- --data-dir "$data_dir" >"$data_dir/gentd.log" 2>&1 &
daemon_pid="$!"

for _ in $(seq 1 40); do
  [[ -S "$data_dir/gentd.sock" ]] && break
  sleep 0.05
done
[[ -S "$data_dir/gentd.sock" ]]

GENT_DATA_DIR="$data_dir" cargo run --quiet -p gent-cli -- status >"$data_dir/status.json"
GENT_DATA_DIR="$data_dir" cargo run --quiet -p gent-cli -- submit --kind ping --payload '{"message":"smoke"}' >"$data_dir/receipt.json"
GENT_DATA_DIR="$data_dir" cargo run --quiet -p gent-cli -- events >"$data_dir/events.json"

python3 - "$data_dir" <<'PY'
import json
import pathlib
import sys

data_dir = pathlib.Path(sys.argv[1])
status = json.loads((data_dir / "status.json").read_text())
receipt = json.loads((data_dir / "receipt.json").read_text())
events = json.loads((data_dir / "events.json").read_text())

assert status["type"] == "status"
assert receipt["body"]["status"] == "settled"
assert [event["kind"] for event in events["body"]["events"]] == [
    "commandAccepted",
    "commandSettled",
]
PY
