#!/usr/bin/env bash
# Run the enforced production-library coverage gate in an isolated target directory.
set -euo pipefail

ROOT=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
IGNORE='(^|/)crates/(gent-cli|gentd|gent-testkit)/|/tests/|_tests\.rs$|/src/bin/'
minimum_mb=${GENT_COVERAGE_MIN_FREE_MB:-4096}
target_dir=${GENT_COVERAGE_TARGET_DIR:-}
temporary_target=0
print_only=0
keep_target=0

usage() {
  cat <<'EOF'
Usage: tools/run-coverage.sh [--keep-target] [--print-command]

Runs the unchanged 90% production-library line-coverage gate with cargo-llvm-cov.
Set GENT_COVERAGE_TARGET_DIR to a directory on a volume with enough space. Without
it, the script creates and removes a temporary target directory below
GENT_COVERAGE_TMPDIR, TMPDIR, or /tmp. It never uses or removes the repository's
normal target directory. GENT_COVERAGE_MIN_FREE_MB defaults to 4096.
EOF
}

while (($#)); do
  case "$1" in
    --keep-target) keep_target=1 ;;
    --print-command) print_only=1 ;;
    --help|-h) usage; exit 0 ;;
    *) usage >&2; exit 2 ;;
  esac
  shift
done

cleanup() {
  if ((temporary_target && !keep_target)); then
    rm -rf -- "$target_dir"
  fi
}

if [[ -z "$target_dir" ]]; then
  base_dir=${GENT_COVERAGE_TMPDIR:-${TMPDIR:-/tmp}}
  [[ -d "$base_dir" ]] || { echo "coverage temporary base does not exist: $base_dir" >&2; exit 2; }
  target_dir=$(mktemp -d "$base_dir/gent-llvm-cov.XXXXXX")
  temporary_target=1
  trap cleanup EXIT
else
  mkdir -p -- "$target_dir"
fi

available_kb=$(df -Pk "$target_dir" | awk 'NR == 2 { print $4 }')
required_kb=$((minimum_mb * 1024))
if [[ ! "$available_kb" =~ ^[0-9]+$ ]] || ((available_kb < required_kb)); then
  echo "coverage target needs ${minimum_mb} MiB free; $target_dir has ${available_kb:-unknown} KiB" >&2
  echo "set GENT_COVERAGE_TARGET_DIR to a directory on a larger volume" >&2
  exit 1
fi

command=(
  cargo llvm-cov --target-dir "$target_dir" --workspace --all-targets --all-features
  --summary-only --ignore-filename-regex "$IGNORE" --fail-under-lines 90
)
if ((print_only)); then
  printf '%q ' "${command[@]}"
  printf '\n'
  exit 0
fi

cd "$ROOT"
CARGO_TARGET_DIR="$target_dir" "${command[@]}"
