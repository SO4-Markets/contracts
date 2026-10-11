# Oracle / keeper price-submission workflows.
#
# All targets read ORACLE, TOKEN, MIN_PRICE, and MAX_PRICE from the environment
# or from .deployed/<network>.env.  They NEVER fall back to placeholder values;
# if a required variable is unset the script exits non-zero immediately.
#
# The SOURCE key must hold the ORDER_KEEPER role.  Grant it with:
#   make grant-keeper SOURCE=admin KEEPER=alice NETWORK=testnet

.PHONY: submit-prices grant-keeper

# Submit a single token price through the oracle's real signed set_prices entrypoint.
#
# Required env vars:
#   ORACLE      Deployed oracle contract address.
#   TOKEN       Token contract address (or symbol) to price.
#
# Optional env vars:
#   ORACLE_URL  Oracle worker URL (default: https://oracle.biscotti-proxy-worker.workers.dev).
#   MIN_PRICE   Minimum price (required if SIGNATURE is passed).
#   MAX_PRICE   Maximum price (required if SIGNATURE is passed).
#   SIGNATURE   64-byte hex ed25519 signature from authorized keeper.
#
# Example (via keeper oracle feed):
#   ORACLE=C... TOKEN=TUSDC make submit-prices NETWORK=testnet SOURCE=alice
#
# Example (via explicit signed price):
#   ORACLE=C... TOKEN=C... MIN_PRICE=500000000000000000000000000000000 \
#   MAX_PRICE=500500000000000000000000000000000 SIGNATURE=... \
#     make submit-prices NETWORK=testnet SOURCE=alice
submit-prices: preflight
	@test -n "$(ORACLE)" || { printf '%s\n' 'ORACLE is not set.  Export the deployed oracle address.'; exit 1; }
	@test -n "$(TOKEN)"  || { printf '%s\n' 'TOKEN is not set.  Export the token contract address or symbol.'; exit 1; }
	ORACLE="$(ORACLE)" TOKEN="$(TOKEN)" MIN_PRICE="$(MIN_PRICE)" MAX_PRICE="$(MAX_PRICE)" SIGNATURE="$(SIGNATURE)" \
	  bash scripts/submit_prices.sh "$(NETWORK)" "$(SOURCE)"

# Grant ORDER_KEEPER role to a key so it can call set_prices.
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
