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

report_failure() {
  status=$?
  printf 'Unix IPC smoke failed (exit %s). Daemon log follows:\n' "$status" >&2
  cat "$data_dir/gentd.log" >&2 || true
  exit "$status"
}
trap report_failure ERR

cargo run --quiet -p gentd -- --data-dir "$data_dir" >"$data_dir/gentd.log" 2>&1 &
daemon_pid="$!"

for _ in $(seq 1 120); do
  [[ -S "$data_dir/gentd.sock" ]] && break
  sleep 0.05
done
if [[ ! -S "$data_dir/gentd.sock" ]]; then
  cat "$data_dir/gentd.log" >&2 || true
  exit 1
fi

run_gent() {
  local output=$1
  shift
  local error="${output%.json}.err"
  for _ in $(seq 1 40); do
    if GENT_DATA_DIR="$data_dir" cargo run --quiet -p gent-cli -- --no-autostart "$@" >"$output" 2>"$error"; then
      return
    fi
    sleep 0.05
  done
  cat "$error" >&2 || true
  return 1
}

run_gent "$data_dir/status.json" status
run_gent "$data_dir/receipt.json" submit --kind ping --payload '{"message":"smoke"}' --idempotency-key smoke-ping
run_gent "$data_dir/events.json" events
run_gent "$data_dir/decision.json" decision submit --decision-id smoke-decision --idempotency-key smoke-key
run_gent "$data_dir/decision-terminal.json" decision unprovable --decision-id smoke-decision

python3 - "$data_dir" <<'PY'
import json
import pathlib
import sys

data_dir = pathlib.Path(sys.argv[1])
status = json.loads((data_dir / "status.json").read_text())
receipt = json.loads((data_dir / "receipt.json").read_text())
events = json.loads((data_dir / "events.json").read_text())
decision = json.loads((data_dir / "decision.json").read_text())
terminal_decision = json.loads((data_dir / "decision-terminal.json").read_text())

assert status["type"] == "status"
assert receipt["body"]["status"] == "settled"
assert [event["kind"] for event in events["body"]["events"]] == [
    "commandAccepted",
    "commandSettled",
]
assert decision["type"] == "decisionSubmission"
assert decision["body"]["outcome"] == "accepted"
assert decision["body"]["decision"]["phase"] == "pending"
assert terminal_decision["type"] == "decisionSettlement"
assert terminal_decision["body"]["phase"] == "unprovable"
PY
