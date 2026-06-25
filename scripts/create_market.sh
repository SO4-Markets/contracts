#!/usr/bin/env bash
# scripts/create_market.sh — Add a new market to an already-running SO4 deployment.
#
# Assumes the protocol is fully deployed (.deployed/<NETWORK>.env exists) and
# keeper roles are already granted. Performs only:
#   1. market_factory::create_market  — deploys GM token, registers in data_store
#   2. configure_market.sh            — writes all per-market config keys
#   3. Prints oracle keeper registration command for the new market's index token
#
# Usage:
#   bash scripts/create_market.sh [NETWORK] [SOURCE_KEY]
#
#   NETWORK    : testnet (default) | local
#   SOURCE_KEY : stellar key name  (default: alice)
#
# Required env vars (read from .deployed/<NETWORK>.env and TOKEN_ENV, or set manually):
#   MARKET_FACTORY   contract ID of the market_factory
#   DATA_STORE       contract ID of the data_store
#   LONG_TOKEN       contract ID of the long (collateral) token
#   SHORT_TOKEN      contract ID of the short (collateral) token
#   INDEX_TOKEN      contract ID of the index token  (defaults to LONG_TOKEN)
#
# Optional overrides (passed through to configure_market.sh):
#   See configure_market.sh for the full list (MAX_OI, BORROWING_FACTOR, etc.)

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
NETWORK="${1:-testnet}"
SOURCE="${2:-alice}"

DEPLOYED_DIR=".deployed"
DEPLOYED_ENV="$DEPLOYED_DIR/$NETWORK.env"
TOKEN_ENV="$DEPLOYED_DIR/tokens-$NETWORK.env"

RED='\033[0;31m'; GREEN='\033[0;32m'; YELLOW='\033[1;33m'
CYAN='\033[0;36m'; BOLD='\033[1m'; NC='\033[0m'

log()  { echo -e "${CYAN}▸${NC} $*" >&2; }
ok()   { echo -e "  ${GREEN}✔${NC} $*"; }
warn() { echo -e "  ${YELLOW}⚠${NC} $*" >&2; }
die()  { echo -e "${RED}✖ $*${NC}" >&2; exit 1; }

# ── Preflight ─────────────────────────────────────────────────────────────────
command -v stellar  >/dev/null 2>&1 || die "stellar CLI not found"
command -v python3  >/dev/null 2>&1 || die "python3 not found"

[[ -f "$DEPLOYED_ENV" ]] || \
  die "Deployed addresses not found: $DEPLOYED_ENV\nRun 'make deploy-all NETWORK=$NETWORK SOURCE=$SOURCE' first."

source "$DEPLOYED_ENV"
[[ -f "$TOKEN_ENV" ]] && source "$TOKEN_ENV"

ADMIN=$(stellar keys address "$SOURCE" 2>/dev/null) || \
  die "Key '$SOURCE' not found."

MARKET_FACTORY="${MARKET_FACTORY:?MARKET_FACTORY not set in $DEPLOYED_ENV}"
DATA_STORE="${DATA_STORE:?DATA_STORE not set in $DEPLOYED_ENV}"
LONG_TOKEN="${LONG_TOKEN:?LONG_TOKEN not set}"
SHORT_TOKEN="${SHORT_TOKEN:?SHORT_TOKEN not set}"
INDEX_TOKEN="${INDEX_TOKEN:-$LONG_TOKEN}"

# ── Helpers ───────────────────────────────────────────────────────────────────
key() { python3 "$SCRIPT_DIR/compute_key.py" "$@"; }

set_env_var() {
  local file="$1" k="$2" v="$3" tmp
  tmp="$(mktemp)"
  [[ -f "$file" ]] && grep -v -E "^${k}=" "$file" > "$tmp" || true
  printf '%s=%s\n' "$k" "$v" >> "$tmp"
  mv "$tmp" "$file"
}

# ── Header ────────────────────────────────────────────────────────────────────
echo -e "${BOLD}Create Market${NC}"
echo -e "  Network      : ${CYAN}$NETWORK${NC}"
echo -e "  Admin        : ${CYAN}$ADMIN${NC}"
echo -e "  Long token   : ${CYAN}$LONG_TOKEN${NC}"
echo -e "  Short token  : ${CYAN}$SHORT_TOKEN${NC}"
echo -e "  Index token  : ${CYAN}$INDEX_TOKEN${NC}"
echo

# ── Step 1: Create market ─────────────────────────────────────────────────────
log "[1/3] Creating market via market_factory"

MARKET_TYPE=$(key market_type_default)

MARKET_TOKEN=$(stellar contract invoke \
  --id   "$MARKET_FACTORY" \
  --source "$SOURCE" \
  --network "$NETWORK" \
  -- create_market \
  --caller      "$ADMIN" \
  --index_token "$INDEX_TOKEN" \
  --long_token  "$LONG_TOKEN" \
  --short_token "$SHORT_TOKEN" \
  --market_type "$MARKET_TYPE" \
  | python3 -c "import sys,json; print(json.loads(sys.stdin.read().strip())['market_token'])")

ok "market_token = $MARKET_TOKEN"

# Persist: derive key name from the last 8 chars of each token to avoid collisions
LONG_SHORT_SUFFIX="${LONG_TOKEN: -8}_${SHORT_TOKEN: -8}"
MARKET_KEY="MARKET_TOKEN_$LONG_SHORT_SUFFIX"

set_env_var "$DEPLOYED_ENV" "$MARKET_KEY"       "$MARKET_TOKEN"
set_env_var "$DEPLOYED_ENV" "MARKET_TOKEN"      "$MARKET_TOKEN"
set_env_var "$DEPLOYED_ENV" "${MARKET_KEY}_LONG"  "$LONG_TOKEN"
set_env_var "$DEPLOYED_ENV" "${MARKET_KEY}_SHORT" "$SHORT_TOKEN"
set_env_var "$DEPLOYED_ENV" "${MARKET_KEY}_INDEX" "$INDEX_TOKEN"
ok "Saved to $DEPLOYED_ENV"

echo

# ── Step 2: Configure market parameters ──────────────────────────────────────
log "[2/3] Writing market config keys via configure_market.sh"

export DATA_STORE MARKET_TOKEN LONG_TOKEN SHORT_TOKEN
bash "$SCRIPT_DIR/configure_market.sh" "$NETWORK" "$SOURCE"

echo

# ── Step 3: Oracle keeper registration reminder ───────────────────────────────
log "[3/3] Oracle keeper setup"

KEEPER_KEY_HEX=$(key keeper_pubkey_key 0 2>/dev/null || echo "<run: python3 scripts/compute_key.py keeper_pubkey_key 0>")

echo -e "  Register a 32-byte ed25519 public key so the oracle can verify prices for"
echo -e "  index token ${CYAN}$INDEX_TOKEN${NC}."
echo
echo -e "  ${YELLOW}stellar contract invoke \\"
echo -e "    --id   \$DATA_STORE \\"
echo -e "    --source \$SOURCE \\"
echo -e "    --network $NETWORK \\"
echo -e "    -- set_bytes32 \\"
echo -e "    --caller \"\$ADMIN\" \\"
echo -e "    --key    \"$KEEPER_KEY_HEX\" \\"
echo -e "    --value  \"<32-byte-pubkey-hex>\"${NC}"
echo
echo -e "  See ${CYAN}docs/oracle.md${NC} for the full key-generation and registration steps."
echo

# ── Done ──────────────────────────────────────────────────────────────────────
echo -e "${GREEN}${BOLD}Market created${NC}"
echo -e "  market_token : ${CYAN}$MARKET_TOKEN${NC}"
echo -e "  Deployed env : ${CYAN}$DEPLOYED_ENV${NC}"
echo
echo "Next steps:"
echo "  1. Register oracle keeper pubkey (command above)"
echo "  2. Submit initial prices:  bash scripts/submit_prices.sh $NETWORK <keeper-key>"
echo "  3. Smoke test deposit:     bash scripts/seed_liquidity.sh $NETWORK $SOURCE"
