#!/usr/bin/env bash
# scripts/bootstrap.sh — Bootstrap a testnet market after protocol deployment.
#
# Reads deployed contract addresses from .deployed/<NETWORK>.env and token
# addresses from .deployed/tokens-<NETWORK>.env, then:
#   1. Grants keeper roles (MARKET_KEEPER, ORDER_KEEPER, LIQUIDATION_KEEPER,
#      ADL_KEEPER, FEE_KEEPER) to the configured keeper account.
#   2. Creates a market via market_factory (index/long/short token triplet).
#   3. Writes all GMX-style per-market config keys via configure_market.sh.
#   4. Seeds the market with initial liquidity via deposit_handler.
#   5. Exports frontend contract addresses to .deployed/frontend-<NETWORK>.{env,ts}.
#
# Usage:
#   bash scripts/bootstrap.sh [NETWORK] [SOURCE_KEY]
#
#   NETWORK    : testnet (default) | local
#   SOURCE_KEY : stellar key name  (default: alice)
#
# Environment variables:
#   KEEPER          stellar key name for the keeper  (default: SOURCE_KEY)
#   LONG_TOKEN      contract ID of the long token    (read from TOKEN_ENV if set)
#   SHORT_TOKEN     contract ID of the short token   (read from TOKEN_ENV if set)
#   INDEX_TOKEN     contract ID of the index token   (defaults to LONG_TOKEN)
#   LONG_CODE       ticker code used in TOKEN_ENV    (default: TWBTC)
#   SHORT_CODE      ticker code used in TOKEN_ENV    (default: TUSDC)
#   SEED_LONG       long token amount to seed        (default: 10000000 = 1 token)
#   SEED_SHORT      short token amount to seed       (default: 10000000 = 1 token)
#   SKIP_ROLES      set to 1 to skip role grants     (default: 0)
#   SKIP_MARKET     set to 1 to skip market creation (default: 0)
#   SKIP_CONFIG     set to 1 to skip config keys     (default: 0)
#   SKIP_SEED       set to 1 to skip liquidity seed  (default: 0)

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
NETWORK="${1:-testnet}"
SOURCE="${2:-alice}"

DEPLOYED_DIR=".deployed"
DEPLOYED_ENV="$DEPLOYED_DIR/$NETWORK.env"
TOKEN_ENV="$DEPLOYED_DIR/tokens-$NETWORK.env"

# ── Colours ───────────────────────────────────────────────────────────────────
RED='\033[0;31m'; GREEN='\033[0;32m'; YELLOW='\033[1;33m'
CYAN='\033[0;36m'; BOLD='\033[1m'; NC='\033[0m'

log()  { echo -e "${CYAN}▸${NC} $*" >&2; }
ok()   { echo -e "  ${GREEN}✔${NC} $*"; }
warn() { echo -e "  ${YELLOW}⚠${NC} $*" >&2; }
die()  { echo -e "${RED}✖ $*${NC}" >&2; exit 1; }
sep()  { echo; }

# ── Preflight ─────────────────────────────────────────────────────────────────
command -v stellar >/dev/null 2>&1 || \
  die "stellar CLI not found. Install: cargo install stellar-cli --features opt"
command -v python3 >/dev/null 2>&1 || \
  die "python3 not found (required for market type + config key computation)"

[[ -f "$DEPLOYED_ENV" ]] || \
  die "Deployed addresses not found: $DEPLOYED_ENV\nRun 'make deploy-all NETWORK=$NETWORK SOURCE=$SOURCE' first."

# Load deployed contract addresses
# shellcheck source=/dev/null
source "$DEPLOYED_ENV"

ADMIN=$(stellar keys address "$SOURCE" 2>/dev/null) || \
  die "Key '$SOURCE' not found. Run: stellar keys generate --global $SOURCE --network $NETWORK"

# ── Config ────────────────────────────────────────────────────────────────────
KEEPER="${KEEPER:-$SOURCE}"
KEEPER_ADDR=$(stellar keys address "$KEEPER" 2>/dev/null) || KEEPER_ADDR="$ADMIN"

LONG_CODE="${LONG_CODE:-TWBTC}"
SHORT_CODE="${SHORT_CODE:-TUSDC}"

# Load token addresses from TOKEN_ENV if available
if [[ -f "$TOKEN_ENV" ]]; then
  source "$TOKEN_ENV"
fi

# Allow explicit override; fall back to TOKEN_ENV variable by code name
LONG_TOKEN="${LONG_TOKEN:-${!LONG_CODE:-}}"
SHORT_TOKEN="${SHORT_TOKEN:-${!SHORT_CODE:-}}"
INDEX_TOKEN="${INDEX_TOKEN:-$LONG_TOKEN}"

[[ -n "$LONG_TOKEN"  ]] || die "LONG_TOKEN not set. Run 'make test-tokens-with-faucet NETWORK=$NETWORK' first or set LONG_TOKEN=<contract_id>."
[[ -n "$SHORT_TOKEN" ]] || die "SHORT_TOKEN not set. Run 'make test-tokens-with-faucet NETWORK=$NETWORK' first or set SHORT_TOKEN=<contract_id>."

SEED_LONG="${SEED_LONG:-10000000}"
SEED_SHORT="${SEED_SHORT:-10000000}"

SKIP_ROLES="${SKIP_ROLES:-0}"
SKIP_MARKET="${SKIP_MARKET:-0}"
SKIP_CONFIG="${SKIP_CONFIG:-0}"
SKIP_SEED="${SKIP_SEED:-0}"

# ── Compute market_type = sha256("DEFAULT") ───────────────────────────────────
# This is the standard discriminant for fully-backed single-pair GM markets.
# Passed to create_market so the deterministic salt includes the type.
MARKET_TYPE=$(python3 "$SCRIPT_DIR/compute_key.py" market_type_default)

# ── Role keys — sha256(push_str(name)) matching libs/keys/src/lib.rs ──────────
# Each is sha256(2-byte-BE-length + UTF8-bytes), NOT plain sha256(name).
MARKET_KEEPER_ROLE="aa0c430a340620b0209835199d3ef24a66c03b650c655df7e818b120cbfaefb7"
ORDER_KEEPER_ROLE="95ec07fb03934d1fbf13fd192bc4cf29f950027fee71d7c1c7e7ffd07c06540b"
LIQUIDATION_KEEPER_ROLE="a3ac6772a11d5de2dcb186e89d1746fdc1104adb57a4b8012209147fc2ad637c"
ADL_KEEPER_ROLE="27dc3bca18550d6c651c2a746a71b73343ba95d7893676f1e3067c09583fb473"
FEE_KEEPER_ROLE="61b7a008df6cd00908ebf71fdbce8887aa4dfb4f1f419c0c9fb670b34f64a137"

# ── Helper ────────────────────────────────────────────────────────────────────
invoke() {
  local contract_id="$1"; shift
  stellar contract invoke \
    --id "$contract_id" \
    --source "$SOURCE" \
    --network "$NETWORK" \
    -- "$@" >/dev/null
}

invoke_out() {
  local contract_id="$1"; shift
  stellar contract invoke \
    --id "$contract_id" \
    --source "$SOURCE" \
    --network "$NETWORK" \
    -- "$@"
}

set_env_var() {
  local file="$1" key="$2" value="$3" tmp
  tmp="$(mktemp)"
  if [[ -f "$file" ]]; then
    grep -v -E "^${key}=" "$file" > "$tmp" || true
  fi
  printf '%s=%s\n' "$key" "$value" >> "$tmp"
  mv "$tmp" "$file"
}

# ── Header ────────────────────────────────────────────────────────────────────
echo -e "${BOLD}"
echo "  ██████╗  ██████╗  ██████╗ ████████╗"
echo "  ██╔══██╗██╔═══██╗██╔═══██╗╚══██╔══╝"
echo "  ██████╔╝██║   ██║██║   ██║   ██║   "
echo "  ██╔══██╗██║   ██║██║   ██║   ██║   "
echo "  ██████╔╝╚██████╔╝╚██████╔╝   ██║   "
echo "  ╚═════╝  ╚═════╝  ╚═════╝    ╚═╝   "
echo "           · bootstrap ·"
echo -e "${NC}"
echo -e "  Network      : ${CYAN}$NETWORK${NC}"
echo -e "  Admin        : ${CYAN}$SOURCE${NC}  ($ADMIN)"
echo -e "  Keeper       : ${CYAN}$KEEPER${NC}  ($KEEPER_ADDR)"
echo -e "  Long token   : ${CYAN}$LONG_CODE${NC}  $LONG_TOKEN"
echo -e "  Short token  : ${CYAN}$SHORT_CODE${NC}  $SHORT_TOKEN"
echo -e "  Index token  : ${CYAN}$LONG_CODE${NC}  $INDEX_TOKEN"
echo -e "  Market type  : ${CYAN}$MARKET_TYPE${NC}"
sep

# ── Step 1: Grant roles ────────────────────────────────────────────────────────
if [[ "$SKIP_ROLES" == "1" ]]; then
  warn "Skipping role grants (SKIP_ROLES=1)"
else
  echo -e "${BOLD}[1/5] Grant keeper roles${NC}"

  invoke "$ROLE_STORE" grant_role \
    --caller "$ADMIN" --account "$KEEPER_ADDR" --role "$MARKET_KEEPER_ROLE"
  ok "MARKET_KEEPER → $KEEPER_ADDR"

  invoke "$ROLE_STORE" grant_role \
    --caller "$ADMIN" --account "$KEEPER_ADDR" --role "$ORDER_KEEPER_ROLE"
  ok "ORDER_KEEPER → $KEEPER_ADDR"

  invoke "$ROLE_STORE" grant_role \
    --caller "$ADMIN" --account "$KEEPER_ADDR" --role "$LIQUIDATION_KEEPER_ROLE"
  ok "LIQUIDATION_KEEPER → $KEEPER_ADDR"

  invoke "$ROLE_STORE" grant_role \
    --caller "$ADMIN" --account "$KEEPER_ADDR" --role "$ADL_KEEPER_ROLE"
  ok "ADL_KEEPER → $KEEPER_ADDR"

  invoke "$ROLE_STORE" grant_role \
    --caller "$ADMIN" --account "$KEEPER_ADDR" --role "$FEE_KEEPER_ROLE"
  ok "FEE_KEEPER → $KEEPER_ADDR"
fi
sep

# ── Step 2: Create market ─────────────────────────────────────────────────────
if [[ "$SKIP_MARKET" == "1" ]]; then
  warn "Skipping market creation (SKIP_MARKET=1)"
  MARKET_TOKEN="${MARKET_TOKEN:?MARKET_TOKEN must be set when SKIP_MARKET=1}"
else
  echo -e "${BOLD}[2/5] Create market${NC}"

  MARKET_TOKEN=$(invoke_out "$MARKET_FACTORY" create_market \
    --caller      "$ADMIN" \
    --index_token "$INDEX_TOKEN" \
    --long_token  "$LONG_TOKEN" \
    --short_token "$SHORT_TOKEN" \
    --market_type "$MARKET_TYPE" \
    | python3 -c "import sys,json; d=sys.stdin.read().strip(); print(json.loads(d)['market_token'])")

  ok "market_token  $MARKET_TOKEN"

  # Derive the env key name from the token codes so multiple markets can coexist
  MARKET_KEY="MARKET_TOKEN_${LONG_CODE}_${SHORT_CODE}"

  # Persist market token address and related IDs. Re-running bootstrap should
  # replace the market entry instead of leaving stale duplicates behind.
  set_env_var "$DEPLOYED_ENV" "$MARKET_KEY" "$MARKET_TOKEN"
  set_env_var "$DEPLOYED_ENV" "MARKET_TOKEN" "$MARKET_TOKEN"
  set_env_var "$DEPLOYED_ENV" "${MARKET_KEY}_LONG" "$LONG_TOKEN"
  set_env_var "$DEPLOYED_ENV" "${MARKET_KEY}_SHORT" "$SHORT_TOKEN"
  set_env_var "$DEPLOYED_ENV" "${MARKET_KEY}_INDEX" "$INDEX_TOKEN"
fi
sep

# ── Step 3: Set market config keys ────────────────────────────────────────────
if [[ "$SKIP_CONFIG" == "1" ]]; then
  warn "Skipping config keys (SKIP_CONFIG=1)"
else
  echo -e "${BOLD}[3/5] Set market config keys${NC}"

  # Export vars so configure_market.sh picks them up without re-sourcing envs
  export DATA_STORE MARKET_TOKEN LONG_TOKEN SHORT_TOKEN

  bash "$SCRIPT_DIR/configure_market.sh" "$NETWORK" "$SOURCE"
fi
sep

# ── Step 4: Seed liquidity ─────────────────────────────────────────────────────
if [[ "$SKIP_SEED" == "1" ]]; then
  warn "Skipping liquidity seed (SKIP_SEED=1)"
else
  echo -e "${BOLD}[4/5] Seed initial liquidity${NC}"

  MT="${MARKET_TOKEN}"

  warn "Liquidity seeding requires oracle prices to be submitted first."
  warn "Run 'bash scripts/submit_prices.sh' before calling execute_deposit."
  warn ""
  warn "Manual seeding steps:"
  warn "  1. stellar contract invoke --id $LONG_TOKEN  -- approve --from $ADMIN --spender $DEPOSIT_HANDLER --amount $SEED_LONG  --expiration_ledger 999999"
  warn "  2. stellar contract invoke --id $SHORT_TOKEN -- approve --from $ADMIN --spender $DEPOSIT_HANDLER --amount $SEED_SHORT --expiration_ledger 999999"
  warn "  3. stellar contract invoke --id $DEPOSIT_HANDLER -- create_deposit \\"
  warn "       --caller $ADMIN --params '{ ... }'"
  warn "  4. stellar contract invoke --id $DEPOSIT_HANDLER -- execute_deposit \\"
  warn "       --keeper $KEEPER_ADDR --key <key>"
  warn ""
  warn "Or use 'make seed-liquidity MARKET_TOKEN=$MT NETWORK=$NETWORK SOURCE=$SOURCE'"
fi
sep

# ── Step 5: Export frontend config ────────────────────────────────────────────
echo -e "${BOLD}[5/5] Export frontend config${NC}"
bash "$SCRIPT_DIR/export_frontend_config.sh" "$NETWORK"
sep

# ── Summary ────────────────────────────────────────────────────────────────────
echo -e "${BOLD}Bootstrap complete${NC}"
echo -e "  Deployed env  : ${CYAN}$DEPLOYED_ENV${NC}"
echo -e "  Market token  : ${CYAN}$MARKET_TOKEN${NC}"
echo -e "  Frontend env  : ${CYAN}.deployed/frontend-$NETWORK.env${NC}"
echo -e "  Frontend TS   : ${CYAN}.deployed/frontend-$NETWORK.ts${NC}"
echo
echo "Next steps:"
echo "  1. Submit initial oracle prices:  bash scripts/submit_prices.sh"
echo "  2. Seed pool liquidity:           make seed-liquidity NETWORK=$NETWORK SOURCE=$SOURCE"
echo "  3. Repeat for other markets:      make bootstrap NETWORK=$NETWORK SOURCE=$SOURCE LONG_CODE=TETH SHORT_CODE=TUSDC"
