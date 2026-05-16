# SO4.market — Contract Makefile
#
# Targets:
#   make build              build all contracts to wasm
#   make check              type-check without producing wasm
#   make test               run the full test suite
#   make deploy             deploy all contracts to testnet  (default)
#   make deploy-mainnet     deploy all contracts to mainnet
#   make clean              remove build artefacts and deployed address files
#
# Variables (override on the command line):
#   NETWORK=testnet         target network (testnet | mainnet | local)
#   SOURCE=alice            stellar key name used to sign transactions

NETWORK ?= testnet
SOURCE  ?= alice

.PHONY: all build check lint test deploy deploy-mainnet clean

# ── Default target ─────────────────────────────────────────────────────────────
all: build test

# ── Rust ───────────────────────────────────────────────────────────────────────
check:
	cargo check --workspace

lint:
	cargo clippy --workspace -- -D warnings

test:
	cargo test --workspace

# ── Wasm build ─────────────────────────────────────────────────────────────────
build:
	stellar contract build

# ── Deploy ─────────────────────────────────────────────────────────────────────
# Runs scripts/deploy.sh which:
#   1. Builds all contracts
#   2. Uploads wasm blobs
#   3. Deploys + initialises each contract in dependency order
#   4. Grants CONTROLLER role to all handlers
#   5. Prints a summary table and saves addresses to .deployed/<NETWORK>.env

deploy:
	@bash scripts/deploy.sh $(NETWORK) $(SOURCE)

deploy-mainnet:
	@$(MAKE) deploy NETWORK=mainnet SOURCE=$(SOURCE)

# ── Clean ──────────────────────────────────────────────────────────────────────
clean:
	cargo clean
	rm -rf .deployed
