# Build Guide & WASM Size Budget

**Issue:** [#301](https://github.com/SO4-Markets/contracts/issues/301)

## Prerequisites

```bash
# Install the wasm32 target
rustup target add wasm32-unknown-unknown

# Install wasm-opt (part of the Binaryen toolchain)
# macOS
brew install binaryen

# Debian/Ubuntu
apt-get install binaryen
```

## Building contracts

```bash
# Build all contracts for release
cargo build --target wasm32-unknown-unknown --release

# Optimise with wasm-opt (required step before measuring size or deploying)
for f in target/wasm32-unknown-unknown/release/*.wasm; do
  wasm-opt -O3 -o "$f" "$f"
done
```

## WASM size baseline

> The table below is a local reference snapshot, generated with `make wasm-sizes`.
> It is **not** read by CI — see "How the CI size check actually works" below.

| Contract | Optimised WASM size (bytes) | Notes |
|---|---|---|
| `data_store` | — | Run `make wasm-sizes` to populate |
| `oracle` | — | |
| `role_store` | — | |
| `market_factory` | — | |
| `market_token` | — | |
| `deposit_handler` | — | |
| `deposit_vault` | — | |
| `withdrawal_handler` | — | |
| `withdrawal_vault` | — | |
| `order_handler` | — | Largest contract; monitor closely |
| `order_vault` | — | |
| `order_cleanup` | — | |
| `fee_handler` | — | |
| `fee_batch_sweeper` | — | |
| `liquidation_handler` | — | |
| `adl_handler` | — | |
| `reader` | — | |
| `market_util_reader` | — | |
| `referral_storage` | — | |
| `insurance_fund_router` | — | |
| `exchange_router` | — | |
| `test_faucet` | — | Test-only; excluded from budget |
| `test_token` | — | Test-only; excluded from budget |

Run `make wasm-sizes` (see `Makefile`) to fill in the table for your own reference.

## How the CI size check actually works

The `WASM Size Budget` workflow (`.github/workflows/wasm-size.yml`) does **not** compare
against any committed baseline file — there is no such file in this repo. Instead, for
every PR it:

1. Checks out the PR branch, builds and `wasm-opt`s every contract, and records each
   contract's optimised size.
2. Checks out the PR's base branch (e.g. `main`), rebuilds and re-optimises the same
   contracts, and records those sizes too.
3. Diffs the two sets of sizes directly and posts the result as a PR comment.

## Size budget rules

| Threshold | Action |
|---|---|
| < +5% growth vs the base branch | Pass silently |
| +5% – +10% growth vs the base branch | PR author receives a warning comment; merge is not blocked |
| > +10% growth vs the base branch | CI step fails; PR cannot merge until size is reduced |

## Intentional size increases

Because CI always rebuilds and compares live against the current base branch, there is
no baseline file to regenerate or commit. If a size increase is intentional (e.g. a
significant new feature), there is nothing extra to do for the size check itself —
simply keep the growth within the thresholds above, or explain the increase in the PR
description if it trips the warning/block threshold so reviewers have context.

## Makefile targets

```makefile
wasm-sizes:
	@echo "Contract\tSize (bytes)"
	@for f in target/wasm32-unknown-unknown/release/*.wasm; do \
	  name=$$(basename $$f .wasm); \
	  size=$$(wc -c < $$f); \
	  echo "$$name\t$$size"; \
	done

wasm-baseline:
	@cargo build --target wasm32-unknown-unknown --release 2>/dev/null
	@for f in target/wasm32-unknown-unknown/release/*.wasm; do \
	  wasm-opt -O3 -o "$$f" "$$f"; \
	done
	@python3 scripts/gen_baseline.py > docs/build-baseline.json
```
