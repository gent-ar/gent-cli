SHELL := /bin/bash

.PHONY: bootstrap-dev-claurst-runtime bootstrap-release-claurst-runtime clean-spaces

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
