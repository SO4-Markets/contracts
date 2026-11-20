# Oracle / keeper price-submission workflows.
#
# All targets read ORACLE, TOKEN, MIN_PRICE, and MAX_PRICE from the environment
# or from .deployed/<network>.env.  They NEVER fall back to placeholder values;
# if a required variable is unset the script exits non-zero immediately.
#
# The SOURCE key must hold the ORDER_KEEPER role.  Grant it with:
#   make grant-keeper SOURCE=admin KEEPER=alice NETWORK=testnet

.PHONY: submit-prices grant-keeper

# Submit a single token price through the oracle's set_prices_simple entrypoint.
#
# Required env vars (no defaults):
#   ORACLE      Deployed oracle contract address.
#   TOKEN       Token contract address to price.
#   MIN_PRICE   Minimum price, FLOAT_PRECISION-scaled integer (10^30).
#   MAX_PRICE   Maximum price, FLOAT_PRECISION-scaled integer (10^30).
#
# Example:
#   ORACLE=C... TOKEN=C... MIN_PRICE=500000000000000000000000000000000 \
#   MAX_PRICE=500500000000000000000000000000000 \
#     make submit-prices NETWORK=testnet SOURCE=alice
submit-prices: preflight
	@test -n "$(ORACLE)"    || { printf '%s\n' 'ORACLE is not set.  Export the deployed oracle address.'; exit 1; }
	@test -n "$(TOKEN)"     || { printf '%s\n' 'TOKEN is not set.  Export the token contract address.'; exit 1; }
	@test -n "$(MIN_PRICE)" || { printf '%s\n' 'MIN_PRICE is not set.'; exit 1; }
	@test -n "$(MAX_PRICE)" || { printf '%s\n' 'MAX_PRICE is not set.'; exit 1; }
	ORACLE="$(ORACLE)" TOKEN="$(TOKEN)" MIN_PRICE="$(MIN_PRICE)" MAX_PRICE="$(MAX_PRICE)" \
	  bash scripts/submit_prices.sh "$(NETWORK)" "$(SOURCE)"

# Grant ORDER_KEEPER role to a key so it can call set_prices_simple.
#
# Required:
#   KEEPER   Name of the Stellar key to promote (default: alice).
#
# Example:
#   make grant-keeper SOURCE=alice KEEPER=alice NETWORK=testnet
KEEPER ?= alice
grant-keeper: preflight
	@test -f "$(DEPLOY_ENV)" || { printf 'Missing %s. Run make deploy-all first.\n' "$(DEPLOY_ENV)"; exit 1; }
	source "$(DEPLOY_ENV)"
	keeper_addr="$$(stellar keys address "$(KEEPER)")"
	stellar contract invoke \
		--id "$$ROLE_STORE" \
		--source "$(SOURCE)" \
		--network "$(NETWORK)" \
		-- grant_role \
		--caller "$$(stellar keys address "$(SOURCE)")" \
		--account "$$keeper_addr" \
		--role "$(ORDER_KEEPER_ROLE)"
	printf 'Granted ORDER_KEEPER to %s (%s)\n' "$(KEEPER)" "$$keeper_addr"
