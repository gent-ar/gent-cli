SHELL := /bin/bash

.PHONY: clean-spaces

clean-spaces:
	cargo clean
	rm -rf target
	rm -rf .cargo-cache
