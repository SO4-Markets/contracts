# Build and static-analysis workflows.

.PHONY: check lint lint-all build build-release build-wasm test-wasm bindings inspect list-contracts wasm-sizes wasm-baseline

check:
	cargo check --workspace

lint:
	cargo clippy --workspace -- -D warnings

# Clippy over test code too (`#[cfg(test)]` modules and tests/). Plain `lint`
# never compiles those, which let broken tests rot unnoticed (see AGENTS.md).
lint-all: test-wasm
	cargo clippy --workspace --all-targets -- -D warnings

build: preflight
	stellar contract build

# stellar-cli >= 26 dropped `--release`; release is already the default profile.
build-release: preflight
	stellar contract build --profile release

# Build every deployable contract in $(CONTRACTS) for the deploy target with
# plain cargo (no stellar CLI), as CI does. Don't use
# `cargo build --workspace --target $(WASM_TARGET)`: that also compiles the
# test-only crates, which turns on soroban-sdk `testutils` for wasm and fails.
build-wasm:
	cargo build --target $(WASM_TARGET) --profile $(WASM_PROFILE) $(foreach c,$(CONTRACTS),-p $(subst _,-,$(c)))

# WASM artifacts that test code embeds via `contractimport!`
# (tests/router_and_factory_coverage.rs). Plain cargo — no stellar CLI needed.
test-wasm:
	cargo build -p market-token --target $(WASM_TARGET) --profile $(WASM_PROFILE)

# Regenerate TypeScript bindings for every deployable contract from the local
# WASM (no network needed). Run after `make build` whenever a public
# entrypoint or contract type changes. Output: bindings/<contract-name>/.
bindings: build
	@for c in $(CONTRACTS); do \
	  out="bindings/$$(printf '%s' "$$c" | tr '_' '-')"; \
	  printf 'bindings  %s -> %s\n' "$$c" "$$out"; \
	  stellar contract bindings typescript --wasm "$(WASM_DIR)/$$c.wasm" --output-dir "$$out" --overwrite >/dev/null; \
	done

inspect: preflight
	@test -n "$(CONTRACT)" || { printf '%s\n' 'Usage: make inspect CONTRACT=deposit_handler'; exit 1; }
	stellar contract inspect --wasm "$(WASM_DIR)/$(CONTRACT).wasm"

list-contracts:
	@printf '%s\n' $(CONTRACTS)

# Print optimised WASM size for every contract (issue #301).
# Run after `make build-release` and the wasm-opt pass below.
wasm-sizes:
	@printf '%-40s %s\n' 'Contract' 'Size (bytes)'
	@printf '%-40s %s\n' '--------' '------------'
	@for f in $(WASM_DIR)/*.wasm; do \
	  name=$$(basename "$$f" .wasm); \
	  size=$$(wc -c < "$$f"); \
	  printf '%-40s %s\n' "$$name" "$$size"; \
	done

# Build, optimise with wasm-opt, then regenerate docs/build-baseline.json (issue #301).
# Requires: wasm-opt (brew install binaryen  /  apt install binaryen)
wasm-baseline:
	@cargo build --workspace --target $(WASM_TARGET) --profile $(WASM_PROFILE)
	@for f in $(WASM_DIR)/*.wasm; do \
	  wasm-opt -O3 -o "$$f" "$$f"; \
	done
	@WASM_DIR="$(WASM_DIR)" python3 scripts/gen_baseline.py > docs/build-baseline.json
	@printf '%s\n' 'Baseline written to docs/build-baseline.json'
