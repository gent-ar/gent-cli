SHELL := /bin/bash

.PHONY: bootstrap-dev-claurst-runtime bootstrap-release-claurst-runtime clean-spaces verify-live-drivers

MAKEFILE_DIR := $(dir $(abspath $(lastword $(MAKEFILE_LIST))))
GENT_CARGO_CACHE := $(MAKEFILE_DIR).cargo-cache

bootstrap-dev-claurst-runtime:

	cd "$(MAKEFILE_DIR)" && python3 tools/bootstrap-dev-claurst-runtime.py

bootstrap-release-claurst-runtime:

	cd "$(MAKEFILE_DIR)" && python3 tools/bootstrap-dev-claurst-runtime.py --profile release

clean-spaces:
	@test -f "$(MAKEFILE_DIR)Cargo.toml"
	cd "$(MAKEFILE_DIR)" && env -u CARGO_TARGET_DIR cargo clean
	rm -rf -- "$(GENT_CARGO_CACHE)"

# Local-only: replays a real transcript from your own authenticated Claude/Codex CLI
# through the real parser. Never run in CI — it makes real, billed calls against your
# own subscription. See tools/verify-live-driver-parsing.py for what this does and does
# not cover (Claurst is not covered yet; see that file's docstring).
verify-live-drivers:
	cd "$(MAKEFILE_DIR)" && python3 tools/verify-live-driver-parsing.py claude codex
