#!/usr/bin/env bash
# Verifies the supported Unix local-host vertical slice without a provider process.
set -euo pipefail

data_dir="$(mktemp -d "${TMPDIR:-/tmp}/gent-smoke.XXXXXX")"
daemon_pid=""
default_daemon_pid=""

cleanup() {
  if [[ -n "$daemon_pid" ]]; then
    kill -TERM "$daemon_pid" 2>/dev/null || true
    wait "$daemon_pid" 2>/dev/null || true
  fi
  if [[ -n "$default_daemon_pid" ]]; then
    kill -TERM "$default_daemon_pid" 2>/dev/null || true
    wait "$default_daemon_pid" 2>/dev/null || true
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

cargo build --quiet -p gentd -p gent-cli

default_home="$data_dir/home"
mkdir -p "$default_home"
HOME="$default_home" target/debug/gentd --standalone-authority >"$data_dir/default-gentd.log" 2>&1 &
default_daemon_pid="$!"
for _ in $(seq 1 600); do
  [[ -S "$default_home/.gentd/gentd.sock" ]] && break
  sleep 0.05
done
if [[ ! -S "$default_home/.gentd/gentd.sock" ]]; then
  cat "$data_dir/default-gentd.log" >&2 || true
  exit 1
fi
HOME="$default_home" target/debug/gent --no-autostart status >"$data_dir/default-status.json"

target/debug/gentd --data-dir "$data_dir" --standalone-authority >"$data_dir/gentd.log" 2>&1 &
daemon_pid="$!"

# A fresh dev-profile build can legitimately precede daemon startup. Keep this
# separate from request retries so the smoke check never mistakes compilation
# latency for a failed local IPC listener.
for _ in $(seq 1 600); do
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
run_gent "$data_dir/chat-created.json" chat create --provider claude --model haiku --effort medium --mode ask
run_gent "$data_dir/conversations.json" conversation list
conversation_id="$(python3 -c 'import json,sys; print(json.load(open(sys.argv[1]))["body"]["conversationId"])' "$data_dir/chat-created.json")"
run_gent "$data_dir/chat-queued.json" chat queue --conversation-id "$conversation_id" --text "durable smoke prompt"
printf 'attachment smoke' >"$data_dir/attachment.txt"
run_gent "$data_dir/chat-attached.json" chat queue --conversation-id "$conversation_id" --text "attached smoke prompt" --attach "$data_dir/attachment.txt"
run_gent "$data_dir/transcript.json" chat transcript --conversation-id "$conversation_id"

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
default_status = json.loads((data_dir / "default-status.json").read_text())
chat_created = json.loads((data_dir / "chat-created.json").read_text())
conversations = json.loads((data_dir / "conversations.json").read_text())
queued = json.loads((data_dir / "chat-queued.json").read_text())
attached = json.loads((data_dir / "chat-attached.json").read_text())
transcript = json.loads((data_dir / "transcript.json").read_text())

assert default_status["type"] == "status"
assert status["type"] == "status"
assert receipt["body"]["status"] == "settled"
assert [event["kind"] for event in events["body"]["page"]["events"]] == [
    "commandAccepted",
    "commandSettled",
]
assert decision["type"] == "decisionSubmission"
assert decision["body"]["outcome"] == "accepted"
assert decision["body"]["decision"]["phase"] == "pending"
assert terminal_decision["type"] == "decisionSettlement"
assert terminal_decision["body"]["phase"] == "unprovable"
assert chat_created["body"]["receipt"]["status"] == "settled"
assert len(conversations) == 1
assert conversations[0]["conversationId"] == chat_created["body"]["conversationId"]
assert queued["type"] == "accepted"
assert attached["type"] == "accepted"
assert any(event["kind"] == "userMessage" and event["text"] == "durable smoke prompt" for event in transcript["events"])
assert any(event["kind"] == "userMessage" and event["text"] == "attached smoke prompt" for event in transcript["events"])
PY
