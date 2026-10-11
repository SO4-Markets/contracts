#!/usr/bin/env bash
# scripts/submit_prices.sh — Submit signed token price(s) to the SO4.market oracle.
#
# Usage:
#   TOKEN=<contract_id|symbol> [ORACLE_URL=<url>] \
#     bash scripts/submit_prices.sh [NETWORK] [SOURCE_KEY]
#
#   Or with explicit signed bundle:
#   TOKEN=<contract_id> MIN_PRICE=<i128> MAX_PRICE=<i128> SIGNATURE=<hex> \
#     [LEDGER_SEQ=<u32>] [TIMESTAMP=<u64>] [KEEPER_INDEX=<u32>] \
#     bash scripts/submit_prices.sh [NETWORK] [SOURCE_KEY]
#
# Required env vars:
#   ORACLE      Address of the deployed oracle contract.
#   TOKEN       Address or symbol of the token whose price you are submitting.
#
# Optional:
#   ORACLE_URL   URL to oracle worker providing signed prices
#                (default: https://oracle.biscotti-proxy-worker.workers.dev)
#   MIN_PRICE    Minimum price (integer, scaled to protocol FLOAT_PRECISION).
#   MAX_PRICE    Maximum price (integer, scaled to protocol FLOAT_PRECISION).
#   SIGNATURE    64-byte hex ed25519 signature from authorized keeper.
#   LEDGER_SEQ   Ledger sequence number associated with signature.
#   TIMESTAMP    Unix timestamp in seconds associated with signature.
#   KEEPER_INDEX Index of the keeper in oracle data_store (default: 0).
#   NETWORK      testnet (default) | mainnet | local
#   SOURCE_KEY   Stellar key name of an authorized order-keeper (default: alice)
#
# The caller (SOURCE_KEY) must hold the ORDER_KEEPER role in role_store before
# this script will succeed. Register a keeper with:
#   make grant-keeper SOURCE=admin KEEPER=alice NETWORK=testnet
#
# Example (signed feed via oracle worker):
#   ORACLE=C... TOKEN=TUSDC bash scripts/submit_prices.sh testnet alice
#
# Example (explicit signed price):
#   ORACLE=C... TOKEN=C... MIN_PRICE=500000000000000000000000000000000 \
#   MAX_PRICE=500500000000000000000000000000000 SIGNATURE=5f26... \
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
[[ -v ORACLE && -n "${ORACLE}" ]] || \
  die "ORACLE is not set. Export the deployed oracle contract address:\n  export ORACLE=C..."

[[ -v TOKEN && -n "${TOKEN}" ]] || \
  die "TOKEN is not set. Export the token contract address or symbol:\n  export TOKEN=TUSDC"

# ── Preflight ─────────────────────────────────────────────────────────────────
command -v stellar >/dev/null 2>&1 || \
  die "stellar CLI not found. Install: cargo install stellar-cli --features opt"

stellar keys address "$SOURCE" >/dev/null 2>&1 || \
  die "Key '$SOURCE' not found. Run: stellar keys generate --global $SOURCE --network $NETWORK"

PYTHON=""
for py in python3 python; do
  if command -v "$py" >/dev/null 2>&1; then
    PYTHON="$py"
    break
  fi
done
[[ -n "$PYTHON" ]] || die "python3 or python is required to format oracle price payloads."

# ── Build signed prices payload ───────────────────────────────────────────────
if [[ -n "${SIGNATURE:-}" ]]; then
  [[ -v MIN_PRICE && -n "${MIN_PRICE}" ]] || \
    die "MIN_PRICE is required when providing explicit SIGNATURE."
  [[ -v MAX_PRICE && -n "${MAX_PRICE}" ]] || \
    die "MAX_PRICE is required when providing explicit SIGNATURE."

  if (( MIN_PRICE > MAX_PRICE )); then
    die "MIN_PRICE ($MIN_PRICE) is greater than MAX_PRICE ($MAX_PRICE)."
  fi

  TS="${TIMESTAMP:-$(date +%s)}"
  SEQ="${LEDGER_SEQ:-0}"
  K_IDX="${KEEPER_INDEX:-0}"
  SIGNED_PRICES_ARG="[{\"token\":\"$TOKEN\",\"min_price\":\"$MIN_PRICE\",\"max_price\":\"$MAX_PRICE\",\"timestamp\":$TS,\"signature\":\"$SIGNATURE\",\"keeper_index\":$K_IDX,\"ledger_seq\":$SEQ}]"
else
  ORACLE_URL="${ORACLE_URL:-https://oracle.biscotti-proxy-worker.workers.dev}"
  printf "${CYAN}▸${NC} Fetching signed price for %s from %s/prices...\n" "$TOKEN" "$ORACLE_URL" >&2
  PRICES_JSON=$(curl -sSf -H "User-Agent: curl/7.68.0" "$ORACLE_URL/prices") || \
    die "Could not fetch signed prices from $ORACLE_URL/prices. If submitting manually, pass SIGNATURE=<hex>."

  SIGNED_PRICES_ARG=$(echo "$PRICES_JSON" | "$PYTHON" -c "
import sys, json

data = json.load(sys.stdin)
target = '$TOKEN'
entries = []
for p in data:
    if p.get('token') == target or p.get('symbol') == target:
        min_p = str(p.get('min_price', p.get('min')))
        max_p = str(p.get('max_price', p.get('max')))
        entries.append({
            'token': p['token'],
            'min_price': min_p,
            'max_price': max_p,
            'timestamp': int(p['timestamp']),
            'signature': p['signature'],
            'keeper_index': int(p.get('keeper_index', 0)),
            'ledger_seq': int(p['ledger_seq'])
        })
        break

if not entries:
    symbols = [p.get('symbol') or p.get('token') for p in data]
    print(f'ERROR: token \'{target}\' not found in signed prices feed (available: {symbols})', file=sys.stderr)
    sys.exit(1)

print(json.dumps(entries))
") || die "Failed to build signed price payload for $TOKEN."
fi

# ── Submit prices via real signed set_prices entrypoint ───────────────────────
printf "${CYAN}▸${NC} Submitting signed price for %s on %s via %s\n" "$TOKEN" "$NETWORK" "$SOURCE" >&2

stellar contract invoke \
  --id    "$ORACLE" \
  --source "$SOURCE" \
  --network "$NETWORK" \
  -- set_prices \
  --caller "$(stellar keys address "$SOURCE")" \
  --prices "$SIGNED_PRICES_ARG"

ok "signed price submitted  token=$TOKEN  network=$NETWORK"
