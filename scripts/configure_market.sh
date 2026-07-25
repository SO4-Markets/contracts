#!/usr/bin/env bash
# scripts/configure_market.sh — Write all GMX-style config keys for one market.
#
# Requires CONTROLLER role on the caller (SOURCE key).
# Called by bootstrap.sh after create_market, or standalone for re-configuration.
#
# Usage:
#   bash scripts/configure_market.sh [NETWORK] [SOURCE_KEY]
#
#   NETWORK    : testnet (default) | local
#   SOURCE_KEY : stellar key name  (default: alice)
#
# Required env vars (all read from .deployed/<NETWORK>.env and TOKEN_ENV, or set manually):
#   DATA_STORE      contract ID of the data_store
#   MARKET_TOKEN    contract ID of the market token (LP token)
#   LONG_TOKEN      contract ID of the long token
#   SHORT_TOKEN     contract ID of the short token
#
# Optional overrides (defaults are safe testnet starter values):
#   MAX_POOL            max pool amount per token in base units  (default: 10^13 = 1M tokens)
#   MAX_OI              max open interest in USD * FLOAT_PREC    (default: 5*10^35 = 500k USD)
#   MIN_COLL_FACTOR     min collateral factor                    (default: 10^28  = 1%)
#   POS_FEE_FACTOR      position fee factor                      (default: 10^27  = 0.1%)
#   SWAP_FEE_FACTOR     swap fee factor                          (default: 5*10^26 = 0.05%)
#   BORROWING_FACTOR    borrowing factor per side                (default: 10^24)
#   FUNDING_FACTOR      funding factor                           (default: 10^26)
#   MAX_PNL_FACTOR      max pnl factor (deposits/withdrawals/traders) (default: 5*10^29 = 50%)
#   MIN_FIRST_DEPOSIT   min market tokens for first deposit      (default: 1000)

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
NETWORK="${1:-testnet}"
SOURCE="${2:-alice}"

DEPLOYED_DIR=".deployed"
DEPLOYED_ENV="$DEPLOYED_DIR/$NETWORK.env"
TOKEN_ENV="$DEPLOYED_DIR/tokens-$NETWORK.env"

RED='\033[0;31m'; GREEN='\033[0;32m'; YELLOW='\033[1;33m'
CYAN='\033[0;36m'; NC='\033[0m'

log()  { echo -e "${CYAN}  ▸${NC} $*" >&2; }
ok()   { echo -e "    ${GREEN}✔${NC} $*"; }
warn() { echo -e "    ${YELLOW}⚠${NC} $*" >&2; }
die()  { echo -e "${RED}✖ $*${NC}" >&2; exit 1; }

# ── Preflight ──────────────────────────────────────────────────────────────────
command -v stellar  >/dev/null 2>&1 || die "stellar CLI not found"
command -v python3  >/dev/null 2>&1 || die "python3 not found (needed for key computation)"

[[ -f "$DEPLOYED_ENV" ]] && source "$DEPLOYED_ENV"
[[ -f "$TOKEN_ENV"    ]] && source "$TOKEN_ENV"

ADMIN=$(stellar keys address "$SOURCE" 2>/dev/null) || \
  die "Key '$SOURCE' not found."

DATA_STORE="${DATA_STORE:?DATA_STORE not set. Source $DEPLOYED_ENV first.}"
MARKET_TOKEN="${MARKET_TOKEN:?MARKET_TOKEN not set.}"
LONG_TOKEN="${LONG_TOKEN:?LONG_TOKEN not set.}"
SHORT_TOKEN="${SHORT_TOKEN:?SHORT_TOKEN not set.}"

MT="$MARKET_TOKEN"

# ── Key helper ────────────────────────────────────────────────────────────────
key() { python3 "$SCRIPT_DIR/compute_key.py" "$@"; }

# ── Config defaults (all in FLOAT_PRECISION = 10^30 unless noted) ─────────────
#
# FLOAT_PRECISION = 1_000_000_000_000_000_000_000_000_000_000  (10^30)

# Pool / OI caps
MAX_POOL="${MAX_POOL:-10000000000000}"             # 1,000,000 tokens (7 dec)
MAX_OI="${MAX_OI:-500000000000000000000000000000000000}"  # 500k USD * 10^30

# Collateral / leverage
MIN_COLL_FACTOR="${MIN_COLL_FACTOR:-10000000000000000000000000000}"  # 1% (10^28)
MAX_LEVERAGE="${MAX_LEVERAGE:-50000000000000000000000000000000}"     # 50x (50*10^30)

# Fees
POS_FEE_FACTOR="${POS_FEE_FACTOR:-1000000000000000000000000000}"    # 0.1%  (10^27)
SWAP_FEE_FACTOR="${SWAP_FEE_FACTOR:-500000000000000000000000000}"   # 0.05% (5*10^26)

# Borrowing
BORROWING_FACTOR="${BORROWING_FACTOR:-1000000000000000000000000}"        # 10^24
BORROWING_EXPONENT="${BORROWING_EXPONENT:-1000000000000000000000000000000}"  # 1.0 (10^30)

# Funding
FUNDING_FACTOR="${FUNDING_FACTOR:-100000000000000000000000000}"              # 10^26
FUNDING_EXPONENT="${FUNDING_EXPONENT:-1000000000000000000000000000000}"      # 1.0 (10^30)
FUNDING_INCREASE="${FUNDING_INCREASE:-100000000000000000000000000}"          # 10^26
FUNDING_DECREASE="${FUNDING_DECREASE:-500000000000000000000000000}"          # 5*10^26
MIN_FUNDING="${MIN_FUNDING:-10000000000000000000000}"                        # 10^22
MAX_FUNDING="${MAX_FUNDING:-100000000000000000000000000}"                    # 10^26

# Swap impact
SWAP_IMPACT_POS="${SWAP_IMPACT_POS:-200000000000000000000000}"       # 2*10^23
SWAP_IMPACT_NEG="${SWAP_IMPACT_NEG:-400000000000000000000000}"       # 4*10^23
SWAP_IMPACT_EXP="${SWAP_IMPACT_EXP:-1000000000000000000000000000000}"  # 1.0 (linear)

# Position impact
POS_IMPACT_POS="${POS_IMPACT_POS:-100000000000000000000000}"          # 10^23
POS_IMPACT_NEG="${POS_IMPACT_NEG:-200000000000000000000000}"          # 2*10^23
POS_IMPACT_EXP="${POS_IMPACT_EXP:-2000000000000000000000000000000}"   # 2.0 (quadratic)

# PnL
MAX_PNL_FACTOR="${MAX_PNL_FACTOR:-500000000000000000000000000000}"  # 50% (5*10^29)
MAX_PNL_ADL="${MAX_PNL_ADL:-450000000000000000000000000000}"        # 45% (4.5*10^29)

# First deposit floor
MIN_FIRST_DEPOSIT="${MIN_FIRST_DEPOSIT:-1000}"

# ── Precompute PnL type hashes ────────────────────────────────────────────────
PNL_TYPE_TRADERS=$(key max_pnl_factor_for_traders)
PNL_TYPE_DEPOSITS=$(key max_pnl_factor_for_deposits)
PNL_TYPE_WITHDRAWALS=$(key max_pnl_factor_for_withdrawals)

# ── Write helper ──────────────────────────────────────────────────────────────
set_u128() {
  local label="$1" key_hex="$2" value="$3"
  log "$label"
  stellar contract invoke \
    --id   "$DATA_STORE" \
    --source "$SOURCE" \
    --network "$NETWORK" \
    -- set_u128 \
    --caller "$ADMIN" \
    --key    "$key_hex" \
    --value  "$value" >/dev/null
  ok "$label = $value"
}

set_bool_key() {
  local label="$1" key_hex="$2" value="$3"
  log "$label"
  stellar contract invoke \
    --id   "$DATA_STORE" \
    --source "$SOURCE" \
    --network "$NETWORK" \
    -- set_bool \
    --caller "$ADMIN" \
    --key    "$key_hex" \
    --value  "$value" >/dev/null
  ok "$label = $value"
}

echo -e "${CYAN}Configuring market ${NC}$MT"
echo

# ── Pool size caps ────────────────────────────────────────────────────────────
log "Pool size caps"
set_u128 "max_pool_amount (long)"  "$(key max_pool_amount "$MT" "$LONG_TOKEN")"  "$MAX_POOL"
set_u128 "max_pool_amount (short)" "$(key max_pool_amount "$MT" "$SHORT_TOKEN")" "$MAX_POOL"

# ── Open interest caps ────────────────────────────────────────────────────────
log "Open interest caps"
set_u128 "max_open_interest (long)"  "$(key max_open_interest "$MT" true)"  "$MAX_OI"
set_u128 "max_open_interest (short)" "$(key max_open_interest "$MT" false)" "$MAX_OI"

# ── Collateral & leverage ─────────────────────────────────────────────────────
log "Collateral / leverage"
set_u128 "min_collateral_factor" "$(key min_collateral_factor "$MT")" "$MIN_COLL_FACTOR"
set_u128 "max_leverage"          "$(key max_leverage "$MT")"          "$MAX_LEVERAGE"

# ── Fees ──────────────────────────────────────────────────────────────────────
log "Fee factors"
set_u128 "position_fee_factor (positive impact)" "$(key position_fee_factor "$MT" true)"  "$POS_FEE_FACTOR"
set_u128 "position_fee_factor (negative impact)" "$(key position_fee_factor "$MT" false)" "$POS_FEE_FACTOR"
set_u128 "swap_fee_factor (positive impact)"     "$(key swap_fee_factor "$MT" true)"      "$SWAP_FEE_FACTOR"
set_u128 "swap_fee_factor (negative impact)"     "$(key swap_fee_factor "$MT" false)"     "$SWAP_FEE_FACTOR"

# ── Borrowing ─────────────────────────────────────────────────────────────────
log "Borrowing factors"
set_u128 "borrowing_factor (long)"           "$(key borrowing_factor "$MT" true)"           "$BORROWING_FACTOR"
set_u128 "borrowing_factor (short)"          "$(key borrowing_factor "$MT" false)"          "$BORROWING_FACTOR"
set_u128 "borrowing_exponent_factor (long)"  "$(key borrowing_exponent_factor "$MT" true)"  "$BORROWING_EXPONENT"
set_u128 "borrowing_exponent_factor (short)" "$(key borrowing_exponent_factor "$MT" false)" "$BORROWING_EXPONENT"

# ── Funding ───────────────────────────────────────────────────────────────────
log "Funding factors"
set_u128 "funding_factor"                         "$(key funding_factor "$MT")"                          "$FUNDING_FACTOR"
set_u128 "funding_exponent_factor"                "$(key funding_exponent_factor "$MT")"                 "$FUNDING_EXPONENT"
set_u128 "funding_increase_factor_per_second"     "$(key funding_increase_factor_per_second "$MT")"      "$FUNDING_INCREASE"
set_u128 "funding_decrease_factor_per_second"     "$(key funding_decrease_factor_per_second "$MT")"      "$FUNDING_DECREASE"
set_u128 "min_funding_factor_per_second"          "$(key min_funding_factor_per_second "$MT")"           "$MIN_FUNDING"
set_u128 "max_funding_factor_per_second"          "$(key max_funding_factor_per_second "$MT")"           "$MAX_FUNDING"

# ── Swap price impact ─────────────────────────────────────────────────────────
log "Swap impact factors"
set_u128 "swap_impact_factor (positive)"  "$(key swap_impact_factor "$MT" true)"  "$SWAP_IMPACT_POS"
set_u128 "swap_impact_factor (negative)"  "$(key swap_impact_factor "$MT" false)" "$SWAP_IMPACT_NEG"
set_u128 "swap_impact_exponent_factor"    "$(key swap_impact_exponent_factor "$MT")" "$SWAP_IMPACT_EXP"

# ── Position price impact ─────────────────────────────────────────────────────
log "Position impact factors"
set_u128 "position_impact_factor (positive)"  "$(key position_impact_factor "$MT" true)"  "$POS_IMPACT_POS"
set_u128 "position_impact_factor (negative)"  "$(key position_impact_factor "$MT" false)" "$POS_IMPACT_NEG"
set_u128 "position_impact_exponent_factor"    "$(key position_impact_exponent_factor "$MT")" "$POS_IMPACT_EXP"

# ── PnL caps ──────────────────────────────────────────────────────────────────
log "PnL factor caps"
set_u128 "max_pnl_factor (traders, long)"      "$(key max_pnl_factor "$PNL_TYPE_TRADERS"     "$MT" true)"  "$MAX_PNL_FACTOR"
set_u128 "max_pnl_factor (traders, short)"     "$(key max_pnl_factor "$PNL_TYPE_TRADERS"     "$MT" false)" "$MAX_PNL_FACTOR"
set_u128 "max_pnl_factor (deposits, long)"     "$(key max_pnl_factor "$PNL_TYPE_DEPOSITS"    "$MT" true)"  "$MAX_PNL_FACTOR"
set_u128 "max_pnl_factor (deposits, short)"    "$(key max_pnl_factor "$PNL_TYPE_DEPOSITS"    "$MT" false)" "$MAX_PNL_FACTOR"
set_u128 "max_pnl_factor (withdrawals, long)"  "$(key max_pnl_factor "$PNL_TYPE_WITHDRAWALS" "$MT" true)"  "$MAX_PNL_FACTOR"
set_u128 "max_pnl_factor (withdrawals, short)" "$(key max_pnl_factor "$PNL_TYPE_WITHDRAWALS" "$MT" false)" "$MAX_PNL_FACTOR"
set_u128 "max_pnl_factor_for_adl (long)"       "$(key max_pnl_factor_for_adl "$MT" true)"  "$MAX_PNL_ADL"
set_u128 "max_pnl_factor_for_adl (short)"      "$(key max_pnl_factor_for_adl "$MT" false)" "$MAX_PNL_ADL"

# ── First-deposit floor ───────────────────────────────────────────────────────
log "First deposit floor"
set_u128 "min_market_tokens_for_first_deposit" \
  "$(key min_market_tokens_for_first_deposit "$MT")" "$MIN_FIRST_DEPOSIT"

echo
echo -e "${GREEN}Market config complete${NC} — $MT"
echo "  Review and tune values before production use."
