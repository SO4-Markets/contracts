#!/usr/bin/env bash
# scripts/deploy.sh — Deploy all SO4.market contracts in dependency order.
#
# Usage:
#   bash scripts/deploy.sh [NETWORK] [SOURCE_KEY]
#
#   NETWORK    : testnet (default) | mainnet | local
#   SOURCE_KEY : stellar key name  (default: alice)
#
# Outputs:
#   - Progress log to stdout
#   - Addresses saved to .deployed/<NETWORK>.env
#   - Summary table printed at completion

set -euo pipefail

# ── Args & config ─────────────────────────────────────────────────────────────
NETWORK="${1:-testnet}"
SOURCE="${2:-alice}"
WASM_DIR="target/wasm32-unknown-unknown/release"
OUT_DIR=".deployed"
OUT_FILE="$OUT_DIR/$NETWORK.env"

# ── Colours ───────────────────────────────────────────────────────────────────
RED='\033[0;31m'; GREEN='\033[0;32m'; YELLOW='\033[1;33m'
CYAN='\033[0;36m'; BOLD='\033[1m'; NC='\033[0m'

log()  { echo -e "${CYAN}▸${NC} $*"; }
ok()   { echo -e "  ${GREEN}✔${NC} $*"; }
warn() { echo -e "  ${YELLOW}⚠${NC} $*"; }
die()  { echo -e "${RED}✖ $*${NC}"; exit 1; }
sep()  { echo; }

# ── Preflight checks ──────────────────────────────────────────────────────────
command -v stellar >/dev/null 2>&1 || \
  die "stellar CLI not found. Install: cargo install stellar-cli --features opt"
command -v cargo >/dev/null 2>&1 || \
  die "cargo not found. Install Rust from https://rustup.rs"

if [[ "$NETWORK" == "mainnet" ]]; then
  warn "Deploying to MAINNET. This costs real XLM."
  warn "Press Ctrl-C within 5 seconds to abort."
  sleep 5
fi

ADMIN=$(stellar keys address "$SOURCE" 2>/dev/null) || \
  die "Key '$SOURCE' not found. Run: stellar keys generate --global $SOURCE --network $NETWORK"

echo -e "${BOLD}"
echo "  ███████╗ ██████╗ ██╗  ██╗"
echo "  ██╔════╝██╔═══██╗██║  ██║"
echo "  ███████╗██║   ██║███████║"
echo "  ╚════██║██║   ██║╚════██║"
echo "  ███████║╚██████╔╝     ██║"
echo "  ╚══════╝ ╚═════╝      ╚═╝  · deploy ·"
echo -e "${NC}"
echo -e "  Network : ${CYAN}$NETWORK${NC}"
echo -e "  Source  : ${CYAN}$SOURCE${NC}  ($ADMIN)"
echo -e "  Output  : ${CYAN}$OUT_FILE${NC}"
sep

# ── Helpers ───────────────────────────────────────────────────────────────────

# Upload a wasm blob and return its hash.
upload() {
  local label="$1" file="$WASM_DIR/$2"
  [[ -f "$file" ]] || die "wasm not found: $file  (run 'stellar contract build' first)"
  log "upload  $label"
  stellar contract upload \
    --wasm "$file" \
    --source "$SOURCE" \
    --network "$NETWORK" \
    2>/dev/null
}

# Deploy a wasm hash and return the new contract address.
deploy_contract() {
  local label="$1" hash="$2"
  log "deploy  $label"
  stellar contract deploy \
    --wasm-hash "$hash" \
    --source "$SOURCE" \
    --network "$NETWORK" \
    2>/dev/null
}

# Invoke a contract function (fire-and-forget, output suppressed).
invoke() {
  local contract_id="$1"; shift
  stellar contract invoke \
    --id "$contract_id" \
    --source "$SOURCE" \
    --network "$NETWORK" \
    -- "$@" >/dev/null 2>&1
}

# ── Step 1: Build ─────────────────────────────────────────────────────────────
echo -e "${BOLD}[1/8] Build${NC}"
stellar contract build
ok "all contracts compiled"
sep

# ── Step 2: Upload all wasm blobs ─────────────────────────────────────────────
echo -e "${BOLD}[2/8] Upload wasm blobs${NC}"

ROLE_STORE_HASH=$(upload         "role_store"          "role_store.wasm")
DATA_STORE_HASH=$(upload         "data_store"          "data_store.wasm")
ORACLE_HASH=$(upload             "oracle"              "oracle.wasm")
MARKET_TOKEN_HASH=$(upload       "market_token"        "market_token.wasm")
MARKET_FACTORY_HASH=$(upload     "market_factory"      "market_factory.wasm")
DEPOSIT_VAULT_HASH=$(upload      "deposit_vault"       "deposit_vault.wasm")
DEPOSIT_HANDLER_HASH=$(upload    "deposit_handler"     "deposit_handler.wasm")
WITHDRAWAL_VAULT_HASH=$(upload   "withdrawal_vault"    "withdrawal_vault.wasm")
WITHDRAWAL_HANDLER_HASH=$(upload "withdrawal_handler"  "withdrawal_handler.wasm")
ORDER_VAULT_HASH=$(upload        "order_vault"         "order_vault.wasm")
ORDER_HANDLER_HASH=$(upload      "order_handler"       "order_handler.wasm")
LIQ_HANDLER_HASH=$(upload        "liquidation_handler" "liquidation_handler.wasm")
ADL_HANDLER_HASH=$(upload        "adl_handler"         "adl_handler.wasm")
FEE_HANDLER_HASH=$(upload        "fee_handler"         "fee_handler.wasm")
REFERRAL_HASH=$(upload           "referral_storage"    "referral_storage.wasm")
READER_HASH=$(upload             "reader"              "reader.wasm")
ROUTER_HASH=$(upload             "exchange_router"     "exchange_router.wasm")

ok "17 blobs uploaded"
sep

# ── Step 3: Core contracts ─────────────────────────────────────────────────────
echo -e "${BOLD}[3/8] Core contracts${NC}"

ROLE_STORE=$(deploy_contract "role_store" "$ROLE_STORE_HASH")
invoke "$ROLE_STORE" initialize --admin "$ADMIN"
ok "role_store    $ROLE_STORE"

DATA_STORE=$(deploy_contract "data_store" "$DATA_STORE_HASH")
invoke "$DATA_STORE" initialize --role_store "$ROLE_STORE"
ok "data_store    $DATA_STORE"

ORACLE=$(deploy_contract "oracle" "$ORACLE_HASH")
invoke "$ORACLE" initialize \
  --admin "$ADMIN" \
  --role_store "$ROLE_STORE" \
  --data_store "$DATA_STORE"
ok "oracle        $ORACLE"
sep

# ── Step 4: Market factory ─────────────────────────────────────────────────────
echo -e "${BOLD}[4/8] Market factory${NC}"

MARKET_FACTORY=$(deploy_contract "market_factory" "$MARKET_FACTORY_HASH")
invoke "$MARKET_FACTORY" initialize \
  --admin "$ADMIN" \
  --role_store "$ROLE_STORE" \
  --data_store "$DATA_STORE" \
  --market_token_wasm_hash "$MARKET_TOKEN_HASH"
ok "market_factory  $MARKET_FACTORY"
sep

# ── Step 5: Vaults + handlers ─────────────────────────────────────────────────
echo -e "${BOLD}[5/8] Vaults and handlers${NC}"

DEPOSIT_VAULT=$(deploy_contract "deposit_vault" "$DEPOSIT_VAULT_HASH")
invoke "$DEPOSIT_VAULT" initialize --admin "$ADMIN" --role_store "$ROLE_STORE"
ok "deposit_vault       $DEPOSIT_VAULT"

DEPOSIT_HANDLER=$(deploy_contract "deposit_handler" "$DEPOSIT_HANDLER_HASH")
invoke "$DEPOSIT_HANDLER" initialize \
  --admin "$ADMIN" \
  --role_store "$ROLE_STORE" \
  --data_store "$DATA_STORE" \
  --oracle "$ORACLE" \
  --deposit_vault "$DEPOSIT_VAULT"
ok "deposit_handler     $DEPOSIT_HANDLER"

WITHDRAWAL_VAULT=$(deploy_contract "withdrawal_vault" "$WITHDRAWAL_VAULT_HASH")
invoke "$WITHDRAWAL_VAULT" initialize --admin "$ADMIN" --role_store "$ROLE_STORE"
ok "withdrawal_vault    $WITHDRAWAL_VAULT"

WITHDRAWAL_HANDLER=$(deploy_contract "withdrawal_handler" "$WITHDRAWAL_HANDLER_HASH")
invoke "$WITHDRAWAL_HANDLER" initialize \
  --admin "$ADMIN" \
  --role_store "$ROLE_STORE" \
  --data_store "$DATA_STORE" \
  --oracle "$ORACLE" \
  --withdrawal_vault "$WITHDRAWAL_VAULT"
ok "withdrawal_handler  $WITHDRAWAL_HANDLER"

ORDER_VAULT=$(deploy_contract "order_vault" "$ORDER_VAULT_HASH")
invoke "$ORDER_VAULT" initialize --admin "$ADMIN" --role_store "$ROLE_STORE"
ok "order_vault         $ORDER_VAULT"

ORDER_HANDLER=$(deploy_contract "order_handler" "$ORDER_HANDLER_HASH")
invoke "$ORDER_HANDLER" initialize \
  --admin "$ADMIN" \
  --role_store "$ROLE_STORE" \
  --data_store "$DATA_STORE" \
  --oracle "$ORACLE" \
  --order_vault "$ORDER_VAULT"
ok "order_handler       $ORDER_HANDLER"
sep

# ── Step 6: Risk handlers + periphery ─────────────────────────────────────────
echo -e "${BOLD}[6/8] Risk handlers and periphery${NC}"

LIQUIDATION_HANDLER=$(deploy_contract "liquidation_handler" "$LIQ_HANDLER_HASH")
invoke "$LIQUIDATION_HANDLER" initialize \
  --admin "$ADMIN" \
  --role_store "$ROLE_STORE" \
  --data_store "$DATA_STORE" \
  --oracle "$ORACLE" \
  --order_handler "$ORDER_HANDLER"
ok "liquidation_handler  $LIQUIDATION_HANDLER"

ADL_HANDLER=$(deploy_contract "adl_handler" "$ADL_HANDLER_HASH")
invoke "$ADL_HANDLER" initialize \
  --admin "$ADMIN" \
  --role_store "$ROLE_STORE" \
  --data_store "$DATA_STORE" \
  --oracle "$ORACLE" \
  --order_handler "$ORDER_HANDLER"
ok "adl_handler          $ADL_HANDLER"

FEE_HANDLER=$(deploy_contract "fee_handler" "$FEE_HANDLER_HASH")
invoke "$FEE_HANDLER" initialize \
  --admin "$ADMIN" \
  --role_store "$ROLE_STORE" \
  --data_store "$DATA_STORE"
ok "fee_handler          $FEE_HANDLER"

REFERRAL_STORAGE=$(deploy_contract "referral_storage" "$REFERRAL_HASH")
invoke "$REFERRAL_STORAGE" initialize --admin "$ADMIN"
ok "referral_storage     $REFERRAL_STORAGE"

READER=$(deploy_contract "reader" "$READER_HASH")
ok "reader               $READER"
sep

# ── Step 7: Exchange router ────────────────────────────────────────────────────
echo -e "${BOLD}[7/8] Exchange router${NC}"

EXCHANGE_ROUTER=$(deploy_contract "exchange_router" "$ROUTER_HASH")
invoke "$EXCHANGE_ROUTER" initialize \
  --admin "$ADMIN" \
  --role_store "$ROLE_STORE" \
  --data_store "$DATA_STORE" \
  --deposit_handler "$DEPOSIT_HANDLER" \
  --withdrawal_handler "$WITHDRAWAL_HANDLER" \
  --order_handler "$ORDER_HANDLER" \
  --fee_handler "$FEE_HANDLER"
ok "exchange_router  $EXCHANGE_ROUTER"
sep

# ── Step 8: Grant roles ────────────────────────────────────────────────────────
echo -e "${BOLD}[8/8] Grant CONTROLLER role${NC}"

for CONTRACT in \
  "$DEPOSIT_HANDLER" \
  "$WITHDRAWAL_HANDLER" \
  "$ORDER_HANDLER" \
  "$LIQUIDATION_HANDLER" \
  "$ADL_HANDLER" \
  "$FEE_HANDLER" \
  "$EXCHANGE_ROUTER"
do
  invoke "$ROLE_STORE" grant_role --account "$CONTRACT" --role CONTROLLER
  ok "CONTROLLER → $CONTRACT"
done
sep

# ── Save addresses to file ─────────────────────────────────────────────────────
mkdir -p "$OUT_DIR"
TIMESTAMP=$(date -u +"%Y-%m-%dT%H:%M:%SZ")
cat > "$OUT_FILE" <<ENV
# SO4.market — deployed addresses
# Network   : $NETWORK
# Admin     : $ADMIN
# Timestamp : $TIMESTAMP

NETWORK=$NETWORK
ADMIN=$ADMIN

ROLE_STORE=$ROLE_STORE
DATA_STORE=$DATA_STORE
ORACLE=$ORACLE
MARKET_FACTORY=$MARKET_FACTORY
MARKET_TOKEN_WASM_HASH=$MARKET_TOKEN_HASH

DEPOSIT_VAULT=$DEPOSIT_VAULT
DEPOSIT_HANDLER=$DEPOSIT_HANDLER
WITHDRAWAL_VAULT=$WITHDRAWAL_VAULT
WITHDRAWAL_HANDLER=$WITHDRAWAL_HANDLER
ORDER_VAULT=$ORDER_VAULT
ORDER_HANDLER=$ORDER_HANDLER

LIQUIDATION_HANDLER=$LIQUIDATION_HANDLER
ADL_HANDLER=$ADL_HANDLER
FEE_HANDLER=$FEE_HANDLER
REFERRAL_STORAGE=$REFERRAL_STORAGE
READER=$READER
EXCHANGE_ROUTER=$EXCHANGE_ROUTER
ENV

# ── Summary table ─────────────────────────────────────────────────────────────
W=56  # address column width
DIV=$(printf '═%.0s' $(seq 1 $((W + 30))))

echo -e "${BOLD}$DIV${NC}"
echo -e "${BOLD}  SO4.market — $NETWORK — $TIMESTAMP${NC}"
echo -e "${BOLD}$DIV${NC}"
printf "  ${BOLD}%-22s  %-${W}s${NC}\n" "Contract" "Address"
printf "  %-22s  %-${W}s\n" "──────────────────────" "$(printf '─%.0s' $(seq 1 $W))"
printf "  ${GREEN}%-22s${NC}  %s\n" "role_store"          "$ROLE_STORE"
printf "  ${GREEN}%-22s${NC}  %s\n" "data_store"          "$DATA_STORE"
printf "  ${GREEN}%-22s${NC}  %s\n" "oracle"              "$ORACLE"
printf "  ${GREEN}%-22s${NC}  %s\n" "market_factory"      "$MARKET_FACTORY"
printf "  ${GREEN}%-22s${NC}  %s\n" "deposit_vault"       "$DEPOSIT_VAULT"
printf "  ${GREEN}%-22s${NC}  %s\n" "deposit_handler"     "$DEPOSIT_HANDLER"
printf "  ${GREEN}%-22s${NC}  %s\n" "withdrawal_vault"    "$WITHDRAWAL_VAULT"
printf "  ${GREEN}%-22s${NC}  %s\n" "withdrawal_handler"  "$WITHDRAWAL_HANDLER"
printf "  ${GREEN}%-22s${NC}  %s\n" "order_vault"         "$ORDER_VAULT"
printf "  ${GREEN}%-22s${NC}  %s\n" "order_handler"       "$ORDER_HANDLER"
printf "  ${GREEN}%-22s${NC}  %s\n" "liquidation_handler" "$LIQUIDATION_HANDLER"
printf "  ${GREEN}%-22s${NC}  %s\n" "adl_handler"         "$ADL_HANDLER"
printf "  ${GREEN}%-22s${NC}  %s\n" "fee_handler"         "$FEE_HANDLER"
printf "  ${GREEN}%-22s${NC}  %s\n" "referral_storage"    "$REFERRAL_STORAGE"
printf "  ${GREEN}%-22s${NC}  %s\n" "reader"              "$READER"
printf "  ${GREEN}%-22s${NC}  %s\n" "exchange_router"     "$EXCHANGE_ROUTER"
echo -e "${BOLD}$DIV${NC}"
echo -e "  Addresses saved → ${CYAN}$OUT_FILE${NC}"
echo -e "${BOLD}$DIV${NC}"
