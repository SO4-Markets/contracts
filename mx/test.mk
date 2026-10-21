# Local and network-oriented test workflows.

.PHONY: test test-one test-snap testnet-smoke bench-resources bench-baseline

test: test-wasm
	cargo test --workspace

test-one: test-wasm
	@test -n "$(PACKAGE)" || { printf '%s\n' 'Usage: make test-one PACKAGE=deposit-handler'; exit 1; }
	cargo test -p "$(PACKAGE)"

test-snap: test-wasm
	INSTA_UPDATE=always cargo test --workspace

testnet-smoke: preflight
	@test -f "$(DEPLOY_ENV)" || { printf 'Missing %s. Run make deploy-all first.\n' "$(DEPLOY_ENV)"; exit 1; }
	DEPLOY_ENV="$(DEPLOY_ENV)" TOKEN_ENV="$(TOKEN_ENV)" \
		bash scripts/smoke_test.sh "$(NETWORK)" "$(SOURCE)"

smoke-prices:
	@bash scripts/submit_prices.sh

# Resource benchmark regression gate (#1064). Soroban metering is deterministic,
# so the harness output is compared exactly-ish (per-metric thresholds) against
# the committed baseline. See docs/performance.md.
BENCH_OUT ?= target/bench/resource-bench.json
BENCH_BASELINE ?= docs/benchmarks/resource-baseline.json

bench-resources:
	mkdir -p "$(dir $(BENCH_OUT))"
	RESOURCE_BENCH_OUT="$(abspath $(BENCH_OUT))" cargo test -p gmx-integration-tests --test resource_bench
	python3 scripts/bench_compare.py compare --baseline "$(BENCH_BASELINE)" --current "$(BENCH_OUT)"

# Rewrite the baseline from a fresh run. A rationale is mandatory and is stored
# in the baseline; CI rejects baseline changes that don't update it.
bench-baseline:
	@test -n "$(RATIONALE)" || { printf '%s\n' 'Usage: make bench-baseline RATIONALE="why the numbers changed"'; exit 1; }
	mkdir -p "$(dir $(BENCH_OUT))"
	RESOURCE_BENCH_OUT="$(abspath $(BENCH_OUT))" cargo test -p gmx-integration-tests --test resource_bench
	python3 scripts/bench_compare.py update --baseline "$(BENCH_BASELINE)" --current "$(BENCH_OUT)" --rationale "$(RATIONALE)"
