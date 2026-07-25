# Build and static-analysis workflows.

.PHONY: check lint build build-release inspect list-contracts wasm-sizes wasm-baseline

check:
	cargo check --workspace

lint:
	cargo clippy --workspace -- -D warnings

build: preflight
	stellar contract build

build-release: preflight
	stellar contract build --release

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
	@for f in target/wasm32-unknown-unknown/release/*.wasm; do \
	  name=$$(basename "$$f" .wasm); \
	  size=$$(wc -c < "$$f"); \
	  printf '%-40s %s\n' "$$name" "$$size"; \
	done

# Build, optimise with wasm-opt, then regenerate docs/build-baseline.json (issue #301).
# Requires: wasm-opt (brew install binaryen  /  apt install binaryen)
wasm-baseline:
	@cargo build --target wasm32-unknown-unknown --release
	@for f in target/wasm32-unknown-unknown/release/*.wasm; do \
	  wasm-opt -O3 -o "$$f" "$$f"; \
	done
	@python3 scripts/gen_baseline.py > docs/build-baseline.json
	@printf '%s\n' 'Baseline written to docs/build-baseline.json'
