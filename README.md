# GMX Synthetics — Soroban

A faithful re-implementation of [GMX Synthetics](https://github.com/gmx-io/gmx-synthetics) on [Stellar](https://stellar.org) using [Soroban](https://soroban.stellar.org) smart contracts (SDK 25, Rust).

GMX Synthetics is a decentralised perpetuals and spot exchange. This port preserves the full financial model — isolated markets, LP pools, two-step keeper execution, dynamic funding rates, borrowing fees, price impact, liquidations, and auto-deleveraging — adapted to Soroban's execution environment.

---

## Architecture

```
ExchangeRouter  ──► DepositHandler    ──► (mint LP tokens, update pool)
                ──► WithdrawalHandler ──► (burn LP tokens, return collateral)
                ──► OrderHandler      ──► IncreasePositionUtils
                                      ──► DecreasePositionUtils
                                      ──► SwapUtils
                ──► LiquidationHandler
                ──► AdlHandler

All handlers read/write ──► DataStore  (universal key-value store)
All handlers price via  ──► Oracle     (keeper-fed min/max price pairs)
All handlers check      ──► RoleStore  (role-based access control)

MarketFactory  ──► deploy MarketToken (SEP-41 LP) + register in DataStore
Reader         ──► stateless views over DataStore (no writes)
```

### Contract Map

| Contract | Description |
|---|---|
| `data_store` | Universal typed key-value store. All protocol state lives here. |
| `role_store` | Role-based access control — CONTROLLER, MARKET_KEEPER, ORDER_KEEPER, etc. |
| `oracle` | Keeper-fed price store. Prices are temporary (expire per ledger). Ed25519-verified. |
| `market_token` | SEP-41 LP token deployed per market by `market_factory`. |
| `market_factory` | Deterministically deploys `market_token` instances and registers markets. |
| `deposit_vault` | Holds long/short tokens between deposit creation and keeper execution. |
| `deposit_handler` | Two-step deposit lifecycle: create → (keeper) execute / cancel. |
| `withdrawal_vault` | Holds LP tokens between withdrawal creation and keeper execution. |
| `withdrawal_handler` | Two-step withdrawal lifecycle: create → (keeper) execute / cancel. |
| `order_vault` | Holds collateral for pending orders. |
| `order_handler` | Full order lifecycle with routing by `OrderType` to position/swap utils. |
| `liquidation_handler` | Force-closes under-collateralised positions (LIQUIDATION_KEEPER). |
| `adl_handler` | Auto-deleverages profitable positions when pool PnL exceeds threshold. |
| `fee_handler` | Claims accumulated protocol fees and user funding fee credits. |
| `referral_storage` | On-chain referral code registry with tier-based rebate/discount config. |
| `reader` | Read-only aggregate views: positions, markets, OI, funding, liquidation checks. |
| `exchange_router` | Single user entry point. Supports multicall for atomic multi-step actions. |

### Shared Libraries

| Crate | Description |
|---|---|
| `libs/types` | All shared structs: `MarketProps`, `PositionProps`, `OrderProps`, `PriceProps`, `PositionFees`, etc. |
| `libs/math` | `FLOAT_PRECISION` (10³⁰), `TOKEN_PRECISION` (10⁷), `mul_div_wide` (I256), `pow_factor`, `sqrt_fp`. |
| `libs/keys` | ~58 deterministic `sha256`-based key derivation functions mirroring GMX's `Keys.sol`. |
| `libs/market_utils` | Pool value, open interest, PnL, funding state, borrowing fees, pool/OI validation. |
| `libs/position_utils` | Per-position PnL, fee breakdown, funding settlement, leverage validation, liquidation check. |
| `libs/pricing_utils` | Swap and position price impact, execution price, impact pool management. |
| `libs/swap_utils` | Single-hop and multi-hop token swaps through market pools. |
| `libs/increase_position_utils` | Open or increase a long/short position (14-step GMX-faithful flow). |
| `libs/decrease_position_utils` | Partial or full position close with PnL settlement (14-step GMX-faithful flow). |

---

## Key Financial Mechanics

### Price Precision
- All USD values use `FLOAT_PRECISION = 10^30` (matching GMX).
- All token amounts use `TOKEN_PRECISION = 10^7` (Stellar's 7-decimal standard).
- Wide multiplication via Soroban's `I256` host functions prevents overflow.

### LP Minting
```
mint_amount        = deposit_usd × TOKEN_PRECISION / market_token_price
market_token_price = pool_value / lp_supply  (1 USD on first deposit)
```

### Price Impact
```
initial_diff = |sideA_usd - sideB_usd|
next_diff    = |after_delta|
positive_impact (improves balance) → paid from impact pool, capped by pool balance
negative_impact (worsens balance)  → paid into impact pool
impact = factor × (diff ^ exponent)
```

### Funding Rate
```
funding_factor_per_second = funding_factor × (|long_oi - short_oi| / total_oi) ^ exponent
funding_amount_per_size  += factor_per_second × dt × index_token_price
```

### Borrowing Fee
```
cumulative_borrowing_factor += borrowing_factor × dt × (open_interest / pool_amount)
position_borrow_fee          = (cumulative_factor_now - factor_at_open) × size_in_tokens
```

### Liquidation
```
remaining   = collateral_usd - borrowing_fees - funding_fees + unrealised_pnl
liquidatable when: remaining < min_collateral_factor × position_size_usd
```

---

## Prerequisites

**Rust** with the `wasm32-unknown-unknown` target:
```bash
rustup target add wasm32-unknown-unknown
```

**Stellar CLI:**
```bash
cargo install --locked stellar-cli --features opt
```

**A funded Stellar account** (testnet):
```bash
stellar keys generate --global alice --network testnet
stellar keys fund alice --network testnet
```

---

## Build

### Type-check only (fastest, no wasm output)
```bash
cargo check
```

### Build all contracts to wasm
```bash
stellar contract build
```
Output: `target/wasm32-unknown-unknown/release/<contract_name>.wasm`

### Build a single contract
```bash
stellar contract build --package data-store
stellar contract build --package oracle
stellar contract build --package order-handler
stellar contract build --package exchange-router
```

### Optimised release build
```bash
stellar contract build --release
```

---

## Test

### Run the full test suite
```bash
cargo test --workspace
```

### Test a specific crate
```bash
cargo test -p gmx-market-utils
cargo test -p gmx-math
cargo test -p oracle
cargo test -p data-store
cargo test -p role-store
```

### Run a single test by name
```bash
cargo test -p gmx-market-utils apply_delta_to_pool_amount_works
cargo test -p oracle set_and_get_price
```

### Show test output (disable capture)
```bash
cargo test --workspace -- --nocapture
```

---

## Deploy to Testnet

Contracts must be deployed in dependency order. Complete sequence below.

### Step 1 — Build
```bash
stellar contract build
```

### Step 2 — Upload wasm blobs

Each upload returns a `WASM_HASH`. Record each one.

```bash
stellar contract upload \
  --wasm target/wasm32-unknown-unknown/release/role_store.wasm \
  --source alice --network testnet

stellar contract upload \
  --wasm target/wasm32-unknown-unknown/release/data_store.wasm \
  --source alice --network testnet

stellar contract upload \
  --wasm target/wasm32-unknown-unknown/release/oracle.wasm \
  --source alice --network testnet

stellar contract upload \
  --wasm target/wasm32-unknown-unknown/release/market_token.wasm \
  --source alice --network testnet

stellar contract upload \
  --wasm target/wasm32-unknown-unknown/release/market_factory.wasm \
  --source alice --network testnet

# Vaults and handlers
stellar contract upload \
  --wasm target/wasm32-unknown-unknown/release/deposit_vault.wasm \
  --source alice --network testnet

stellar contract upload \
  --wasm target/wasm32-unknown-unknown/release/deposit_handler.wasm \
  --source alice --network testnet

stellar contract upload \
  --wasm target/wasm32-unknown-unknown/release/withdrawal_vault.wasm \
  --source alice --network testnet

stellar contract upload \
  --wasm target/wasm32-unknown-unknown/release/withdrawal_handler.wasm \
  --source alice --network testnet

stellar contract upload \
  --wasm target/wasm32-unknown-unknown/release/order_vault.wasm \
  --source alice --network testnet

stellar contract upload \
  --wasm target/wasm32-unknown-unknown/release/order_handler.wasm \
  --source alice --network testnet

# Risk, periphery, router
stellar contract upload --wasm target/wasm32-unknown-unknown/release/liquidation_handler.wasm --source alice --network testnet
stellar contract upload --wasm target/wasm32-unknown-unknown/release/adl_handler.wasm          --source alice --network testnet
stellar contract upload --wasm target/wasm32-unknown-unknown/release/fee_handler.wasm          --source alice --network testnet
stellar contract upload --wasm target/wasm32-unknown-unknown/release/referral_storage.wasm    --source alice --network testnet
stellar contract upload --wasm target/wasm32-unknown-unknown/release/reader.wasm               --source alice --network testnet
stellar contract upload --wasm target/wasm32-unknown-unknown/release/exchange_router.wasm     --source alice --network testnet
```

### Step 3 — Deploy core contracts

```bash
# Role store
ROLE_STORE=$(stellar contract deploy \
  --wasm-hash <ROLE_STORE_WASM_HASH> \
  --source alice --network testnet \
  -- --admin <ALICE_ADDRESS>)

# Data store
DATA_STORE=$(stellar contract deploy \
  --wasm-hash <DATA_STORE_WASM_HASH> \
  --source alice --network testnet \
  -- --role_store $ROLE_STORE)

# Oracle
ORACLE=$(stellar contract deploy \
  --wasm-hash <ORACLE_WASM_HASH> \
  --source alice --network testnet \
  -- --admin <ALICE_ADDRESS> --role_store $ROLE_STORE --data_store $DATA_STORE)
```

### Step 4 — Deploy market infrastructure

```bash
MARKET_FACTORY=$(stellar contract deploy \
  --wasm-hash <MARKET_FACTORY_WASM_HASH> \
  --source alice --network testnet \
  -- --admin <ALICE_ADDRESS> \
     --role_store $ROLE_STORE \
     --data_store $DATA_STORE \
     --market_token_wasm_hash <MARKET_TOKEN_WASM_HASH>)
```

### Step 5 — Deploy vaults and handlers

```bash
# Deposit
DEPOSIT_VAULT=$(stellar contract deploy --wasm-hash <DEPOSIT_VAULT_WASM_HASH> \
  --source alice --network testnet -- --admin <ALICE_ADDRESS> --role_store $ROLE_STORE)

DEPOSIT_HANDLER=$(stellar contract deploy --wasm-hash <DEPOSIT_HANDLER_WASM_HASH> \
  --source alice --network testnet \
  -- --admin <ALICE_ADDRESS> --role_store $ROLE_STORE --data_store $DATA_STORE \
     --oracle $ORACLE --deposit_vault $DEPOSIT_VAULT)

# Withdrawal
WITHDRAWAL_VAULT=$(stellar contract deploy --wasm-hash <WITHDRAWAL_VAULT_WASM_HASH> \
  --source alice --network testnet -- --admin <ALICE_ADDRESS> --role_store $ROLE_STORE)

WITHDRAWAL_HANDLER=$(stellar contract deploy --wasm-hash <WITHDRAWAL_HANDLER_WASM_HASH> \
  --source alice --network testnet \
  -- --admin <ALICE_ADDRESS> --role_store $ROLE_STORE --data_store $DATA_STORE \
     --oracle $ORACLE --withdrawal_vault $WITHDRAWAL_VAULT)

# Orders
ORDER_VAULT=$(stellar contract deploy --wasm-hash <ORDER_VAULT_WASM_HASH> \
  --source alice --network testnet -- --admin <ALICE_ADDRESS> --role_store $ROLE_STORE)

ORDER_HANDLER=$(stellar contract deploy --wasm-hash <ORDER_HANDLER_WASM_HASH> \
  --source alice --network testnet \
  -- --admin <ALICE_ADDRESS> --role_store $ROLE_STORE --data_store $DATA_STORE \
     --oracle $ORACLE --order_vault $ORDER_VAULT)
```

### Step 6 — Deploy risk, periphery, and router

```bash
LIQUIDATION_HANDLER=$(stellar contract deploy --wasm-hash <LIQUIDATION_HANDLER_WASM_HASH> \
  --source alice --network testnet \
  -- --admin <ALICE_ADDRESS> --role_store $ROLE_STORE --data_store $DATA_STORE --oracle $ORACLE)

ADL_HANDLER=$(stellar contract deploy --wasm-hash <ADL_HANDLER_WASM_HASH> \
  --source alice --network testnet \
  -- --admin <ALICE_ADDRESS> --role_store $ROLE_STORE --data_store $DATA_STORE --oracle $ORACLE)

FEE_HANDLER=$(stellar contract deploy --wasm-hash <FEE_HANDLER_WASM_HASH> \
  --source alice --network testnet \
  -- --admin <ALICE_ADDRESS> --role_store $ROLE_STORE --data_store $DATA_STORE)

stellar contract deploy --wasm-hash <REFERRAL_STORAGE_WASM_HASH> \
  --source alice --network testnet -- --admin <ALICE_ADDRESS>

stellar contract deploy --wasm-hash <READER_WASM_HASH> --source alice --network testnet

EXCHANGE_ROUTER=$(stellar contract deploy --wasm-hash <EXCHANGE_ROUTER_WASM_HASH> \
  --source alice --network testnet \
  -- --admin <ALICE_ADDRESS> \
     --role_store $ROLE_STORE \
     --data_store $DATA_STORE \
     --deposit_handler $DEPOSIT_HANDLER \
     --withdrawal_handler $WITHDRAWAL_HANDLER \
     --order_handler $ORDER_HANDLER \
     --fee_handler $FEE_HANDLER)
```

### Step 7 — Grant roles

Each handler needs the `CONTROLLER` role to write to `data_store` and withdraw from market pools:

```bash
for CONTRACT in $DEPOSIT_HANDLER $WITHDRAWAL_HANDLER $ORDER_HANDLER \
                $LIQUIDATION_HANDLER $ADL_HANDLER $FEE_HANDLER $EXCHANGE_ROUTER; do
  stellar contract invoke --id $ROLE_STORE \
    --source alice --network testnet \
    -- grant_role --account $CONTRACT --role CONTROLLER
done
```

---

## Invoke Contracts (Examples)

### Create a market
```bash
stellar contract invoke --id $MARKET_FACTORY \
  --source alice --network testnet \
  -- create_market \
     --index_token <ETH_TOKEN_ADDRESS> \
     --long_token  <WETH_TOKEN_ADDRESS> \
     --short_token <USDC_TOKEN_ADDRESS>
```

### Read pool value
```bash
stellar contract invoke --id <READER_ADDRESS> \
  --source alice --network testnet \
  -- get_market_pool_value_info \
     --data_store $DATA_STORE \
     --oracle $ORACLE \
     --market_token <MARKET_TOKEN_ADDRESS> \
     --maximize false
```

### Create a market increase order (via exchange router)
```bash
stellar contract invoke --id $EXCHANGE_ROUTER \
  --source alice --network testnet \
  -- create_order \
     --market <MARKET_TOKEN_ADDRESS> \
     --receiver <ALICE_ADDRESS> \
     --initial_collateral_token <USDC_ADDRESS> \
     --size_delta_usd 1000000000000000000000000000000000 \
     --collateral_delta_amount 1000000000 \
     --trigger_price 0 \
     --acceptable_price 0 \
     --execution_fee 100000 \
     --min_output_amount 0 \
     --order_type MarketIncrease \
     --is_long true
```

---

## Project Structure

```
contracts/
├── Cargo.toml                    # workspace root
├── README.md                     # this file
│
├── contracts/
│   ├── data_store/               # Phase 1 — universal KV store
│   ├── role_store/               # Phase 1 — access control
│   ├── market_token/             # Phase 2 — SEP-41 LP token
│   ├── market_factory/           # Phase 2 — deterministic market deploy
│   ├── oracle/                   # Phase 3 — keeper-fed prices (ed25519)
│   ├── deposit_vault/            # Phase 4 — token custody for deposits
│   ├── deposit_handler/          # Phase 4 — deposit lifecycle
│   ├── withdrawal_vault/         # Phase 4 — LP custody for withdrawals
│   ├── withdrawal_handler/       # Phase 4 — withdrawal lifecycle
│   ├── order_vault/              # Phase 5 — collateral custody for orders
│   ├── order_handler/            # Phase 5 — full order lifecycle
│   ├── liquidation_handler/      # Phase 6 — force-close underwater positions
│   ├── adl_handler/              # Phase 6 — auto-deleverage profitable positions
│   ├── fee_handler/              # Phase 7 — fee distribution and claims
│   ├── referral_storage/         # Phase 7 — referral codes and tier rebates
│   ├── reader/                   # Phase 7 — stateless aggregate views
│   └── exchange_router/          # Phase 8 — user entry point, multicall
│
└── libs/
    ├── types/                    # shared #[contracttype] structs
    ├── math/                     # precision constants and safe math
    ├── keys/                     # sha256 key derivation (~58 functions)
    ├── market_utils/             # pool, OI, funding, borrowing math
    ├── position_utils/           # per-position PnL, fees, validation
    ├── pricing_utils/            # price impact, execution price
    ├── swap_utils/               # single and multi-hop swaps
    ├── increase_position_utils/  # position open/increase logic
    └── decrease_position_utils/  # position close/decrease logic
```

---

## EVM → Soroban Reference

| Solidity / EVM | Soroban / Rust |
|---|---|
| `bytes32` | `BytesN<32>` |
| `keccak256(abi.encode(...))` | `env.crypto().sha256(bytes)` |
| `mapping(bytes32 => uint256)` | `env.storage().persistent().set(key, val)` |
| `uint256` | `u128` (or `U256` for overflow-sensitive paths) |
| `int256` | `i128` (or `I256`) |
| `address` | `Address` |
| `block.timestamp` | `env.ledger().timestamp()` |
| `ERC-20` | SEP-41 via `soroban_sdk::token::Client` |
| `CREATE2` | `env.deployer().with_address(deployer, salt).deploy_v2(wasm, args)` |
| `emit Event(...)` | `env.events().publish((Symbol,), data)` |
| `msg.sender` | passed as `Address` arg + `caller.require_auth()` |
| `onlyRole` modifier | `role_store.has_role(caller, role)` cross-contract call |
| `ReentrancyGuard` | not needed — Soroban execution is atomic per transaction |

---

## Implementation Status

| Phase | Description | Status |
|---|---|---|
| 1 | Foundation — data_store, role_store, types, math, keys | ✅ Complete |
| 2 | Market infrastructure — market_token, market_factory, market_utils | ✅ Complete |
| 3 | Oracle — keeper-fed prices, ed25519 verification | ✅ Complete |
| 4 | Liquidity — deposit and withdrawal vaults + handlers | ✅ Complete |
| 5 | Trading — order vault, position utils, order handler | 🔧 Scaffolded |
| 6 | Risk — liquidation handler, ADL handler | 🔧 Scaffolded |
| 7 | Periphery — fee handler, referral storage, reader | 🔧 Scaffolded |
| 8 | Router — exchange router with multicall | 🔧 Scaffolded |

> **Scaffolded** means all function signatures, parameter types, and detailed implementation TODOs are in place. Logic bodies are the next step.

---

## License

MIT
