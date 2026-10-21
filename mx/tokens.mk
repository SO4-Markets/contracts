# Test-token workflows.
#
# Two supported paths:
#
#   1. Native Soroban tokens (PRIMARY for testnet demos / faucet UX):
#      Deploys contracts/test_token + contracts/test_faucet.
#      Users claim tokens from the faucet; no trustlines required.
#      Target: test-tokens-with-faucet  (and the four-token variant: test-tokens-all-with-faucet)
#
#   2. Stellar classic assets / SAC (for production-like collateral plumbing):
#      Wraps a classic Stellar asset in a SAC via the stellar CLI.
#      Requires trustlines and issuer keypairs.
#      Targets: token-bootstrap, market-tokens
#
# Use 7-decimal amounts for SO4.market math:  1 TWBTC = 10000000
#
# IMPORTANT: test tokens must NEVER be deployed to mainnet. The guard-not-mainnet
# target enforces this for all faucet/native-token deploy targets.

CODE ?= TWBTC
ISSUER ?= $(CODE)-issuer
TO ?= $(SOURCE)
AMOUNT ?= 1000000000

.PHONY: token-issuer token-deploy token-id token-trust token-mint token-balance token-bootstrap tokens guard-not-mainnet

guard-not-mainnet:
	@[ "$(NETWORK)" != "mainnet" ] || \
	  { printf 'ERROR: test tokens must not be deployed to mainnet. Use real SAC IDs instead.\n' >&2; exit 1; }

token-issuer: preflight
	@if ! stellar keys address "$(ISSUER)" >/dev/null 2>&1; then \
		stellar keys generate --global "$(ISSUER)" --network "$(NETWORK)"; \
	fi
	@if [ "$(NETWORK)" = "testnet" ]; then stellar keys fund "$(ISSUER)" --network "$(NETWORK)" >/dev/null; fi
	@stellar keys address "$(ISSUER)"

token-deploy: preflight token-issuer
	@mkdir -p "$(DEPLOY_DIR)"
	issuer_addr="$$(stellar keys address "$(ISSUER)")"
	asset="$(CODE):$$issuer_addr"
	contract_id="$$(stellar contract asset deploy --source "$(SOURCE)" --network "$(NETWORK)" --asset "$$asset")"
	printf '%s_ASSET=%s\n' "$(CODE)" "$$asset" >> "$(TOKEN_ENV)"
	printf '%s=%s\n' "$(CODE)" "$$contract_id" >> "$(TOKEN_ENV)"
	printf '%s\n' "$$contract_id"

token-id: preflight token-issuer
	issuer_addr="$$(stellar keys address "$(ISSUER)")"
	stellar contract id asset --network "$(NETWORK)" --asset "$(CODE):$$issuer_addr"

token-trust: preflight token-issuer
	issuer_addr="$$(stellar keys address "$(ISSUER)")"
	if [ "$(NETWORK)" = "testnet" ]; then stellar keys fund "$(TO)" --network "$(NETWORK)" >/dev/null || true; fi
	stellar tx new change-trust --source "$(TO)" --network "$(NETWORK)" --line "$(CODE):$$issuer_addr"

token-mint: preflight token-issuer
	issuer_addr="$$(stellar keys address "$(ISSUER)")"
	contract_id="$$(stellar contract id asset --network "$(NETWORK)" --asset "$(CODE):$$issuer_addr")"
	stellar contract invoke \
		--id "$$contract_id" \
		--source "$(ISSUER)" \
		--network "$(NETWORK)" \
		-- mint --to "$(TO)" --amount "$(AMOUNT)"

token-balance: preflight token-issuer
	issuer_addr="$$(stellar keys address "$(ISSUER)")"
	contract_id="$$(stellar contract id asset --network "$(NETWORK)" --asset "$(CODE):$$issuer_addr")"
	stellar contract invoke \
		--id "$$contract_id" \
		--source "$(SOURCE)" \
		--network "$(NETWORK)" \
		-- balance --id "$(TO)"

token-bootstrap: token-deploy token-trust token-mint token-balance

# Bootstrap all market tokens for a standard testnet deployment.
# Creates TWBTC and TUSDC, mints an initial amount to SOURCE.
#
# Usage:
#   make market-tokens NETWORK=testnet SOURCE=alice
#   make market-tokens LONG_CODE=TWBTC SHORT_CODE=TUSDC NETWORK=testnet SOURCE=alice

LONG_CODE  ?= TWBTC
SHORT_CODE ?= TUSDC
SEED_LONG  ?= 1000000000
SEED_SHORT ?= 1000000000

.PHONY: market-tokens

market-tokens: preflight
	$(MAKE) token-bootstrap CODE="$(LONG_CODE)"  TO="$(SOURCE)" AMOUNT="$(SEED_LONG)"  NETWORK="$(NETWORK)" SOURCE="$(SOURCE)"
	$(MAKE) token-bootstrap CODE="$(SHORT_CODE)" TO="$(SOURCE)" AMOUNT="$(SEED_SHORT)" NETWORK="$(NETWORK)" SOURCE="$(SOURCE)"
	@printf 'Market tokens ready. Run:\n'
	@printf '  make deploy-all NETWORK=$(NETWORK) SOURCE=$(SOURCE)\n'
	@printf '  make bootstrap  NETWORK=$(NETWORK) SOURCE=$(SOURCE) LONG_CODE=$(LONG_CODE) SHORT_CODE=$(SHORT_CODE)\n'

tokens:
	@test -f "$(TOKEN_ENV)" || { printf 'Missing %s. Run make token-bootstrap first.\n' "$(TOKEN_ENV)"; exit 1; }
	@sed -n '1,220p' "$(TOKEN_ENV)"

# ── Native Soroban test-token + faucet workflows ─────────────────────────────
#
# These targets deploy the repo-local mintable test_token contract rather than
# Stellar classic assets / SACs. They are useful for app demos and faucets. For
# mainnet collateral, use real SAC IDs instead.

TOKEN_NAME ?= Test $(CODE)
TOKEN_DECIMALS ?= 7
CLAIM_AMOUNT ?= 1000000000
FAUCET_COOLDOWN ?= 17280
FAUCET ?=

.PHONY: faucet-deploy test-token-deploy faucet-add-token faucet-claim faucet-claim-market test-tokens-with-faucet test-tokens-all-with-faucet

faucet-deploy: preflight build guard-not-mainnet
	@mkdir -p "$(DEPLOY_DIR)"
	admin_addr="$$(stellar keys address "$(SOURCE)")"
	wasm_hash="$$(stellar contract upload --wasm "$(WASM_DIR)/test_faucet.wasm" --source "$(SOURCE)" --network "$(NETWORK)")"
	faucet_id="$$(stellar contract deploy --wasm-hash "$$wasm_hash" --source "$(SOURCE)" --network "$(NETWORK)")"
	stellar contract invoke \
		--id "$$faucet_id" \
		--source "$(SOURCE)" \
		--network "$(NETWORK)" \
		-- initialize --admin "$$admin_addr" --cooldown_ledgers "$(FAUCET_COOLDOWN)" >/dev/null
	tmp="$$(mktemp)"
	{ grep -v '^FAUCET=' "$(TOKEN_ENV)" 2>/dev/null || true; printf 'FAUCET=%s\n' "$$faucet_id"; } > "$$tmp"
	mv "$$tmp" "$(TOKEN_ENV)"
	printf '%s\n' "$$faucet_id"

test-token-deploy: preflight build guard-not-mainnet
	@test -n "$(FAUCET)" || { printf '%s\n' 'Usage: make test-token-deploy CODE=TWBTC FAUCET=C... NETWORK=testnet SOURCE=alice'; exit 1; }
	@mkdir -p "$(DEPLOY_DIR)"
	wasm_hash="$$(stellar contract upload --wasm "$(WASM_DIR)/test_token.wasm" --source "$(SOURCE)" --network "$(NETWORK)")"
	token_id="$$(stellar contract deploy --wasm-hash "$$wasm_hash" --source "$(SOURCE)" --network "$(NETWORK)")"
	stellar contract invoke \
		--id "$$token_id" \
		--source "$(SOURCE)" \
		--network "$(NETWORK)" \
		-- initialize --owner "$(FAUCET)" --decimal "$(TOKEN_DECIMALS)" --name "$(TOKEN_NAME)" --symbol "$(CODE)" >/dev/null
	tmp="$$(mktemp)"
	{ grep -v -e '^$(CODE)=' -e '^$(CODE)_NATIVE=' "$(TOKEN_ENV)" 2>/dev/null || true; printf '%s=%s\n' "$(CODE)" "$$token_id"; printf '%s_NATIVE=%s\n' "$(CODE)" "$$token_id"; } > "$$tmp"
	mv "$$tmp" "$(TOKEN_ENV)"
	printf '%s\n' "$$token_id"

faucet-add-token: preflight
	@test -n "$(FAUCET)" || { printf '%s\n' 'Usage: make faucet-add-token FAUCET=C... TOKEN=C... CLAIM_AMOUNT=1000000000'; exit 1; }
	@test -n "$(TOKEN)" || { printf '%s\n' 'Usage: make faucet-add-token FAUCET=C... TOKEN=C... CLAIM_AMOUNT=1000000000'; exit 1; }
	admin_addr="$$(stellar keys address "$(SOURCE)")"
	stellar contract invoke \
		--id "$(FAUCET)" \
		--source "$(SOURCE)" \
		--network "$(NETWORK)" \
		-- set_token --caller "$$admin_addr" --token "$(TOKEN)" --claim_amount "$(CLAIM_AMOUNT)"

faucet-claim: preflight
	@test -n "$(FAUCET)" || { printf '%s\n' 'Usage: make faucet-claim FAUCET=C... TOKEN=C... TO=alice NETWORK=testnet SOURCE=alice'; exit 1; }
	@test -n "$(TOKEN)" || { printf '%s\n' 'Usage: make faucet-claim FAUCET=C... TOKEN=C... TO=alice NETWORK=testnet SOURCE=alice'; exit 1; }
	to_addr="$$(stellar keys address "$(TO)" 2>/dev/null || printf '%s' "$(TO)")"
	stellar contract invoke \
		--id "$(FAUCET)" \
		--source "$(TO)" \
		--network "$(NETWORK)" \
		-- claim --account "$$to_addr" --token "$(TOKEN)"

faucet-claim-market: preflight
	@test -f "$(TOKEN_ENV)" || { printf 'Missing %s. Run make test-tokens-with-faucet first.\n' "$(TOKEN_ENV)"; exit 1; }
	@. "$(TOKEN_ENV)"; \
	test -n "$$FAUCET" || { printf 'FAUCET not set in %s.\n' "$(TOKEN_ENV)"; exit 1; }; \
	test -n "$$$(LONG_CODE)" || { printf '$(LONG_CODE) not set in %s.\n' "$(TOKEN_ENV)"; exit 1; }; \
	test -n "$$$(SHORT_CODE)" || { printf '$(SHORT_CODE) not set in %s.\n' "$(TOKEN_ENV)"; exit 1; }; \
	$(MAKE) faucet-claim FAUCET="$$FAUCET" TOKEN="$$$(LONG_CODE)" TO="$(TO)" NETWORK="$(NETWORK)" SOURCE="$(SOURCE)"; \
	$(MAKE) faucet-claim FAUCET="$$FAUCET" TOKEN="$$$(SHORT_CODE)" TO="$(TO)" NETWORK="$(NETWORK)" SOURCE="$(SOURCE)"

test-tokens-with-faucet: preflight guard-not-mainnet
	@mkdir -p "$(DEPLOY_DIR)"
	faucet_id="$$($(MAKE) --no-print-directory faucet-deploy NETWORK="$(NETWORK)" SOURCE="$(SOURCE)" FAUCET_COOLDOWN="$(FAUCET_COOLDOWN)" | tail -n 1)"
	long_id="$$($(MAKE) --no-print-directory test-token-deploy CODE="$(LONG_CODE)" TOKEN_NAME="Test $(LONG_CODE)" TOKEN_DECIMALS="$(TOKEN_DECIMALS)" FAUCET="$$faucet_id" NETWORK="$(NETWORK)" SOURCE="$(SOURCE)" | tail -n 1)"
	short_id="$$($(MAKE) --no-print-directory test-token-deploy CODE="$(SHORT_CODE)" TOKEN_NAME="Test $(SHORT_CODE)" TOKEN_DECIMALS="$(TOKEN_DECIMALS)" FAUCET="$$faucet_id" NETWORK="$(NETWORK)" SOURCE="$(SOURCE)" | tail -n 1)"
	$(MAKE) --no-print-directory faucet-add-token FAUCET="$$faucet_id" TOKEN="$$long_id" CLAIM_AMOUNT="$(CLAIM_AMOUNT)" NETWORK="$(NETWORK)" SOURCE="$(SOURCE)"
	$(MAKE) --no-print-directory faucet-add-token FAUCET="$$faucet_id" TOKEN="$$short_id" CLAIM_AMOUNT="$(CLAIM_AMOUNT)" NETWORK="$(NETWORK)" SOURCE="$(SOURCE)"
	@printf 'Native test tokens ready:\n'
	@printf '  FAUCET=%s\n' "$$faucet_id"
	@printf '  $(LONG_CODE)=%s\n' "$$long_id"
	@printf '  $(SHORT_CODE)=%s\n' "$$short_id"

# Deploy all four testnet tokens (TUSDC, TWBTC, TETH, TXLM) under a single faucet.
# This is the standard full-testnet setup that backs all three GM markets.
#
# Usage:
#   make test-tokens-all-with-faucet NETWORK=testnet SOURCE=alice
test-tokens-all-with-faucet: preflight guard-not-mainnet
	@mkdir -p "$(DEPLOY_DIR)"
	faucet_id="$$($(MAKE) --no-print-directory faucet-deploy NETWORK="$(NETWORK)" SOURCE="$(SOURCE)" FAUCET_COOLDOWN="$(FAUCET_COOLDOWN)" | tail -n 1)"
	tusdc_id="$$($(MAKE) --no-print-directory test-token-deploy CODE="TUSDC" TOKEN_NAME="Test USDC" TOKEN_DECIMALS="$(TOKEN_DECIMALS)" FAUCET="$$faucet_id" NETWORK="$(NETWORK)" SOURCE="$(SOURCE)" | tail -n 1)"
	twbtc_id="$$($(MAKE) --no-print-directory test-token-deploy CODE="TWBTC" TOKEN_NAME="Test Wrapped Bitcoin" TOKEN_DECIMALS="$(TOKEN_DECIMALS)" FAUCET="$$faucet_id" NETWORK="$(NETWORK)" SOURCE="$(SOURCE)" | tail -n 1)"
	teth_id="$$($(MAKE)  --no-print-directory test-token-deploy CODE="TETH"  TOKEN_NAME="Test Ether"           TOKEN_DECIMALS="$(TOKEN_DECIMALS)" FAUCET="$$faucet_id" NETWORK="$(NETWORK)" SOURCE="$(SOURCE)" | tail -n 1)"
	txlm_id="$$($(MAKE)  --no-print-directory test-token-deploy CODE="TXLM"  TOKEN_NAME="Test Stellar Lumen"  TOKEN_DECIMALS="$(TOKEN_DECIMALS)" FAUCET="$$faucet_id" NETWORK="$(NETWORK)" SOURCE="$(SOURCE)" | tail -n 1)"
	$(MAKE) --no-print-directory faucet-add-token FAUCET="$$faucet_id" TOKEN="$$tusdc_id" CLAIM_AMOUNT="$(CLAIM_AMOUNT)" NETWORK="$(NETWORK)" SOURCE="$(SOURCE)"
	$(MAKE) --no-print-directory faucet-add-token FAUCET="$$faucet_id" TOKEN="$$twbtc_id" CLAIM_AMOUNT="$(CLAIM_AMOUNT)" NETWORK="$(NETWORK)" SOURCE="$(SOURCE)"
	$(MAKE) --no-print-directory faucet-add-token FAUCET="$$faucet_id" TOKEN="$$teth_id"  CLAIM_AMOUNT="$(CLAIM_AMOUNT)" NETWORK="$(NETWORK)" SOURCE="$(SOURCE)"
	$(MAKE) --no-print-directory faucet-add-token FAUCET="$$faucet_id" TOKEN="$$txlm_id"  CLAIM_AMOUNT="$(CLAIM_AMOUNT)" NETWORK="$(NETWORK)" SOURCE="$(SOURCE)"
	@printf 'All four testnet tokens ready:\n'
	@printf '  FAUCET=%s\n' "$$faucet_id"
	@printf '  TUSDC=%s\n'  "$$tusdc_id"
	@printf '  TWBTC=%s\n'  "$$twbtc_id"
	@printf '  TETH=%s\n'   "$$teth_id"
	@printf '  TXLM=%s\n'   "$$txlm_id"
