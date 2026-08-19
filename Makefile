SHELL := /bin/bash

.PHONY: clean-spaces

clean-spaces:
	cargo clean
	rm -rf .cargo-cache
