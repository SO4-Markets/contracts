# Local and network-oriented test workflows.

.PHONY: test test-one test-snap testnet-smoke

test:
	cargo test --workspace

test-one:
	@test -n "$(PACKAGE)" || { printf '%s\n' 'Usage: make test-one PACKAGE=deposit-handler'; exit 1; }
	cargo test -p "$(PACKAGE)"

test-snap:
	INSTA_UPDATE=always cargo test --workspace

testnet-smoke: preflight
	@test -f "$(DEPLOY_ENV)" || { printf 'Missing %s. Run make deploy-all first.\n' "$(DEPLOY_ENV)"; exit 1; }
	DEPLOY_ENV="$(DEPLOY_ENV)" TOKEN_ENV="$(TOKEN_ENV)" \
		bash scripts/smoke_test.sh "$(NETWORK)" "$(SOURCE)"

smoke-prices:
	@bash scripts/submit_prices.sh
