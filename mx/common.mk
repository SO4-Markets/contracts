# Shared Make settings for SO4.market contract operations.

SHELL := bash
.SHELLFLAGS := --noprofile --norc -eu -o pipefail -c
.ONESHELL:

NETWORK ?= testnet
SOURCE ?= alice
CONTRACT ?= deposit_handler
DEPLOY_DIR ?= .deployed
DEPLOY_ENV ?= $(DEPLOY_DIR)/$(NETWORK).env
TOKEN_ENV ?= $(DEPLOY_DIR)/tokens-$(NETWORK).env

# Stellar CLI v23+ writes SDK 23+ builds to wasm32v1-none. Older local builds in
# this repo used wasm32-unknown-unknown, so keep the target configurable.
WASM_TARGET ?= wasm32v1-none
WASM_PROFILE ?= release
WASM_DIR ?= target/$(WASM_TARGET)/$(WASM_PROFILE)

CONTRACTS := \
	role_store \
	data_store \
	oracle \
	test_token \
	test_faucet \
	market_token \
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

.PHONY: preflight print-config help-mx

preflight:
	@command -v stellar >/dev/null || { printf '%s\n' 'stellar CLI not found. Install stellar-cli first.'; exit 1; }
	@command -v cargo >/dev/null || { printf '%s\n' 'cargo not found. Install Rust first.'; exit 1; }

print-config:
	@printf 'NETWORK=%s\n' '$(NETWORK)'
	@printf 'SOURCE=%s\n' '$(SOURCE)'
	@printf 'CONTRACT=%s\n' '$(CONTRACT)'
	@printf 'DEPLOY_ENV=%s\n' '$(DEPLOY_ENV)'
	@printf 'TOKEN_ENV=%s\n' '$(TOKEN_ENV)'
	@printf 'WASM_DIR=%s\n' '$(WASM_DIR)'

help-mx:
	@sed -n '1,220p' mx/README.md
