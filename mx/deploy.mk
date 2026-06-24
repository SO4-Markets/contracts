# Deployment workflows.

DRY_RUN ?= 0

.PHONY: deploy deploy-all deploy-contract deploy-force deploy-testnet deploy-mainnet deploy-local addresses

deploy: deploy-all

deploy-all: preflight
	CONTRACT="$(CONTRACT)" DRY_RUN="$(DRY_RUN)" bash scripts/deploy.sh "$(NETWORK)" "$(SOURCE)"

deploy-force: preflight
	CONTRACT="$(CONTRACT)" FORCE=1 DRY_RUN="$(DRY_RUN)" bash scripts/deploy.sh "$(NETWORK)" "$(SOURCE)"

deploy-contract: preflight build
	@test -n "$(CONTRACT)" || { printf '%s\n' 'Usage: make deploy-contract CONTRACT=reader NETWORK=testnet SOURCE=deployer'; exit 1; }
	@test -f "$(WASM_DIR)/$(CONTRACT).wasm" || { printf 'Missing wasm: %s/%s.wasm\n' "$(WASM_DIR)" "$(CONTRACT)"; exit 1; }
	wasm_hash="$$(stellar contract upload --wasm "$(WASM_DIR)/$(CONTRACT).wasm" --source "$(SOURCE)" --network "$(NETWORK)")"
	contract_id="$$(stellar contract deploy --wasm-hash "$$wasm_hash" --source "$(SOURCE)" --network "$(NETWORK)")"
	printf '%s deployed at %s\n' "$(CONTRACT)" "$$contract_id"
	printf '%s\n' 'This standalone deploy did not update $(DEPLOY_ENV). Initialize and wire it manually, or use upgrade-contract for an existing protocol deployment.'

deploy-testnet:
	$(MAKE) deploy-all NETWORK=testnet SOURCE="$(SOURCE)"

deploy-mainnet:
	$(MAKE) deploy-all NETWORK=mainnet SOURCE="$(SOURCE)"

deploy-local:
	$(MAKE) deploy-all NETWORK=local SOURCE="$(SOURCE)"

addresses:
	@test -f "$(DEPLOY_ENV)" || { printf 'Missing %s. Run make deploy-all first.\n' "$(DEPLOY_ENV)"; exit 1; }
	@sed -n '1,220p' "$(DEPLOY_ENV)"

# ── Post-deployment bootstrap ─────────────────────────────────────────────────
#
# Runs after deploy-all to:
#   1. Grant keeper roles to SOURCE (or KEEPER=<key>)
#   2. Create a market via market_factory
#   3. Set per-market config keys in data_store
#   4. Print instructions for seeding liquidity
#
# Example (full fresh testnet bootstrap):
#   make token-bootstrap CODE=TWBTC NETWORK=testnet SOURCE=alice
#   make token-bootstrap CODE=TUSDC NETWORK=testnet SOURCE=alice
#   make deploy-all NETWORK=testnet SOURCE=alice
#   make bootstrap NETWORK=testnet SOURCE=alice LONG_CODE=TWBTC SHORT_CODE=TUSDC

KEEPER ?= $(SOURCE)
LONG_CODE ?= TWBTC
SHORT_CODE ?= TUSDC

.PHONY: bootstrap market-init seed-liquidity

bootstrap: preflight
	KEEPER="$(KEEPER)" LONG_CODE="$(LONG_CODE)" SHORT_CODE="$(SHORT_CODE)" \
	bash scripts/bootstrap.sh "$(NETWORK)" "$(SOURCE)"

market-init: preflight
	KEEPER="$(KEEPER)" LONG_CODE="$(LONG_CODE)" SHORT_CODE="$(SHORT_CODE)" \
	SKIP_ROLES=1 SKIP_CONFIG=0 SKIP_SEED=1 \
	bash scripts/bootstrap.sh "$(NETWORK)" "$(SOURCE)"

seed-liquidity: preflight
	@test -f "$(DEPLOY_ENV)" || { printf 'Missing %s. Run make deploy-all first.\n' "$(DEPLOY_ENV)"; exit 1; }
	@. "$(DEPLOY_ENV)"; \
	test -n "$$MARKET_TOKEN" || { printf 'MARKET_TOKEN not set in %s. Run make bootstrap first.\n' "$(DEPLOY_ENV)"; exit 1; }; \
	printf 'Seed LONG_TOKEN  (%s) amount=%s\n' "$(LONG_CODE)"  "$(SEED_LONG)"; \
	printf 'Seed SHORT_TOKEN (%s) amount=%s\n' "$(SHORT_CODE)" "$(SEED_SHORT)"; \
	printf 'Edit scripts/bootstrap.sh SKIP_SEED section or call deposit_handler directly.\n'
