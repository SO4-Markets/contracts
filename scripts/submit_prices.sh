#!/usr/bin/env bash
# scripts/submit_prices.sh — Submit a single token price to the SO4.market oracle.
#
# Usage:
#   TOKEN=<contract_id> MIN_PRICE=<i128> MAX_PRICE=<i128> \
#     bash scripts/submit_prices.sh [NETWORK] [SOURCE_KEY]
#
# Required env vars (no defaults — script fails loudly if unset):
#   ORACLE      Address of the deployed oracle contract.
#   TOKEN       Address of the token whose price you are submitting.
#   MIN_PRICE   Minimum price (integer, scaled to the protocol's FLOAT_PRECISION).
#   MAX_PRICE   Maximum price (integer, scaled to the protocol's FLOAT_PRECISION).
#
# Optional:
#   NETWORK     testnet (default) | mainnet | local
#   SOURCE_KEY  Stellar key name of an authorized order-keeper (default: alice)
#
# The caller (SOURCE_KEY) must hold the ORDER_KEEPER role in role_store before
# this script will succeed.  Register a keeper with:
#   make grant-keeper SOURCE=admin KEEPER=alice NETWORK=testnet
#
# Example:
#   ORACLE=C... TOKEN=C... MIN_PRICE=500000000000000000000000000000000 \
#   MAX_PRICE=500500000000000000000000000000000 \
#     bash scripts/submit_prices.sh testnet alice

set -euo pipefail

# ── Args ──────────────────────────────────────────────────────────────────────
NETWORK="${1:-testnet}"
SOURCE="${2:-alice}"

# ── Colours ───────────────────────────────────────────────────────────────────
RED='\033[0;31m'; GREEN='\033[0;32m'; CYAN='\033[0;36m'; NC='\033[0m'

die() { printf "${RED}✖ %s${NC}\n" "$*" >&2; exit 1; }
ok()  { printf "  ${GREEN}✔${NC} %s\n" "$*"; }

# ── Validate required env vars ────────────────────────────────────────────────
# All four must be set externally. No placeholders, no fallbacks.

[[ -v ORACLE    && -n "${ORACLE}"    ]] || \
  die "ORACLE is not set. Export the deployed oracle contract address:\n  export ORACLE=C..."

[[ -v TOKEN     && -n "${TOKEN}"     ]] || \
  die "TOKEN is not set. Export the token contract address you want to price:\n  export TOKEN=C..."

[[ -v MIN_PRICE && -n "${MIN_PRICE}" ]] || \
  die "MIN_PRICE is not set. Set the minimum price (FLOAT_PRECISION-scaled integer)."

[[ -v MAX_PRICE && -n "${MAX_PRICE}" ]] || \
  die "MAX_PRICE is not set. Set the maximum price (FLOAT_PRECISION-scaled integer)."

# Sanity-check that min ≤ max so we catch configuration errors before hitting
# the contract's InvalidPrice guard.
if (( MIN_PRICE > MAX_PRICE )); then
  die "MIN_PRICE ($MIN_PRICE) is greater than MAX_PRICE ($MAX_PRICE)."
fi

# ── Preflight ─────────────────────────────────────────────────────────────────
command -v stellar >/dev/null 2>&1 || \
  die "stellar CLI not found. Install: cargo install stellar-cli --features opt"

stellar keys address "$SOURCE" >/dev/null 2>&1 || \
  die "Key '$SOURCE' not found. Run: stellar keys generate --global $SOURCE --network $NETWORK"

# ── Submit price ──────────────────────────────────────────────────────────────
printf "${CYAN}▸${NC} Submitting price for %s on %s via %s\n" "$TOKEN" "$NETWORK" "$SOURCE" >&2
printf "  min=%s  max=%s\n" "$MIN_PRICE" "$MAX_PRICE" >&2

# set_prices_simple(caller, prices: Vec<TokenPrice>)
# TokenPrice is a struct with fields: token (Address), min (i128), max (i128).
# The Stellar CLI encodes a Vec of structs as repeated --prices args with
# a JSON-like struct value.
stellar contract invoke \
  --id    "$ORACLE" \
  --source "$SOURCE" \
  --network "$NETWORK" \
  -- set_prices_simple \
  --caller "$(stellar keys address "$SOURCE")" \
  --prices "[{\"token\":\"$TOKEN\",\"min\":$MIN_PRICE,\"max\":$MAX_PRICE}]"

ok "price submitted  token=$TOKEN  network=$NETWORK"
