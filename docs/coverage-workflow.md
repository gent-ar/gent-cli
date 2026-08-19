# Development coverage workflow

The CI coverage gate remains a minimum of 90% line coverage for production
library code. Do not replace it with package-only checks or lower its threshold.
The composition binaries, tests, and testkit remain excluded by the same regex
used in CI.

CI and local verification use `cargo-llvm-cov` 0.6.21. Pinning that version keeps
the wrapper's target-directory and JSON-report behavior reproducible.

`cargo llvm-cov` builds instrumented dependencies and can require substantially
more temporary disk space than ordinary `cargo test`. Use the isolated wrapper
instead of deleting the repository's normal `target/` artifacts:

```sh
GENT_COVERAGE_TARGET_DIR=/Volumes/build-cache/gent-cov \
  bash tools/run-coverage.sh
```

The supplied directory may be on another writable volume. Without that
variable, the wrapper creates a unique temporary target below
`GENT_COVERAGE_TMPDIR`, `TMPDIR`, or `/tmp`, and removes only that directory on
completion. It checks for 4096 MiB free by default; set
`GENT_COVERAGE_MIN_FREE_MB` only when the actual available capacity is known.
`--keep-target` retains an automatically-created target for diagnostics, and
`--print-command` verifies the exact command without building.

The wrapper writes a JSON summary because current `cargo-llvm-cov` requires a
report format with `--summary-only`. To retain it after a temporary build, set
an explicit path outside the target directory:

```sh
GENT_COVERAGE_REPORT_PATH=/Volumes/build-cache/gent-coverage-summary.json \
  bash tools/run-coverage.sh
```

CI runs this same wrapper on its ephemeral worker. Validate it without a
coverage build with:

```sh
bash tools/test-run-coverage.sh
```
