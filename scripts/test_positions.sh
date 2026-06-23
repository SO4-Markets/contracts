#!/usr/bin/env bash
# scripts/test_positions.sh — Open a long and a short position on the TWBTC/TUSDC market.
#
# Flow per position:
#   1. Fetch signed prices from oracle worker
#   2. Submit prices to oracle contract
#   3. Approve exchange_router to pull collateral (send_tokens needs approval)
#   4. Call send_tokens to transfer collateral → order_vault
#   5. Call create_order  → order key
#   6. Re-submit oracle prices (temp storage may expire)
#   7. Call execute_order
#
# Usage:
#   bash scripts/test_positions.sh [NETWORK] [SOURCE]

set -euo pipefail

NETWORK="${1:-testnet}"
SOURCE="${2:-steins-testnet}"
ORACLE_URL="${ORACLE_URL:-https://oracle.biscotti-proxy-worker.workers.dev}"

source ".deployed/$NETWORK.env"

RED='\033[0;31m'; GREEN='\033[0;32m'; YELLOW='\033[1;33m'; CYAN='\033[0;36m'; BOLD='\033[1m'; NC='\033[0m'
log()  { echo -e "${CYAN}▸${NC} $*"; }
ok()   { echo -e "  ${GREEN}✔${NC} $*"; }
warn() { echo -e "  ${YELLOW}⚠${NC} $*" >&2; }
die()  { echo -e "${RED}✖ $*${NC}" >&2; exit 1; }

KEEPER_ADDR=$(stellar keys address "$SOURCE")
log "Keeper: $KEEPER_ADDR"
log "Network: $NETWORK"

# Contract addresses
MARKET="$MARKET_TOKEN_TWBTC_TUSDC"
TWBTC="$MARKET_TOKEN_TWBTC_TUSDC_LONG"   # long token
TUSDC="$MARKET_TOKEN_TWBTC_TUSDC_SHORT"  # short token

# ── Step 1: Fetch + submit oracle prices ────────────────────────────────────────
fetch_and_submit_prices() {
    echo -e "\n${BOLD}Fetch + submit oracle prices${NC}"
    local PRICES_JSON
    PRICES_JSON=$(curl -sf "$ORACLE_URL/prices") || die "Failed to fetch prices"
    log "Got $(echo "$PRICES_JSON" | python3 -c "import sys,json; print(len(json.load(sys.stdin)))") prices"

    local SIGNED_PRICES_ARG
    SIGNED_PRICES_ARG=$(echo "$PRICES_JSON" | python3 -c "
import sys, json
data = json.load(sys.stdin)
entries = []
for p in data:
    entries.append({
        'token': p['token'],
        'min_price': str(p['min']),
        'max_price': str(p['max']),
        'timestamp': p['timestamp'],
        'signature': p['signature'],
        'keeper_index': 0,
        'ledger_seq': p['ledger_seq']
    })
print(json.dumps(entries))
")

    stellar contract invoke \
        --id "$ORACLE" \
        --source "$SOURCE" \
        --network "$NETWORK" \
        -- set_prices \
        --caller "$KEEPER_ADDR" \
        --prices "$SIGNED_PRICES_ARG" 2>&1 | grep -E "✅|❌" | head -1 || true

    # Return the prices JSON for computing size_delta_usd
    echo "$PRICES_JSON"
}

# ── Open a position ──────────────────────────────────────────────────────────────
# $1 = label (e.g. "LONG")
# $2 = is_long ("true" or "false")
# $3 = collateral token address
# $4 = collateral amount (raw, 7 decimals)
# $5 = size_delta_usd (FLOAT_PRECISION = 10^30)
# $6 = acceptable_price (FLOAT_PRECISION; 0 = no check)
open_position() {
    local LABEL="$1"
    local IS_LONG="$2"
    local COLLATERAL_TOKEN="$3"
    local COLLATERAL_AMOUNT="$4"
    local SIZE_USD="$5"
    local ACCEPTABLE_PRICE="$6"

    echo -e "\n${BOLD}═══ Open $LABEL position ═══${NC}"
    log "Collateral : $COLLATERAL_AMOUNT raw units of $COLLATERAL_TOKEN"
    log "Size (USD) : $SIZE_USD (FLOAT_PRECISION)"

    # ── Dynamic expiration ledger ────────────────────────────────────────────────
    local CURRENT_LEDGER
    CURRENT_LEDGER=$(curl -sf "https://soroban-testnet.stellar.org" -X POST \
        -H "Content-Type: application/json" \
        -d '{"jsonrpc":"2.0","id":1,"method":"getLatestLedger"}' \
        | python3 -c "import sys,json; print(json.load(sys.stdin)['result']['sequence'])")
    local EXP_LEDGER=$(( CURRENT_LEDGER + 50000 ))

    # ── Approve exchange_router to pull collateral ───────────────────────────────
    log "Approving exchange_router..."
    stellar contract invoke \
        --id "$COLLATERAL_TOKEN" \
        --source "$SOURCE" \
        --network "$NETWORK" \
        -- approve \
        --from "$KEEPER_ADDR" \
        --spender "$EXCHANGE_ROUTER" \
        --amount "$COLLATERAL_AMOUNT" \
        --expiration_ledger "$EXP_LEDGER" 2>&1 | grep -E "✅|❌" | head -1 || true
    ok "Approved $COLLATERAL_AMOUNT collateral"

    # ── Re-submit prices before send_tokens ─────────────────────────────────────
    local PRICES_JSON
    PRICES_JSON=$(fetch_and_submit_prices)
    ok "Prices in oracle"

    # ── send_tokens: collateral → order_vault ───────────────────────────────────
    log "Sending collateral to order_vault..."
    stellar contract invoke \
        --id "$EXCHANGE_ROUTER" \
        --source "$SOURCE" \
        --network "$NETWORK" \
        -- send_tokens \
        --caller "$KEEPER_ADDR" \
        --token "$COLLATERAL_TOKEN" \
        --receiver "$ORDER_VAULT" \
        --amount "$COLLATERAL_AMOUNT" 2>&1 | grep -E "✅|❌" | head -1 || true
    ok "Collateral in order_vault"

    # ── create_order ─────────────────────────────────────────────────────────────
    log "Creating order..."
    local PY_IS_LONG
    PY_IS_LONG=$([ "$IS_LONG" = "true" ] && echo "True" || echo "False")
    local ORDER_PARAMS
    ORDER_PARAMS=$(python3 -c "
import json
p = {
    'receiver':                '$KEEPER_ADDR',
    'market':                  '$MARKET',
    'initial_collateral_token':'$COLLATERAL_TOKEN',
    'swap_path':               [],
    'size_delta_usd':          str($SIZE_USD),
    'collateral_delta_amount': '0',
    'trigger_price':           '0',
    'acceptable_price':        str($ACCEPTABLE_PRICE),
    'execution_fee':           '0',
    'min_output_amount':       '0',
    'order_type':              {'MarketIncrease': None},
    'is_long':                 $PY_IS_LONG
}
print(json.dumps(p))
")

    local ORDER_KEY
    ORDER_KEY=$(stellar contract invoke \
        --id "$ORDER_HANDLER" \
        --source "$SOURCE" \
        --network "$NETWORK" \
        -- create_order \
        --caller "$KEEPER_ADDR" \
        --params "$ORDER_PARAMS" 2>/dev/null | tr -d '"')

    log "Order key: $ORDER_KEY"
    ok "Order created"

    # ── Re-submit prices before execute ─────────────────────────────────────────
    fetch_and_submit_prices > /dev/null
    ok "Prices refreshed"

    # ── execute_order ────────────────────────────────────────────────────────────
    echo -e "\n${BOLD}Execute $LABEL order${NC}"
    stellar contract invoke \
        --id "$ORDER_HANDLER" \
        --source "$SOURCE" \
        --network "$NETWORK" \
        -- execute_order \
        --keeper "$KEEPER_ADDR" \
        --key "$ORDER_KEY" 2>&1

    ok "$LABEL position opened — key: $ORDER_KEY"
}

# ── Get oracle price for size computation ────────────────────────────────────────
echo -e "\n${BOLD}Fetching current BTC price for sizing...${NC}"
PRICES_JSON=$(curl -sf "$ORACLE_URL/prices") || die "Cannot reach oracle"

BTC_PRICE=$(echo "$PRICES_JSON" | python3 -c "
import sys, json
data = json.load(sys.stdin)
btc = next(p for p in data if p['symbol'] == 'TWBTC')
mid = (int(btc['min']) + int(btc['max'])) // 2
print(mid)
")
log "BTC mid-price (FLOAT_PRECISION): $BTC_PRICE"

# Collateral: 1 TWBTC = 10_000_000 raw units (7 decimals)
# size_delta_usd = collateral_amount × collateral_price × 2× leverage
#                = 10_000_000 × BTC_PRICE / TOKEN_PRECISION × 2
# TOKEN_PRECISION = 10^7, FLOAT_PRECISION = 10^30
# size_delta_usd = 10_000_000 * BTC_PRICE / 10^7 * 2 = BTC_PRICE * 2
COLLATERAL_TWBTC=10000000   # 1 TWBTC
COLLATERAL_TUSDC=10000000   # 1 TUSDC

# 2x long BTC: size = 2 × 1 TWBTC worth in USD
LONG_SIZE_USD=$(python3 -c "
btc = $BTC_PRICE
# size_delta_usd = collateral_tokens × price (FLOAT_PRECISION)
# = 1 token × price = 1 × 10^7 units × price / TOKEN_PRECISION
# = price × 1 (since TOKEN_PRECISION = 10^7)
# 2x leverage: multiply by 2
size = btc * 2
print(int(size))
")

# 2x short BTC: collateral is TUSDC ($1 fixed price = 10^30 FLOAT_PRECISION)
# 1 TUSDC at $1: size = 1 token × $1 price × 2x = 2 × 10^30 / 10^7 × 10^7
TUSDC_PRICE_FLOAT="1000000000000000000000000000000"  # $1 in FLOAT_PRECISION
SHORT_SIZE_USD=$(python3 -c "
# 1 TUSDC = 10^30 FLOAT_PRECISION size when 1x, ×2 for 2x leverage
tusdc = int('$TUSDC_PRICE_FLOAT')
size = tusdc * 2  # 2 TUSDC worth of position
print(size)
")

log "Long size_delta_usd  : $LONG_SIZE_USD"
log "Short size_delta_usd : $SHORT_SIZE_USD"

# acceptable_price: long = oracle × 1.01 (max we pay), short = oracle × 0.99 (min we receive)
LONG_ACCEPTABLE=$(python3 -c "print(int($BTC_PRICE * 1.01))")
SHORT_ACCEPTABLE=$(python3 -c "print(int($BTC_PRICE * 0.99))")

# ── Run tests ────────────────────────────────────────────────────────────────────
open_position "LONG"  "true"  "$TWBTC" "$COLLATERAL_TWBTC" "$LONG_SIZE_USD"  "$LONG_ACCEPTABLE"
open_position "SHORT" "false" "$TUSDC" "$COLLATERAL_TUSDC" "$SHORT_SIZE_USD" "$SHORT_ACCEPTABLE"

echo -e "\n${GREEN}${BOLD}Position test complete!${NC}"
echo "  TWBTC/TUSDC long  — 1 TWBTC collateral, 2× size"
echo "  TWBTC/TUSDC short — 1 TUSDC collateral, 2× size"
