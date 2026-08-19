#!/usr/bin/env bash
set -euo pipefail

ROOT=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
workspace=$(mktemp -d)
trap 'rm -rf -- "$workspace"' EXIT
target_dir="$workspace/isolated-target"

output=$(GENT_COVERAGE_TARGET_DIR="$target_dir" GENT_COVERAGE_MIN_FREE_MB=0 \
  bash "$ROOT/tools/run-coverage.sh" --print-command)
[[ -d "$target_dir" ]]
[[ "$output" == *"--output-path $target_dir/coverage-summary.json"* ]]
[[ "$output" == *"--fail-under-lines 90"* ]]
[[ "$output" == *"--json"* ]]
[[ "$output" == *"--summary-only"* ]]
[[ "$output" == *"--workspace"* ]]
[[ "$output" != *"--target-dir"* ]]
[[ "$output" != *"$ROOT/target"* ]]

if GENT_COVERAGE_TARGET_DIR="$target_dir" GENT_COVERAGE_MIN_FREE_MB=999999999 \
  bash "$ROOT/tools/run-coverage.sh" --print-command >/dev/null 2>&1; then
  echo "coverage preflight accepted an impossibly small volume" >&2
  exit 1
fi
