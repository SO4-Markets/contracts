#!/usr/bin/env bash
# Create and execute a small deposit, then prove that the account received GM tokens.

set -euo pipefail

NETWORK="${1:-testnet}"
SOURCE="${2:-alice}"
DEPLOY_ENV="${DEPLOY_ENV:-.deployed/$NETWORK.env}"
TOKEN_ENV="${TOKEN_ENV:-.deployed/tokens-$NETWORK.env}"
ORACLE_URL="${ORACLE_URL:-https://oracle.biscotti-proxy-worker.workers.dev}"
SMOKE_LONG_AMOUNT="${SMOKE_LONG_AMOUNT:-100000}"
SMOKE_SHORT_AMOUNT="${SMOKE_SHORT_AMOUNT:-100000}"

die() { printf 'smoke test failed: %s\n' "$*" >&2; exit 1; }
log() { printf '==> %s\n' "$*"; }

for command in stellar curl python3; do
  command -v "$command" >/dev/null 2>&1 || die "required command not found: $command"
done

[[ -f "$DEPLOY_ENV" ]] || die "missing $DEPLOY_ENV; deploy the protocol first"
# shellcheck source=/dev/null
source "$DEPLOY_ENV"
if [[ -f "$TOKEN_ENV" ]]; then
  # shellcheck source=/dev/null
  source "$TOKEN_ENV"
fi

ACCOUNT=$(stellar keys address "$SOURCE" 2>/dev/null) || die "Stellar identity not found: $SOURCE"

: "${ORACLE:?ORACLE is not set in $DEPLOY_ENV}"
: "${DEPOSIT_HANDLER:?DEPOSIT_HANDLER is not set in $DEPLOY_ENV}"
: "${MARKET_TOKEN:?Set MARKET_TOKEN or bootstrap a market first}"
: "${LONG_TOKEN:?Set LONG_TOKEN to the market long-token contract ID}"
: "${SHORT_TOKEN:?Set SHORT_TOKEN to the market short-token contract ID}"
INDEX_TOKEN="${INDEX_TOKEN:-$LONG_TOKEN}"
export ACCOUNT MARKET_TOKEN LONG_TOKEN SHORT_TOKEN INDEX_TOKEN
export SMOKE_LONG_AMOUNT SMOKE_SHORT_AMOUNT

invoke() {
  local contract_id="$1"
  shift
  stellar contract invoke \
    --id "$contract_id" \
    --source "$SOURCE" \
    --network "$NETWORK" \
    -- "$@"
}

json_scalar() {
  python3 -c 'import json,sys; value=json.load(sys.stdin); print(value)'
}

log "fetch signed oracle prices"
PRICES_JSON=$(curl --fail --silent --show-error "$ORACLE_URL/prices") || \
  die "could not fetch $ORACLE_URL/prices"

SIGNED_PRICES=$(printf '%s' "$PRICES_JSON" | python3 -c '
import json, os, sys

wanted = {os.environ["LONG_TOKEN"], os.environ["SHORT_TOKEN"], os.environ["INDEX_TOKEN"]}
prices = []
for item in json.load(sys.stdin):
    if item.get("token") not in wanted:
        continue
    prices.append({
        "token": item["token"],
        "min_price": str(item.get("min_price", item.get("min"))),
        "max_price": str(item.get("max_price", item.get("max"))),
        "timestamp": item["timestamp"],
        "signature": item["signature"],
        "keeper_index": item.get("keeper_index", 0),
        "ledger_seq": item["ledger_seq"],
    })
found = {item["token"] for item in prices}
missing = sorted(wanted - found)
if missing:
    raise SystemExit("oracle response has no signed price for: " + ", ".join(missing))
print(json.dumps(prices, separators=(",", ":")))
') || die "signed-price response does not cover this market"

submit_prices() {
  invoke "$ORACLE" set_prices --caller "$ACCOUNT" --prices "$SIGNED_PRICES" >/dev/null
}

log "submit signed prices"
submit_prices

BALANCE_BEFORE=$(invoke "$MARKET_TOKEN" balance --id "$ACCOUNT" | json_scalar)
PARAMS=$(python3 -c '
import json, os
print(json.dumps({
    "receiver": os.environ["ACCOUNT"],
    "market": os.environ["MARKET_TOKEN"],
    "initial_long_token": os.environ["LONG_TOKEN"],
    "initial_short_token": os.environ["SHORT_TOKEN"],
    "long_token_amount": os.environ["SMOKE_LONG_AMOUNT"],
    "short_token_amount": os.environ["SMOKE_SHORT_AMOUNT"],
    "min_market_tokens": "1",
    "execution_fee": "0",
}, separators=(",", ":")))
')

log "create deposit"
DEPOSIT_KEY=$(invoke "$DEPOSIT_HANDLER" create_deposit \
  --caller "$ACCOUNT" \
  --params "$PARAMS" | json_scalar)

# Prices use temporary storage; refresh them immediately before execution.
log "refresh prices and execute deposit $DEPOSIT_KEY"
submit_prices
invoke "$DEPOSIT_HANDLER" execute_deposit --keeper "$ACCOUNT" --key "$DEPOSIT_KEY" >/dev/null

BALANCE_AFTER=$(invoke "$MARKET_TOKEN" balance --id "$ACCOUNT" | json_scalar)
TOTAL_SUPPLY=$(invoke "$MARKET_TOKEN" total_supply | json_scalar)

python3 - "$BALANCE_BEFORE" "$BALANCE_AFTER" "$TOTAL_SUPPLY" <<'PY'
import sys

before, after, supply = map(int, sys.argv[1:])
if after <= before:
    raise SystemExit(f"GM balance did not increase: before={before}, after={after}")
if supply <= 0:
    raise SystemExit(f"market token total supply is not positive: {supply}")
print(f"smoke test passed: GM balance {before} -> {after}; total supply={supply}")
PY
