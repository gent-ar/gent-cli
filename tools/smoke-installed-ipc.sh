#!/usr/bin/env bash
# Verifies a paired installed Gent release through its public launchers.
set -euo pipefail

bin_dir=${GENT_BIN_DIR:?GENT_BIN_DIR must name the installed Gent bin directory}
expected_version=${GENT_EXPECTED_VERSION:-}
data_dir="$(mktemp -d "${TMPDIR:-/tmp}/gent-installed-smoke.XXXXXX")"
daemon_pid=""

cleanup() {
  if [[ -n "$daemon_pid" ]]; then
    kill -TERM "$daemon_pid" 2>/dev/null || true
    wait "$daemon_pid" 2>/dev/null || true
  fi
  rm -rf "$data_dir"
}
trap cleanup EXIT

fail() {
  printf 'Installed Gent smoke failed: %s\n' "$1" >&2
  cat "$data_dir/gentd.log" >&2 || true
  exit 1
}

for binary in gent gentd; do
  [[ -f "$bin_dir/$binary" && -x "$bin_dir/$binary" ]] || fail "missing executable $binary"
done

gent_version="$("$bin_dir/gent" --version)" || fail "gent --version failed"
gentd_version="$("$bin_dir/gentd" --version)" || fail "gentd --version failed"
[[ "$gent_version" == gent\ * && "$gentd_version" == gentd\ * ]] || fail "invalid version output"
if [[ -n "$expected_version" ]]; then
  [[ "$gent_version" == *"$expected_version" && "$gentd_version" == *"$expected_version" ]] || fail "installed pair does not match expected version"
fi
"$bin_dir/gent" --data-dir "$data_dir" update auto status >"$data_dir/auto-update.json" || fail "installed automatic update status failed"
python3 - "$data_dir/auto-update.json" <<'PY'
import json
import pathlib
import sys

status = json.loads(pathlib.Path(sys.argv[1]).read_text(encoding="utf-8"))
assert status["schemaVersion"] == 1
assert status["enabled"] is True
PY

"$bin_dir/gentd" --data-dir "$data_dir" >"$data_dir/gentd.log" 2>&1 &
daemon_pid="$!"
for _ in $(seq 1 200); do
  [[ -S "$data_dir/gentd.sock" ]] && break
  sleep 0.05
done
[[ -S "$data_dir/gentd.sock" ]] || fail "daemon socket was not created"

"$bin_dir/gent" --data-dir "$data_dir" --no-autostart status >"$data_dir/status.json" || fail "installed CLI status failed"
python3 - "$data_dir/status.json" <<'PY'
import json
import pathlib
import sys

status = json.loads(pathlib.Path(sys.argv[1]).read_text(encoding="utf-8"))
assert status["type"] == "status"
assert status["body"]["hostEpoch"] >= 1
PY
