#!/usr/bin/env bash
# scripts/seed_liquidity.sh — Seed initial liquidity into one or all GM markets.
#
# Flow per market:
#   1. Fetch signed prices from oracle keeper worker
#   2. Submit prices to oracle contract (temp storage, valid ~5 min)
#   3. Approve depositHandler to spend long + short tokens
#   4. Call depositHandler.create_deposit  → returns 32-byte deposit key
#   5. Call depositHandler.execute_deposit → mints GM tokens to SOURCE
#
# Usage:
#   bash scripts/seed_liquidity.sh [NETWORK] [SOURCE]
#
# Environment variables:
#   NETWORK         : testnet (default)
#   SOURCE          : stellar key name (default: steins-testnet)
#   ORACLE_URL      : oracle worker URL (default: https://oracle.biscotti-proxy-worker.workers.dev)
#   SEED_LONG       : long-token amount in raw units (7 decimals, default: 10_0000000 = 10 tokens)
#   SEED_SHORT      : short-token amount in raw units (7 decimals, default: 10_0000000 = 10 tokens)
#   MARKET_FILTER   : "TWBTC" | "TETH" | "TXLM" | "" (empty = all three markets)

set -euo pipefail
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

NETWORK="${1:-testnet}"
SOURCE="${2:-steins-testnet}"
ORACLE_URL="${ORACLE_URL:-https://oracle.biscotti-proxy-worker.workers.dev}"
SEED_LONG="${SEED_LONG:-100000000}"   # 10 tokens (7 decimals)
SEED_SHORT="${SEED_SHORT:-100000000}" # 10 tokens (7 decimals)
MARKET_FILTER="${MARKET_FILTER:-}"

DEPLOYED_ENV=".deployed/$NETWORK.env"
[[ -f "$DEPLOYED_ENV" ]] || { echo "Error: $DEPLOYED_ENV not found"; exit 1; }
source "$DEPLOYED_ENV"

RED='\033[0;31m'; GREEN='\033[0;32m'; YELLOW='\033[1;33m'; CYAN='\033[0;36m'; BOLD='\033[1m'; NC='\033[0m'
log()  { echo -e "${CYAN}▸${NC} $*"; }
ok()   { echo -e "  ${GREEN}✔${NC} $*"; }
warn() { echo -e "  ${YELLOW}⚠${NC} $*" >&2; }
die()  { echo -e "${RED}✖ $*${NC}" >&2; exit 1; }

invoke() {
    local contract="$1"; shift
    stellar contract invoke \
        --id "$contract" \
        --source "$SOURCE" \
        --network "$NETWORK" \
        -- "$@"
}

invoke_out() {
    local contract="$1"; shift
    stellar contract invoke \
        --id "$contract" \
        --source "$SOURCE" \
        --network "$NETWORK" \
        -- "$@" 2>/dev/null
}

KEEPER_ADDR=$(stellar keys address "$SOURCE")
FAUCET="${FAUCET:-CCWXXBKXHHP5DXC6TYVIL22XUNHD5A75O6WM5D2KM5PY45IOV5VDMARJ}"
log "Keeper: $KEEPER_ADDR"
log "Network: $NETWORK"
log "Oracle worker: $ORACLE_URL"

# ── Step 0: Fund keeper with test tokens via faucet ────────────────────────────
echo -e "\n${BOLD}[0/5] Claim test tokens from faucet${NC}"
TWBTC="$MARKET_TOKEN_TWBTC_TUSDC_LONG"
TETH="$MARKET_TOKEN_TETH_TUSDC_LONG"
TXLM="$MARKET_TOKEN_TXLM_TUSDC_LONG"
TUSDC="$MARKET_TOKEN_TWBTC_TUSDC_SHORT"  # same for all markets

# claim_many has a Soroban auth-reuse bug — claim each token individually
for _TOKEN in "$TWBTC" "$TETH" "$TXLM" "$TUSDC"; do
    stellar contract invoke \
        --id "$FAUCET" \
        --source "$SOURCE" \
        --network "$NETWORK" \
        -- claim \
        --account "$KEEPER_ADDR" \
        --token "$_TOKEN" 2>&1 | grep -E "✅|❌" | head -1 || true
done
ok "Test tokens claimed (100 each) — cooldown errors above are fine if already claimed"

# ── Step 1: Fetch signed prices from oracle worker ─────────────────────────────
echo -e "\n${BOLD}[1/5] Fetch signed prices from oracle keeper${NC}"

PRICES_JSON=$(curl -sf "$ORACLE_URL/prices") || die "Failed to fetch prices from oracle worker"
log "Got $(echo "$PRICES_JSON" | python3 -c "import sys,json; print(len(json.load(sys.stdin)))") signed prices"

# Validate all 4 tokens are present
for sym in TUSDC TWBTC TETH TXLM; do
    echo "$PRICES_JSON" | python3 -c "
import sys, json
data = json.load(sys.stdin)
symbols = [p['symbol'] for p in data]
if '$sym' not in symbols:
    print('ERROR: missing $sym price')
    sys.exit(1)
" || die "$sym price missing from oracle"
done
ok "All 4 token prices available"

# ── Step 2: Submit prices to oracle contract ───────────────────────────────────
echo -e "\n${BOLD}[2/5] Submit prices to oracle contract${NC}"

# Build the Vec<SignedPrice> JSON for stellar CLI.
# SignedPrice fields: token, min_price, max_price, timestamp, signature (BytesN<64>), keeper_index, ledger_seq
SIGNED_PRICES_ARG=$(echo "$PRICES_JSON" | python3 -c "
import sys, json

data = json.load(sys.stdin)
entries = []
for p in data:
    entries.append({
        'token': p['token'],
        'min_price': str(p['min']),
        'max_price': str(p['max']),
        'timestamp': p['timestamp'],        # u64 must be a plain JSON number
        'signature': p['signature'],
        'keeper_index': 0,
        'ledger_seq': p['ledger_seq']       # u32 plain number
    })
print(json.dumps(entries))
")

log "Submitting ${#SIGNED_PRICES_ARG} byte payload to oracle..."
invoke "$ORACLE" set_prices \
    --caller "$KEEPER_ADDR" \
    --prices "$SIGNED_PRICES_ARG" && ok "Prices written to oracle temp storage"

# ── Step 3–5: Seed each market ─────────────────────────────────────────────────

seed_market() {
    local LABEL="$1"
    local LONG_TOKEN="$2"
    local SHORT_TOKEN="$3"
    local MARKET_TOKEN="$4"

    [[ -n "$MARKET_FILTER" && "$LABEL" != *"$MARKET_FILTER"* ]] && return 0

    echo -e "\n${BOLD}[Seed] $LABEL${NC}"
    echo "  Market token : $MARKET_TOKEN"
    echo "  Long token   : $LONG_TOKEN"
    echo "  Short token  : $SHORT_TOKEN"

    # ── Approve deposit_handler to spend tokens ──────────────────────────────
    echo -e "\n${BOLD}[3/5] Approve depositHandler${NC}"
    # expiration_ledger must not exceed current_ledger + max_ttl (~3.1M on testnet)
    local CURRENT_LEDGER
    CURRENT_LEDGER=$(curl -sf "https://soroban-testnet.stellar.org" -X POST \
        -H "Content-Type: application/json" \
        -d '{"jsonrpc":"2.0","id":1,"method":"getLatestLedger"}' \
        | python3 -c "import sys,json; print(json.load(sys.stdin)['result']['sequence'])")
    local EXP_LEDGER=$(( CURRENT_LEDGER + 50000 ))   # ~2.9 days

    invoke "$LONG_TOKEN" approve \
        --from "$KEEPER_ADDR" \
        --spender "$DEPOSIT_HANDLER" \
        --amount "$SEED_LONG" \
        --expiration_ledger "$EXP_LEDGER" && ok "Approved $SEED_LONG long tokens"

    if [[ "$SHORT_TOKEN" != "$LONG_TOKEN" ]]; then
        invoke "$SHORT_TOKEN" approve \
            --from "$KEEPER_ADDR" \
            --spender "$DEPOSIT_HANDLER" \
            --amount "$SEED_SHORT" \
            --expiration_ledger "$EXP_LEDGER" && ok "Approved $SEED_SHORT short tokens"
    fi

    # ── create_deposit ────────────────────────────────────────────────────────
    echo -e "\n${BOLD}[4/5] create_deposit${NC}"

    local PARAMS
    PARAMS=$(python3 -c "
import json
p = {
    'receiver':            '$KEEPER_ADDR',
    'market':              '$MARKET_TOKEN',
    'initial_long_token':  '$LONG_TOKEN',
    'initial_short_token': '$SHORT_TOKEN',
    'long_token_amount':   str($SEED_LONG),
    'short_token_amount':  str($SEED_SHORT),
    'min_market_tokens':   '0',
    'execution_fee':       '0'
}
print(json.dumps(p))
")

    local DEP_KEY
    DEP_KEY=$(invoke_out "$DEPOSIT_HANDLER" create_deposit \
        --caller "$KEEPER_ADDR" \
        --params "$PARAMS" | tr -d '"')

    log "Deposit key: $DEP_KEY"
    ok "Deposit created"

    # ── Re-submit oracle prices (temp storage may expire between transactions) ──
    log "Re-submitting oracle prices before execute..."
    invoke "$ORACLE" set_prices \
        --caller "$KEEPER_ADDR" \
        --prices "$SIGNED_PRICES_ARG" && ok "Prices refreshed in oracle"

    # ── execute_deposit ───────────────────────────────────────────────────────
    echo -e "\n${BOLD}[5/5] execute_deposit${NC}"
    invoke "$DEPOSIT_HANDLER" execute_deposit \
        --keeper "$KEEPER_ADDR" \
        --key "$DEP_KEY" && ok "Deposit executed — GM tokens minted to $KEEPER_ADDR"
}

# ── Run for each market ────────────────────────────────────────────────────────
seed_market "TWBTC/TUSDC" \
    "$MARKET_TOKEN_TWBTC_TUSDC_LONG" \
    "$MARKET_TOKEN_TWBTC_TUSDC_SHORT" \
    "$MARKET_TOKEN_TWBTC_TUSDC"

seed_market "TETH/TUSDC" \
    "$MARKET_TOKEN_TETH_TUSDC_LONG" \
    "$MARKET_TOKEN_TETH_TUSDC_SHORT" \
    "$MARKET_TOKEN_TETH_TUSDC"

seed_market "TXLM/TUSDC" \
    "$MARKET_TOKEN_TXLM_TUSDC_LONG" \
    "$MARKET_TOKEN_TXLM_TUSDC_SHORT" \
    "$MARKET_TOKEN_TXLM_TUSDC"

echo -e "\n${GREEN}${BOLD}Liquidity seeding complete.${NC}"
echo "  Network : $NETWORK"
echo "  Source  : $SOURCE ($KEEPER_ADDR)"
echo ""
echo "Next: the oracle worker will auto-execute any future user deposits"
echo "      every minute via its Cloudflare cron trigger."
