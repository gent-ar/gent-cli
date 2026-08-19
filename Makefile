SHELL := /bin/bash

.PHONY: clean-spaces

MAKEFILE_DIR := $(dir $(abspath $(lastword $(MAKEFILE_LIST))))
GENT_CARGO_CACHE := $(MAKEFILE_DIR).cargo-cache

clean-spaces:
	@test -f "$(MAKEFILE_DIR)Cargo.toml"
	cd "$(MAKEFILE_DIR)" && env -u CARGO_TARGET_DIR cargo clean
	rm -rf -- "$(GENT_CARGO_CACHE)"
