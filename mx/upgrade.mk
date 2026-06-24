# Upgrade workflows.
#
# Contract upgrades require the deployed contract to expose:
#   pub fn upgrade(env: Env, new_wasm_hash: BytesN<32>)
# where the function authenticates the stored admin and calls:
#   env.deployer().update_current_contract_wasm(new_wasm_hash)

# Contracts that are intentionally immutable and must never be upgraded.
# make upgrade will print "skipped (immutable)" for each of these.
IMMUTABLE_CONTRACTS := market_token

UPGRADE_CONTRACTS ?= \
	role_store \
	data_store \
	oracle \
	market_factory \
	deposit_vault \
	deposit_handler \
	withdrawal_vault \
	withdrawal_handler \
	order_vault \
	order_handler \
	liquidation_handler \
	adl_handler \
	fee_handler \
	referral_storage \
	reader \
	exchange_router

DRY_RUN ?= 0

.PHONY: upload upgrade upgrade-contract upgrade-all upgrade-with-hash

upload: preflight build
	@test -n "$(CONTRACT)" || { printf '%s\n' 'Usage: make upload CONTRACT=deposit_handler'; exit 1; }
	@test -f "$(WASM_DIR)/$(CONTRACT).wasm" || { printf 'Missing wasm: %s/%s.wasm\n' "$(WASM_DIR)" "$(CONTRACT)"; exit 1; }
	stellar contract upload \
		--wasm "$(WASM_DIR)/$(CONTRACT).wasm" \
		--source "$(SOURCE)" \
		--network "$(NETWORK)"

upgrade: upgrade-contract

upgrade-contract: preflight build
	@test -n "$(CONTRACT)" || { printf '%s\n' 'Usage: make upgrade-contract CONTRACT=deposit_handler'; exit 1; }
	@for imm in $(IMMUTABLE_CONTRACTS); do \
		if [ "$$imm" = "$(CONTRACT)" ]; then \
			printf 'skipped (immutable)  %s\n' "$(CONTRACT)"; exit 0; \
		fi; \
	done
	@test -f "$(DEPLOY_ENV)" || { printf 'Missing %s. Run make deploy-all first or pass CONTRACT_ID=...\n' "$(DEPLOY_ENV)"; exit 1; }
	source "$(DEPLOY_ENV)"
	contract_key="$$(printf '%s' "$(CONTRACT)" | tr '[:lower:]-' '[:upper:]_')"
	contract_id="$${CONTRACT_ID:-$${!contract_key:-}}"
	test -n "$$contract_id" || { printf 'failed              %s  (no address in %s)\n' "$(CONTRACT)" "$(DEPLOY_ENV)"; exit 1; }
	if [ "$(DRY_RUN)" = "1" ]; then \
		printf 'dry-run             %s  would upgrade at %s\n' "$(CONTRACT)" "$$contract_id"; \
	else \
		wasm_hash="$$(stellar contract upload --wasm "$(WASM_DIR)/$(CONTRACT).wasm" --source "$(SOURCE)" --network "$(NETWORK)")" && \
		printf 'uploaded            %s -> %s\n' "$(CONTRACT)" "$$wasm_hash" && \
		stellar contract invoke \
			--id "$$contract_id" \
			--source "$(SOURCE)" \
			--network "$(NETWORK)" \
			-- upgrade --new_wasm_hash "$$wasm_hash" && \
		printf 'upgraded            %s at %s\n' "$(CONTRACT)" "$$contract_id" || \
		printf 'failed              %s at %s\n' "$(CONTRACT)" "$$contract_id"; \
	fi

upgrade-all: preflight build
	@test -f "$(DEPLOY_ENV)" || { printf 'Missing %s. Run deploy-all first.\n' "$(DEPLOY_ENV)"; exit 1; }
	@if [ "$(DRY_RUN)" = "1" ]; then printf 'DRY RUN — no transactions will be submitted\n'; fi
	source "$(DEPLOY_ENV)"
	for contract in $(UPGRADE_CONTRACTS) $(IMMUTABLE_CONTRACTS); do \
		is_immutable=0; \
		for imm in $(IMMUTABLE_CONTRACTS); do [ "$$imm" = "$$contract" ] && is_immutable=1; done; \
		if [ "$$is_immutable" = "1" ]; then \
			printf 'skipped (immutable)  %s\n' "$$contract"; continue; \
		fi; \
		contract_key="$$(printf '%s' "$$contract" | tr '[:lower:]-' '[:upper:]_')"; \
		contract_id="$${!contract_key:-}"; \
		if [ -z "$$contract_id" ]; then \
			printf 'failed              %s  (no address in $(DEPLOY_ENV))\n' "$$contract"; continue; \
		fi; \
		if [ "$(DRY_RUN)" = "1" ]; then \
			printf 'dry-run             %s  would upgrade at %s\n' "$$contract" "$$contract_id"; continue; \
		fi; \
		wasm_hash="$$(stellar contract upload --wasm "$(WASM_DIR)/$$contract.wasm" --source "$(SOURCE)" --network "$(NETWORK)" 2>&1)" && \
		stellar contract invoke \
			--id "$$contract_id" --source "$(SOURCE)" --network "$(NETWORK)" \
			-- upgrade --new_wasm_hash "$$wasm_hash" \
		&& printf 'upgraded            %s at %s\n' "$$contract" "$$contract_id" \
		|| printf 'failed              %s at %s\n' "$$contract" "$$contract_id"; \
	done
	printf 'Done — %s\n' "$(NETWORK)"

upgrade-with-hash: preflight
	@test -n "$(CONTRACT_ID)" || { printf '%s\n' 'Usage: make upgrade-with-hash CONTRACT_ID=C... WASM_HASH=...'; exit 1; }
	@test -n "$(WASM_HASH)" || { printf '%s\n' 'Usage: make upgrade-with-hash CONTRACT_ID=C... WASM_HASH=...'; exit 1; }
	stellar contract invoke \
		--id "$(CONTRACT_ID)" \
		--source "$(SOURCE)" \
		--network "$(NETWORK)" \
		-- upgrade --new_wasm_hash "$(WASM_HASH)"
