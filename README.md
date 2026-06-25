```
  ███████╗ ██████╗ ██╗  ██╗
  ██╔════╝██╔═══██╗██║  ██║
  ███████╗██║   ██║███████║
  ╚════██║██║   ██║╚════██║
  ███████║╚██████╔╝     ██║
  ╚══════╝ ╚═════╝      ╚═╝
           · m a r k e t ·

   Perpetuals Exchange on Stellar / Soroban
```

---

SO4.market is a decentralised perpetuals and spot exchange built on [Stellar](https://stellar.org) using [Soroban](https://soroban.stellar.org) smart contracts (SDK 25, Rust).

The protocol implements an isolated-market LP model with two-step keeper execution, dynamic funding rates, borrowing fees, price impact curves, auto-deleveraging, and on-chain liquidations — all adapted faithfully to Soroban's execution environment.

---

## Current Scope

This repository is the Soroban contracts package for the SO4.market GMX Synthetics port. The community issue campaign is over; treat `issues_v2.md` as a historical backlog, not as the source of live project truth.

### v1 contracts scope

| Area | v1 status |
|---|---|
| Isolated markets, LP mint/burn, deposits, withdrawals | In scope |
| Market, limit, stop, and swap order lifecycle | In scope |
| Liquidation, ADL execution, fee claiming, referral storage | In scope |
| Reader views and router multicall | In scope |
| Custom keeper-signed oracle prices | In scope |
| Testnet deployment, token bootstrap, and operator Make workflows | In scope |

### Deferred or non-v1 scope

| Area | Decision |
|---|---|
| Pyth VAA oracle ingestion | Deferred. v1 uses the custom keeper-signature oracle path. |
| Full production frontend and indexer | Deferred from this contracts repository. |
| On-chain governance/timelock | Deferred; production admin policy should use Stellar native multisig. |
| Broad contract mutability | Rejected for v1. Only contracts with actual Rust `upgrade` entrypoints are upgradeable. |

### Intentional GMX-to-Soroban deviations

| GMX Synthetics concept | SO4 Soroban implementation |
|---|---|
| EVM ERC-20 token interactions | SEP-41 token clients and Stellar Asset Contracts. |
| Global Solidity data store for most structs | Handler-local persistent storage for deposits, withdrawals, orders, and positions; `data_store` remains for config, pool accounting, lists, and shared values. |
| Oracle integration | Ledger-scoped keeper-submitted prices with ed25519 signature verification; test helpers are excluded from production builds. |
| Router execution shape | Soroban-friendly multicall with explicit token-send actions before request creation. |
| Execution fees | Contract support is present where implemented, but operator economics and production keeper compensation still need final deployment policy. |

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

Handlers read/write ──► DataStore      (market config, pool accounting, lists)
Requests/positions   ──► local storage (handler-owned persistent records)
All handlers price via  ──► Oracle     (keeper-fed min/max price pairs)
All handlers check      ──► RoleStore  (role-based access control)

MarketFactory  ──► deploy MarketToken (SEP-41 LP) + register in DataStore
Reader         ──► stateless views over DataStore (no writes)
```

### Contract Map

| Contract | Description |
|---|---|
| `data_store` | Universal typed key-value store for market metadata, risk config, pool accounting, role-gated lists, and fee/accounting values. User requests and positions are stored locally in their handlers. |
| `role_store` | Role-based access control — CONTROLLER, MARKET_KEEPER, ORDER_KEEPER, etc. |
| `oracle` | Keeper-fed price store. Prices expire per ledger. Ed25519-verified. (Mainnet uses cryptographically signed prices only; test-only paths are excluded from production builds.) |
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
| `reader` | Read-only aggregate views: positions, markets, OI, funding, liquidation checks. Stores only upgrade admin metadata. |
| `exchange_router` | Single user entry point. Supports multicall for atomic multi-step actions. |

### Shared Libraries

| Crate | Description |
|---|---|
| `libs/types` | All shared structs: `MarketProps`, `PositionProps`, `OrderProps`, `PriceProps`, `PositionFees`, etc. |
| `libs/math` | `FLOAT_PRECISION` (10³⁰), `TOKEN_PRECISION` (10⁷), `mul_div_wide` (I256), `pow_factor`, `sqrt_fp`. |
| `libs/keys` | ~58 deterministic `sha256`-based key derivation functions. |
| `libs/market_utils` | Pool value, open interest, PnL, funding state, borrowing fees, pool/OI validation. |
| `libs/position_utils` | Per-position PnL, fee breakdown, funding settlement, leverage validation, liquidation check. |
| `libs/pricing_utils` | Swap and position price impact, execution price, impact pool management. |
| `libs/swap_utils` | Single-hop and multi-hop token swaps through market pools. |
| `libs/increase_position_utils` | Open or increase a long/short position (14-step flow). |
| `libs/decrease_position_utils` | Partial or full position close with PnL settlement (14-step flow). Accepts an optional `swap_path` — when non-empty, collateral output is swapped through the specified markets before reaching the receiver. |

---

## Key Financial Mechanics

### Price Precision
- All USD values use `FLOAT_PRECISION = 10^30`.
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

## Order Collateral Flow

> **Resolves issue #47 — Unify order collateral transfer model.**

The protocol follows a single, canonical two-step collateral path.
`exchange_router` is the only entry point that may touch a caller's tokens.
`order_handler` is a passive consumer that only reads from the vault.

### Chosen model: Router-push, Handler-snapshot

```
User
 │
 │  multicall([
 │    SendTokens { token, receiver: order_vault, amount },  ← Step 1
 │    CreateOrder { params },                               ← Step 2
 │  ])
 ▼
ExchangeRouter
 │
 ├─ Step 1: token::Client(token).transfer(caller → order_vault, amount)
 │
 └─ Step 2: OrderHandlerClient.create_order(caller, params)
               │
               └─ OrderVault.record_transfer_in(token)
                    │  snapshot = on_chain_balance − last_recorded
                    │  REVERT if snapshot ≤ 0  (ZeroCollateral)
                    └─ stores order with collateral_delta_amount = snapshot
```

**Why this model:**
- The router holds the user's token approval; keeping the pull there means
  handlers never need their own approval and cannot silently double-pull.
- `record_transfer_in` acts as a snapshot oracle: it always reflects exactly
  what arrived since the last order, so double-submission is impossible.
- Decrease / stop-loss / liquidation orders do **not** deposit collateral;
  `create_order` skips the vault snapshot for those order types.

**Balance invariant:** after every `create_order` or `transfer_out`, the vault's
DataStore recorded balance equals its actual on-chain SEP-41 balance.

---

## Keeper Trust Model and Execution Timing

> **Resolves issue #154/#125 — Document keeper execution timing and front-running design.**

### What keepers can do

Keepers are permissioned off-chain bots that hold role-store roles (`ORDER_KEEPER`,
`LIQUIDATION_KEEPER`, `ADL_KEEPER`). They observe pending requests and choose **when** to
submit the execution transaction. This gives a keeper limited influence over two variables:

1. **Oracle prices** — The keeper submits `set_prices` and `execute_order` in the same
   transaction. The oracle prices therefore reflect the market at the ledger the keeper
   chose, not the ledger the user chose.
2. **Execution timing** — A keeper can delay execution, waiting for a price movement that
   benefits them or harms the user, within the bounds of the acceptable-price window.

### What the protocol does to limit the damage

| Protection | Mechanism |
|---|---|
| **Acceptable price window** | Every `MarketIncrease` / `MarketDecrease` / swap order carries `acceptable_price`. `execute_order` reverts if the oracle execution price is worse than this bound. A tight window forces the keeper to execute at a fair price or not at all. |
| **Role gating** | Only accounts holding the `ORDER_KEEPER` role (assigned by admin) can call `execute_order`. Rogue keeper candidates must first compromise the admin key or role-store. |
| **Atomic price + execution** | Prices are set in the same transaction as execution. The keeper cannot front-run itself by setting prices in an earlier ledger. |
| **Price impact** | Large positions that worsens OI balance incur negative price impact, reducing the keeper's incentive to push oversized trades through. |
| **Per-market OI caps** | `MAX_OPEN_INTEREST` per market per side prevents a compromised keeper from accumulating unbounded OI that could drain the pool. |

### Known risks and latency gap

Soroban ledgers close approximately every **5–6 seconds** on the live network. The
expected keeper round-trip (observe → sign → submit → confirm) is **1–3 ledgers**. This
means a keeper may observe a price `P` at ledger `L` but execute at ledger `L+2`, when
the true price is `P′ ≠ P`.

The `acceptable_price` window is the primary mitigation. Its width is user-controlled:
a tight window (e.g. 0.1 % slippage) limits the price deviation a keeper can exploit
to that band. A user who sets `acceptable_price = 0` (no check) accepts unbounded
timing risk.

**Known residual risk:** A keeper that controls block inclusion (e.g. a validator that is
also a keeper) could theoretically execute at the worst price within the acceptable window
on every trade. The protocol does not defend against this at the smart-contract level.
Mitigation strategies — such as multiple competing keepers and keeper-reputation staking —
are operational concerns outside the current scope.

### Acceptable price guidance for order creators

| Order type | Recommended `acceptable_price` |
|---|---|
| `MarketIncrease` (long) | oracle mid-price × (1 + max_slippage) |
| `MarketIncrease` (short) | oracle mid-price × (1 − max_slippage) |
| `MarketDecrease` (long) | oracle mid-price × (1 − max_slippage) |
| `MarketDecrease` (short) | oracle mid-price × (1 + max_slippage) |
| `LimitIncrease` / `LimitDecrease` | set to trigger price (order is only executed at or better than trigger) |
| Swap orders | min output amount derived from worst-case impact + slippage |

A `max_slippage` of 0.5 %–1 % covers normal 1–3 ledger delay at typical volatility
while protecting against malicious timing within the window.

---

## Canonical Storage Model

> **Resolves issue #2 — Decide the canonical storage model for requests and positions.**

All transient request types (Deposits, Withdrawals, Orders) and long-lived Positions live in **handler-local persistent storage** within their respective contracts rather than in the shared global `data_store`.

### Chosen Architecture: Local Persistent Storage

```
DepositHandler      ──► (Local persistent storage: DepositProps)
WithdrawalHandler   ──► (Local persistent storage: WithdrawalProps)
OrderHandler        ──► (Local persistent storage: OrderProps & PositionProps)
```

- **Deposits**: Persisted locally in `deposit_handler` using `LocalKey::Deposit(nonce)`.
- **Withdrawals**: Persisted locally in `withdrawal_handler` using `LocalKey::Withdrawal(nonce)`.
- **Orders**: Persisted locally in `order_handler` using `OrderStorageKey::Order(nonce)`.
- **Positions**: Persisted locally in `order_handler` using `PositionStorageKey::Position(key)`.

### Rationale

1. **Storage Rent (TTL) Isolation**: Soroban requires rent (TTL) for persistent storage. Distributing user-specific transient requests (deposits, withdrawals, orders) and positions to their local handler contracts isolates their TTL management. This prevents the shared `data_store` from becoming an eviction risk or billing bottleneck.
2. **Access Control & Encapsulation**: Storing positions and orders within `order_handler` ensures that only authorized logic in `order_handler` (e.g. `create_order`, `execute_order`) can mutate position state. If stored in a shared `data_store`, any contract with write access to the `data_store` could corrupt position records.
3. **CPU Instruction / Serialization Savings**: Centralized databases like `data_store` store values in generic key-value maps. Cross-contract struct serialization/deserialization into `data_store` introduces significant CPU instruction overhead. Storing structs locally allows direct, type-safe serialization within the contract's namespace.

---

## Multi-hop Swap Semantics

> **Resolves issue #57 — Audit multi-hop swap token movement semantics.**

### Physical token movement model

Tokens move **physically** between pool contracts on every hop.
No virtual accounting shortcut is used — each intermediate transfer is
a real SEP-41 `transfer` call on-chain.

For a two-hop path `A → B → C` via `[market_1, market_2]`:

```
                 order_vault
                      │  transfer_out(token_A → market_1)
                      ▼
          ┌─── market_1 (pool: A, B) ───┐
          │  pool_A += input_A          │
          │  pool_B -= output_B         │
          │  SEP-41 transfer:           │
          │  token_B → market_2  ───────┼────────────────────┐
          └─────────────────────────────┘                    ▼
                                             ┌─── market_2 (pool: B, C) ───┐
                                             │  pool_B += output_B         │
                                             │  pool_C -= output_C         │
                                             │  SEP-41 transfer:           │
                                             │  token_C → receiver  ───────┼──► User
                                             └─────────────────────────────┘
```

**Pool balance invariant (holds after every multi-hop execution):**

| pool          | token   | DataStore record             | on-chain balance |
|---------------|---------|------------------------------|-----------------|
| market_1      | token_A | + input_A                    | + input_A       |
| market_1      | token_B | − output_B                   | − output_B      |
| market_2      | token_B | + output_B                   | + output_B      |
| market_2      | token_C | − output_C                   | − output_C      |

### Duplicate market guard

A swap path with a repeated market address causes double-mutation of pool
state and corrupts price-impact accounting.  `swap_with_path` rejects any
path with duplicate market addresses before any state is touched.

---

## Prerequisites

### 1. Rust toolchain

```bash
# Install Rust (if not already installed)
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh

# Add the wasm target (required for Soroban contract compilation)
rustup target add wasm32-unknown-unknown
```

### 2. Stellar CLI

```bash
# Install from crates.io with wasm optimiser enabled
cargo install --locked stellar-cli --features opt

# Verify
stellar --version
```

### 3. Docker (optional — required for local node)

[Install Docker Desktop](https://docs.docker.com/get-docker/) if you want to run a fully local Stellar node instead of using testnet.

---

## Keys & Identity

Every transaction on Stellar must be signed by a key pair. The CLI manages keys in a local keystore (`~/.config/stellar/identity/`).

### Generate a new key pair

```bash
# Generate and store a key named "alice" globally (persists across projects)
stellar keys generate --global alice --network testnet

# Print the public address for "alice"
stellar keys address alice

# Print the secret key (keep this safe — do not commit it)
stellar keys show alice
```

### Import an existing secret key

```bash
stellar keys add alice --secret-key
# You will be prompted to paste the secret key (starts with S...)
```

### List all stored keys

```bash
stellar keys ls
```

### Fund a key on testnet (Friendbot airdrop)

```bash
# Requests 10,000 XLM from the testnet Friendbot faucet
stellar keys fund alice --network testnet

# Verify the balance
stellar contract invoke \
  --id CDLZFC3SYJYDZT7K67VZ75HPJVIEUVNIXF47ZG2FB2RMQQVU2HHGCYSC \
  --source alice --network testnet \
  -- balance --id $(stellar keys address alice)
```

### Configure the network shorthand (optional)

```bash
# "testnet" is pre-configured. To add a custom RPC:
stellar network add localnet \
  --rpc-url http://localhost:8000/soroban/rpc \
  --network-passphrase "Standalone Network ; February 2017"
```

---

## Build

### Type-check only (fastest — no wasm output)

```bash
cargo check --workspace
```

### Lint with Clippy

```bash
cargo clippy --workspace -- -D warnings
```

### Build all contracts to wasm

```bash
stellar contract build
```

Output: `target/wasm32-unknown-unknown/release/<contract_name>.wasm`

> The `stellar contract build` command compiles every `cdylib` crate in the workspace automatically.

### Build a single contract

```bash
stellar contract build --package data-store
stellar contract build --package role-store
stellar contract build --package oracle
stellar contract build --package market-factory
stellar contract build --package deposit-handler
stellar contract build --package withdrawal-handler
stellar contract build --package order-handler
stellar contract build --package liquidation-handler
stellar contract build --package adl-handler
stellar contract build --package fee-handler
stellar contract build --package referral-storage
stellar contract build --package reader
stellar contract build --package exchange-router
```

### Optimised release build

```bash
# Adds wasm-opt shrinking pass — use this before uploading to mainnet
stellar contract build --release
```

### Inspect a compiled wasm

```bash
# Print all exported function names
stellar contract inspect \
  --wasm target/wasm32-unknown-unknown/release/order_handler.wasm
```

---

## Test

All tests run inside the Soroban sandbox (no network required). The SDK provides a full mock host environment with storage, auth, and events.

### Run the full test suite

```bash
cargo test --workspace
```

### Test a specific crate

```bash
# Shared libraries
cargo test -p gmx-math
cargo test -p gmx-keys
cargo test -p gmx-market-utils
cargo test -p gmx-position-utils
cargo test -p gmx-pricing-utils
cargo test -p gmx-swap-utils
cargo test -p gmx-decrease-position-utils

# Core contracts
cargo test -p data-store
cargo test -p role-store
cargo test -p oracle
cargo test -p fee-handler

# Handler contracts
cargo test -p deposit-handler
cargo test -p withdrawal-handler
cargo test -p order-handler
```

### Run a single test by name

```bash
cargo test -p gmx-market-utils apply_delta_to_pool_amount_works
cargo test -p oracle set_and_get_price
cargo test -p deposit-handler create_and_execute_deposit
```

### Show test output (disable output capture)

```bash
cargo test --workspace -- --nocapture
```

### Run tests with a filter pattern

```bash
# All tests whose name contains "deposit"
cargo test --workspace deposit
```

### Check test coverage (requires cargo-llvm-cov)

```bash
cargo install cargo-llvm-cov
cargo llvm-cov --workspace --open
```

---

## Local Node

For end-to-end integration testing without using public testnet.

### Start a local Stellar node

```bash
stellar network start local
```

This starts a Docker container with a local Stellar + Soroban node on `http://localhost:8000`.

### Generate and fund a local key

```bash
stellar keys generate --global dev --network local
stellar keys fund dev --network local
```

### Stop the local node

```bash
stellar network stop local
```

All state is ephemeral — restarting clears everything.

---

## Makefile

A `Makefile` is provided for the most common workflows.

| Target | Description |
|---|---|
| `make build` | Compile all contracts to wasm via `stellar contract build` |
| `make check` | Type-check without producing wasm (`cargo check`) |
| `make lint` | Run Clippy with warnings as errors |
| `make test` | Run the full Soroban sandbox test suite |
| `make testnet-smoke` | Create and execute a small testnet deposit and verify GM tokens were minted |
| `make deploy-all` | Deploy the full protocol graph to **testnet** (default) |
| `make deploy-contract` | Deploy one standalone contract Wasm for debugging |
| `make upgrade-contract` | Upload new Wasm and upgrade one existing deployed contract |
| `make upgrade-all` | Upgrade every deployed protocol contract listed in `UPGRADE_CONTRACTS` |
| `make deploy-mainnet` | Deploy the full protocol graph to **mainnet** |
| `make clean` | Remove `target/` and `.deployed/` |

### Deploy and upgrade

```bash
# Full testnet deployment (default network)
make deploy-all

# Testnet with a different key name
make deploy-all SOURCE=mykey

# Mainnet
make deploy-mainnet SOURCE=mykey

# Override both
make deploy-all NETWORK=mainnet SOURCE=mykey

# Deploy one standalone contract Wasm for debugging
make deploy-contract CONTRACT=reader NETWORK=testnet SOURCE=mykey

# Upgrade one deployed contract in-place
make upgrade-contract CONTRACT=deposit_handler NETWORK=testnet SOURCE=mykey

# Upgrade every deployed protocol contract listed in UPGRADE_CONTRACTS
make upgrade-all NETWORK=testnet SOURCE=mykey
```

`SOURCE` must match a key stored in your local Stellar keystore (see [Keys & Identity](#keys--identity) above).

The full deploy script (`scripts/deploy.sh`) handles the full sequence automatically:
builds → uploads wasm blobs → deploys each contract → calls `initialize` → grants `CONTROLLER` roles → prints a summary table → saves all addresses to `.deployed/<NETWORK>.env`.

If `.deployed/<NETWORK>.env` already exists, `make deploy-all` refuses to create
a second protocol graph and prints the appropriate upgrade commands. To
intentionally create a fresh deployment, use:

```bash
make deploy-force NETWORK=testnet SOURCE=mykey
```

`deploy-contract` is deliberately standalone: it deploys one Wasm and prints the
new contract address, but it does not update `.deployed/<NETWORK>.env` or wire
the contract into the current protocol deployment.

Upgrade commands require the deployed contract to already expose an admin-gated
`upgrade(env, new_wasm_hash)` function that calls
`env.deployer().update_current_contract_wasm(new_wasm_hash)`.

### Deployed address file

After a successful deploy, addresses are written to `.deployed/testnet.env` (or `mainnet.env`):

```bash
# Source the file to load all contract addresses into your shell
source .deployed/testnet.env
echo $EXCHANGE_ROUTER
```

---

## Testnet Market Bootstrap

For the current end-to-end operator procedure, including signed-oracle setup,
exact initialization arguments, and the deposit smoke test, follow
[`docs/deployment.md`](docs/deployment.md). The commands below are a shorter
reference and may omit production oracle details.

After the protocol contracts are deployed, you need to create a market, grant keeper roles, set config parameters, and seed initial liquidity before the protocol is usable. `scripts/bootstrap.sh` automates all of these steps.

### Quick start (end-to-end fresh testnet deployment)

```bash
# 1. Generate and fund keys
stellar keys generate --global alice  --network testnet
stellar keys generate --global keeper --network testnet
stellar keys fund alice  --network testnet
stellar keys fund keeper --network testnet

# 2. Create and fund test tokens (TWBTC = long, TUSDC = short)
make market-tokens NETWORK=testnet SOURCE=alice LONG_CODE=TWBTC SHORT_CODE=TUSDC

# 3. Deploy all protocol contracts
make deploy-all NETWORK=testnet SOURCE=alice

# 4. Bootstrap: grant roles, create market, set config keys
make bootstrap NETWORK=testnet SOURCE=alice KEEPER=keeper LONG_CODE=TWBTC SHORT_CODE=TUSDC

# 5. Submit initial oracle prices
bash scripts/submit_prices.sh testnet keeper

# 6. Seed the market with initial liquidity
#    (see output of make bootstrap for the exact deposit_handler invocation)
```

### Bootstrap targets

| Target | Description |
|---|---|
| `make market-tokens` | Create and fund both TWBTC and TUSDC test tokens |
| `make bootstrap` | Full post-deploy bootstrap (roles + market + config + seed instructions) |
| `make market-init` | Market creation and config only (skip role grants and seed) |
| `make seed-liquidity` | Print instructions for seeding the pool with initial liquidity |

### Environment variables

| Variable | Default | Description |
|---|---|---|
| `KEEPER` | `$(SOURCE)` | Stellar key name for the keeper account |
| `LONG_CODE` | `TWBTC` | Ticker of the long token |
| `SHORT_CODE` | `TUSDC` | Ticker of the short token |
| `SEED_LONG` | `10000000` | Long token amount for initial liquidity seed |
| `SEED_SHORT` | `10000000` | Short token amount for initial liquidity seed |
| `SKIP_ROLES` | `0` | Set to `1` to skip role grants (idempotent re-run) |
| `SKIP_MARKET` | `0` | Set to `1` to skip market creation (already created) |
| `SKIP_CONFIG` | `0` | Set to `1` to skip config key writes |
| `SKIP_SEED` | `0` | Set to `1` to skip liquidity seed instructions |

### What the bootstrap script does

1. **Grant keeper roles** — grants `MARKET_KEEPER`, `ORDER_KEEPER`, `LIQUIDATION_KEEPER`, `ADL_KEEPER`, and `FEE_KEEPER` roles to the keeper account in `role_store`.
2. **Create market** — calls `market_factory.create_market(index_token, long_token, short_token)` and saves the new `MARKET_TOKEN` address to `.deployed/<NETWORK>.env`.
3. **Set config keys** — writes per-market config parameters (`max_pool_amount`, `min_collateral_factor`, `max_leverage`, fee factors, borrowing factors, funding factors) to `data_store`.
4. **Seed instructions** — prints the manual steps for seeding initial liquidity, since the oracle must be running first.

### Repeatable and idempotent

The bootstrap can be re-run safely with `SKIP_*` flags for the steps already completed:

```bash
# Re-run only config key updates for an existing market
make market-init NETWORK=testnet SOURCE=alice SKIP_ROLES=1 SKIP_SEED=1

# Re-run only role grants
make bootstrap NETWORK=testnet SOURCE=alice SKIP_MARKET=1 SKIP_CONFIG=1 SKIP_SEED=1
```

---

## Deploy to Testnet (manual)

The steps below are the manual equivalent of `make deploy`, useful for debugging individual steps or partial re-deploys.

Contracts must be deployed in dependency order: stores first, then handlers that depend on them, then the router last. The sequence below captures the full stack.

### Step 1 — Build wasm blobs

```bash
stellar contract build
```

### Step 2 — Upload wasm blobs

Each `upload` uploads bytecode and returns a `WASM_HASH`. Record each one — the hash is stable as long as the code doesn't change, so you only need to re-upload after rebuilding.

```bash
ROLE_STORE_HASH=$(stellar contract upload \
  --wasm target/wasm32-unknown-unknown/release/role_store.wasm \
  --source alice --network testnet)

DATA_STORE_HASH=$(stellar contract upload \
  --wasm target/wasm32-unknown-unknown/release/data_store.wasm \
  --source alice --network testnet)

ORACLE_HASH=$(stellar contract upload \
  --wasm target/wasm32-unknown-unknown/release/oracle.wasm \
  --source alice --network testnet)

MARKET_TOKEN_HASH=$(stellar contract upload \
  --wasm target/wasm32-unknown-unknown/release/market_token.wasm \
  --source alice --network testnet)

MARKET_FACTORY_HASH=$(stellar contract upload \
  --wasm target/wasm32-unknown-unknown/release/market_factory.wasm \
  --source alice --network testnet)

DEPOSIT_VAULT_HASH=$(stellar contract upload \
  --wasm target/wasm32-unknown-unknown/release/deposit_vault.wasm \
  --source alice --network testnet)

DEPOSIT_HANDLER_HASH=$(stellar contract upload \
  --wasm target/wasm32-unknown-unknown/release/deposit_handler.wasm \
  --source alice --network testnet)

WITHDRAWAL_VAULT_HASH=$(stellar contract upload \
  --wasm target/wasm32-unknown-unknown/release/withdrawal_vault.wasm \
  --source alice --network testnet)

WITHDRAWAL_HANDLER_HASH=$(stellar contract upload \
  --wasm target/wasm32-unknown-unknown/release/withdrawal_handler.wasm \
  --source alice --network testnet)

ORDER_VAULT_HASH=$(stellar contract upload \
  --wasm target/wasm32-unknown-unknown/release/order_vault.wasm \
  --source alice --network testnet)

ORDER_HANDLER_HASH=$(stellar contract upload \
  --wasm target/wasm32-unknown-unknown/release/order_handler.wasm \
  --source alice --network testnet)

LIQUIDATION_HANDLER_HASH=$(stellar contract upload \
  --wasm target/wasm32-unknown-unknown/release/liquidation_handler.wasm \
  --source alice --network testnet)

ADL_HANDLER_HASH=$(stellar contract upload \
  --wasm target/wasm32-unknown-unknown/release/adl_handler.wasm \
  --source alice --network testnet)

FEE_HANDLER_HASH=$(stellar contract upload \
  --wasm target/wasm32-unknown-unknown/release/fee_handler.wasm \
  --source alice --network testnet)

REFERRAL_STORAGE_HASH=$(stellar contract upload \
  --wasm target/wasm32-unknown-unknown/release/referral_storage.wasm \
  --source alice --network testnet)

READER_HASH=$(stellar contract upload \
  --wasm target/wasm32-unknown-unknown/release/reader.wasm \
  --source alice --network testnet)

EXCHANGE_ROUTER_HASH=$(stellar contract upload \
  --wasm target/wasm32-unknown-unknown/release/exchange_router.wasm \
  --source alice --network testnet)
```

### Step 3 — Capture your admin address

```bash
ALICE=$(stellar keys address alice)
```

### Step 4 — Deploy core infrastructure

```bash
ROLE_STORE=$(stellar contract deploy \
  --wasm-hash $ROLE_STORE_HASH \
  --source alice --network testnet \
  -- initialize --admin $ALICE)

DATA_STORE=$(stellar contract deploy \
  --wasm-hash $DATA_STORE_HASH \
  --source alice --network testnet \
  -- initialize --admin $ALICE --role_store $ROLE_STORE)

ORACLE=$(stellar contract deploy \
  --wasm-hash $ORACLE_HASH \
  --source alice --network testnet \
  -- initialize \
     --admin $ALICE \
     --role_store $ROLE_STORE \
     --data_store $DATA_STORE \
     --network_passphrase "Test SDF Network ; September 2015")
```

### Step 5 — Deploy market factory

```bash
MARKET_FACTORY=$(stellar contract deploy \
  --wasm-hash $MARKET_FACTORY_HASH \
  --source alice --network testnet \
  -- initialize \
     --admin $ALICE \
     --role_store $ROLE_STORE \
     --data_store $DATA_STORE)

stellar contract invoke --id $MARKET_FACTORY \
  --source alice --network testnet \
  -- set_market_token_wasm_hash \
     --caller $ALICE \
     --wasm_hash $MARKET_TOKEN_HASH
```

### Step 6 — Deploy vaults and handlers

```bash
DEPOSIT_VAULT=$(stellar contract deploy \
  --wasm-hash $DEPOSIT_VAULT_HASH \
  --source alice --network testnet \
  -- initialize --admin $ALICE --role_store $ROLE_STORE)

DEPOSIT_HANDLER=$(stellar contract deploy \
  --wasm-hash $DEPOSIT_HANDLER_HASH \
  --source alice --network testnet \
  -- initialize \
     --admin $ALICE \
     --role_store $ROLE_STORE \
     --data_store $DATA_STORE \
     --oracle $ORACLE \
     --deposit_vault $DEPOSIT_VAULT)

WITHDRAWAL_VAULT=$(stellar contract deploy \
  --wasm-hash $WITHDRAWAL_VAULT_HASH \
  --source alice --network testnet \
  -- initialize --admin $ALICE --role_store $ROLE_STORE)

WITHDRAWAL_HANDLER=$(stellar contract deploy \
  --wasm-hash $WITHDRAWAL_HANDLER_HASH \
  --source alice --network testnet \
  -- initialize \
     --admin $ALICE \
     --role_store $ROLE_STORE \
     --data_store $DATA_STORE \
     --oracle $ORACLE \
     --withdrawal_vault $WITHDRAWAL_VAULT)

ORDER_VAULT=$(stellar contract deploy \
  --wasm-hash $ORDER_VAULT_HASH \
  --source alice --network testnet \
  -- initialize --admin $ALICE --role_store $ROLE_STORE)

ORDER_HANDLER=$(stellar contract deploy \
  --wasm-hash $ORDER_HANDLER_HASH \
  --source alice --network testnet \
  -- initialize \
     --admin $ALICE \
     --role_store $ROLE_STORE \
     --data_store $DATA_STORE \
     --oracle $ORACLE \
     --order_vault $ORDER_VAULT)
```

### Step 7 — Deploy risk handlers and periphery

```bash
LIQUIDATION_HANDLER=$(stellar contract deploy \
  --wasm-hash $LIQUIDATION_HANDLER_HASH \
  --source alice --network testnet \
  -- initialize \
     --admin $ALICE \
     --role_store $ROLE_STORE \
     --data_store $DATA_STORE \
     --oracle $ORACLE \
     --order_handler $ORDER_HANDLER)

ADL_HANDLER=$(stellar contract deploy \
  --wasm-hash $ADL_HANDLER_HASH \
  --source alice --network testnet \
  -- initialize \
     --admin $ALICE \
     --role_store $ROLE_STORE \
     --data_store $DATA_STORE \
     --oracle $ORACLE \
     --order_handler $ORDER_HANDLER)

FEE_HANDLER=$(stellar contract deploy \
  --wasm-hash $FEE_HANDLER_HASH \
  --source alice --network testnet \
  -- initialize \
     --admin $ALICE \
     --role_store $ROLE_STORE \
     --data_store $DATA_STORE)

REFERRAL_STORAGE=$(stellar contract deploy \
  --wasm-hash $REFERRAL_STORAGE_HASH \
  --source alice --network testnet \
  -- initialize --admin $ALICE)

READER=$(stellar contract deploy \
  --wasm-hash $READER_HASH \
  --source alice --network testnet \
  -- initialize --admin $ALICE)
```

### Step 8 — Deploy exchange router

```bash
EXCHANGE_ROUTER=$(stellar contract deploy \
  --wasm-hash $EXCHANGE_ROUTER_HASH \
  --source alice --network testnet \
  -- initialize \
     --admin $ALICE \
     --role_store $ROLE_STORE \
     --data_store $DATA_STORE \
     --deposit_handler $DEPOSIT_HANDLER \
     --withdrawal_handler $WITHDRAWAL_HANDLER \
     --order_handler $ORDER_HANDLER \
     --fee_handler $FEE_HANDLER)
```

### Step 9 — Grant CONTROLLER role to all handlers

Handlers need `CONTROLLER` to write to `data_store` and withdraw from market pools:

```bash
for CONTRACT in \
  $DEPOSIT_HANDLER \
  $WITHDRAWAL_HANDLER \
  $ORDER_HANDLER \
  $LIQUIDATION_HANDLER \
  $ADL_HANDLER \
  $FEE_HANDLER \
  $EXCHANGE_ROUTER
do
  stellar contract invoke \
    --id $ROLE_STORE \
    --source alice --network testnet \
    -- grant_role --account $CONTRACT --role CONTROLLER
done
```

### Step 10 — Grant keeper roles (optional, for a test keeper account)

```bash
# Generate a dedicated keeper key
stellar keys generate --global keeper --network testnet
stellar keys fund keeper --network testnet
KEEPER=$(stellar keys address keeper)

# Market keeper (price feeds, execute deposits/withdrawals)
stellar contract invoke --id $ROLE_STORE --source alice --network testnet \
  -- grant_role --account $KEEPER --role MARKET_KEEPER

# Order keeper (execute pending orders)
stellar contract invoke --id $ROLE_STORE --source alice --network testnet \
  -- grant_role --account $KEEPER --role ORDER_KEEPER

# Liquidation keeper
stellar contract invoke --id $ROLE_STORE --source alice --network testnet \
  -- grant_role --account $KEEPER --role LIQUIDATION_KEEPER

# ADL keeper
stellar contract invoke --id $ROLE_STORE --source alice --network testnet \
  -- grant_role --account $KEEPER --role ADL_KEEPER

# Fee keeper (sweep protocol fees)
stellar contract invoke --id $ROLE_STORE --source alice --network testnet \
  -- grant_role --account $KEEPER --role FEE_KEEPER
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

### Open a long position via exchange router
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
│   ├── data_store/               # universal KV store
│   ├── role_store/               # access control
│   ├── market_token/             # SEP-41 LP token
│   ├── market_factory/           # deterministic market deploy
│   ├── oracle/                   # keeper-fed prices (ed25519)
│   ├── deposit_vault/            # token custody for deposits
│   ├── deposit_handler/          # deposit lifecycle
│   ├── withdrawal_vault/         # LP custody for withdrawals
│   ├── withdrawal_handler/       # withdrawal lifecycle
│   ├── order_vault/              # collateral custody for orders
│   ├── order_handler/            # full order lifecycle
│   ├── liquidation_handler/      # force-close underwater positions
│   ├── adl_handler/              # auto-deleverage profitable positions
│   ├── fee_handler/              # fee distribution and claims
│   ├── referral_storage/         # referral codes and tier rebates
│   ├── reader/                   # stateless aggregate views
│   └── exchange_router/          # user entry point, multicall
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

### Quarantine Candidates

The following directories currently sit under `libs/` but are **not** members of
the Cargo workspace and are written against MultiversX APIs, not Soroban:

| Directory | Status |
|---|---|
| `libs/deposit_flow` | Legacy/non-Soroban issue artifact. |
| `libs/withdrawal_flow` | Legacy/non-Soroban issue artifact. |
| `libs/position_list` | Legacy/non-Soroban issue artifact. |
| `libs/storage_ttl` | Legacy/non-Soroban issue artifact. |

Do not use these as implementation references for Soroban contracts. Either port
the useful ideas into the active Soroban crates or move them to an archive in a
dedicated cleanup PR.

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
| 5 | Trading — order vault, position utils, order handler | ✅ Complete |
| 6 | Risk — liquidation handler, ADL handler | ✅ Complete |
| 7 | Periphery — fee handler, referral storage, reader | ✅ Complete |
| 8 | Router — exchange router with multicall | ✅ Complete |
| 9 | Test hardening — fee/PnL/output on partial+full close, frozen-order state machine, collateral-output swap path | ✅ Complete |

---

## Upgrade Policy

Every core contract designed to be upgradeable exposes an `upgrade(env, new_wasm_hash)` function that authenticates the stored admin before calling `env.deployer().update_current_contract_wasm(new_wasm_hash)`.

> [!NOTE]
> **Audit Finding (Code-vs-Doc Disconnect):**
> The operator workflows can upload Wasm for any contract, but only contracts with a Rust `upgrade` entrypoint can be upgraded in place. Core databases and vaults remain functionally immutable, which significantly reduces the key-compromise attack surface for the protocol.

| Contract | Upgradeable (in Rust Code) | Upgrade Authority | Notes |
|---|---|---|---|
| `data_store` | ❌ Immutable | — | Stores all protocol state. Immutability protects against raw data tampering. |
| `role_store` | ❌ Immutable | — | Access control registry. Immutability secures access credentials. |
| `market_factory` | ❌ Immutable | — | Deploys LP tokens. Cannot be upgraded. |
| `market_token` | ❌ Immutable | — | Standard SEP-41 LP token. Always immutable. |
| `oracle` | ❌ Immutable | — | Price feed consumer. Fully immutable. |
| `deposit_vault` | ❌ Immutable | — | Token custodian. Fully immutable. |
| `withdrawal_vault`| ❌ Immutable | — | LP token custodian. Fully immutable. |
| `order_vault` | ❌ Immutable | — | Collateral custodian. Fully immutable. |
| `deposit_handler` | ✅ Yes | local `admin` address | Custody handling and LP mint execution. |
| `withdrawal_handler`| ❌ Immutable | — | Fully immutable. |
| `order_handler` | ✅ Yes | local `admin` address | Trading engine (orders, positions, swaps). |
| `liquidation_handler`| ✅ Yes | local `admin` address | Triggers forced position closing. |
| `adl_handler` | ❌ Immutable | — | Fully immutable. |
| `fee_handler` | ✅ Yes | local `admin` address | Fee claiming and UI-fee config. |
| `referral_storage` | ✅ Yes | local `admin` address | Referral code and tier storage. |
| `reader` | ✅ Yes | local `admin` address | View aggregation API. |
| `exchange_router` | ✅ Yes | local `admin` address | Single user entry point. |

**Upgrade workflow:**

```sh
# Single contract
make upgrade-contract CONTRACT=deposit_handler NETWORK=testnet SOURCE=deployer

# Dry-run (prints planned actions, submits nothing)
make upgrade-all DRY_RUN=1 NETWORK=testnet SOURCE=deployer

# All upgradeable contracts
make upgrade-all NETWORK=testnet SOURCE=deployer
```

`market_token` and other non-upgradeable contracts are skipped or rejected by `make upgrade-contract` and `make upgrade-all` based on lack of entrypoint.

---

## Contract Responsibility Matrix

> **Resolves issue #4 — Create a contract responsibility matrix.**

This matrix maps every contract under `contracts/*` to its state ownership, initialization parameters, access controls, collaboration graph, events, and actual upgrade characteristics.

| Contract | State Keys Owned | Init Arguments | Access Control / Roles Checked | Collaborating Contracts | Emitted Events | Upgrade Policy |
|---|---|---|---|---|---|---|
| `role_store` | `Admin`, `Initialized`, `(account, role) -> bool` | `admin: Address` | `Admin` auth for modification. | None | `RoleGranted`, `RoleRevoked` | ❌ Immutable (Rust) |
| `data_store` | `Admin`, `RoleStore`, arbitrary KV pairs | `admin: Address`, `role_store: Address` | `CONTROLLER` role for state mutation. | `role_store` | None | ❌ Immutable (Rust) |
| `oracle` | `Admin`, `RoleStore`, `DataStore`, `Passphrase`, temporary prices | `admin`, `role_store`, `data_store`, `passphrase: Bytes` | `price_keeper` / `order_keeper` role to set prices. | `role_store`, `data_store` | `prices_set` | ❌ Immutable (Rust) |
| `market_factory` | `Admin`, `RoleStore`, `DataStore`, `MarketTokenWasmHash` | `admin`, `role_store`, `data_store` | `MARKET_KEEPER` role to create markets. | Deploys `market_token`; writes to `data_store`. | `wasm_set`, `mkt_new` | ❌ Immutable (Rust) |
| `market_token` | Token balances, allowances, metadata | `admin`, `role_store`, `decimal`, `name`, `symbol` | `CONTROLLER` role to mint/burn. | `role_store` | SEP-41 Transfer, Approval, Mint, Burn | ❌ Immutable |
| `deposit_vault` | `RoleStore`, `TokenBalance(Address)` | `admin`, `role_store` | `CONTROLLER` role for `transfer_out`. | SEP-41 tokens, `role_store` | None | ❌ Immutable |
| `withdrawal_vault` | `RoleStore`, `TokenBalance(Address)` | `admin`, `role_store` | `CONTROLLER` role for `transfer_out`. | `market_token`, `role_store` | None | ❌ Immutable |
| `order_vault` | `RoleStore`, `TokenBalance(Address)` | `admin`, `role_store` | `CONTROLLER` role for `transfer_out`. | SEP-41 tokens, `role_store` | None | ❌ Immutable |
| `deposit_handler` | `Admin`, `RoleStore`, `DataStore`, `Oracle`, `DepositVault`, `LocalKey::Deposit(nonce)` | `admin`, `role_store`, `data_store`, `oracle`, `deposit_vault` | `ORDER_KEEPER` role to execute/cancel. | `deposit_vault`, `market_token`, `data_store`, `oracle`, `role_store` | `dep_req`, `dep_exec`, `dep_fail` | ✅ Upgradeable (Local Admin) |
| `withdrawal_handler` | `Admin`, `RoleStore`, `DataStore`, `Oracle`, `WithdrawalVault`, `LocalKey::Withdrawal(nonce)` | `admin`, `role_store`, `data_store`, `oracle`, `withdrawal_vault` | `ORDER_KEEPER` role to execute/cancel. | `withdrawal_vault`, `market_token`, `data_store`, `oracle`, `role_store` | `wd_req`, `wd_exec`, `wd_fail` | ❌ Immutable (Rust) |
| `order_handler` | `Admin`, `RoleStore`, `DataStore`, `Oracle`, `OrderVault`, `OrderStorageKey::Order(nonce)`, `PositionStorageKey::Position(key)` | `admin`, `role_store`, `data_store`, `oracle`, `order_vault` | `ORDER_KEEPER` for orders. `LIQUIDATION_KEEPER`/`ADL_KEEPER`/`CONTROLLER` for positions. | `order_vault`, `data_store`, `oracle`, `role_store`, libs | `ord_req`, `ord_exec`, `ord_fail`, `pos_update` | ✅ Upgradeable (Local Admin) |
| `liquidation_handler` | `Admin`, `RoleStore`, `DataStore`, `Oracle`, `OrderHandler` | `admin`, `role_store`, `data_store`, `oracle`, `order_handler` | `LIQUIDATION_KEEPER` role to liquidate. | `order_handler`, `data_store`, `oracle`, `role_store` | `liq_req` | ✅ Upgradeable (Local Admin) |
| `adl_handler` | `Admin`, `RoleStore`, `DataStore`, `Oracle`, `OrderHandler` | `admin`, `role_store`, `data_store`, `oracle`, `order_handler` | `ADL_KEEPER` role to execute ADL. | `order_handler`, `data_store`, `oracle`, `role_store` | `adl_req` | ❌ Immutable (Rust) |
| `fee_handler` | `Admin`, `RoleStore`, `DataStore` | `admin`, `role_store`, `data_store` | `FEE_KEEPER` role to claim fees. | `data_store`, `role_store`, SEP-41 tokens | `fees_claimed` | ✅ Upgradeable (Local Admin) |
| `referral_storage` | `Admin`, `ReferralKey::CodeOwner`, `ReferralKey::TraderCode`, `ReferralKey::ReferrerTier`, `ReferralKey::TierConfig` | `admin: Address` | `Admin` auth for configuring tiers. | None | `CodeRegistered`, `TraderCodeSet` | ✅ Upgradeable (Local Admin) |
| `reader` | `Admin` | `admin: Address` | None for views; local admin for upgrade. | `data_store`, `oracle`, handler view clients | None | ✅ Upgradeable (Local Admin) |
| `exchange_router` | `Admin`, `RoleStore`, `DataStore`, `DepositHandler`, `WithdrawalHandler`, `OrderHandler`, `FeeHandler`, pause flag | `admin`, `role_store`, `data_store`, `deposit_handler`, `withdrawal_handler`, `order_handler`, `fee_handler` | None for user entrypoints; local admin for pause/upgrade. | `deposit_handler`, `withdrawal_handler`, `order_handler`, `fee_handler`, SEP-41 tokens | None | ✅ Upgradeable (Local Admin) |

---

## Protocol Glossary

Definitions for every domain term used throughout the codebase and issue tracker.
New contributors should read this before working on math, risk, or fee logic.
Storage contributors should also read the [Soroban TTL and rent strategy](docs/ttl-strategy.md),
which records the current bump policy and known renewal gaps. Market operators
should read the [price impact tuning guide](docs/price-impact.md) before
changing impact factors.

| Term | Definition |
|---|---|
| **Market token** | The SEP-41 LP token minted by `market_factory` for a specific market (e.g. ETH/USD). Holding market tokens represents a proportional share of the market pool. Burned on withdrawal. Also called "GM token" in the UI. |
| **Index token** | The asset whose price determines PnL for positions in a market (e.g. ETH). Not necessarily held in the pool — only its oracle price matters. |
| **Long token** | The collateral token used for long positions (and one side of the LP pool). Typically the same asset as the index token (e.g. WETH for an ETH/USD market). |
| **Short token** | The collateral token used for short positions (and the other side of the LP pool). Typically a stablecoin (e.g. USDC). |
| **Pool amount** | The protocol-tracked balance of a specific token held by a market pool. Stored in `data_store` under `pool_amount_key(market, token)`. Updated on deposit, withdrawal, and position settlement. |
| **Open interest (OI)** | The total notional USD size of all open positions on one side (long or short) of a market. Tracked separately for longs and shorts. Used to compute funding rates and ADL eligibility. |
| **Funding fee** | A periodic payment between long and short position holders to balance open interest. The dominant side pays the subordinate side. Accumulated as `funding_amount_per_size` and settled when a position is increased, decreased, or liquidated. |
| **Claimable funding amount** (`claimable_funding_amount`) | An accrued funding rebate available to an account for one market and token. It is stored in `data_store` as a `u128` under `claimable_funding_amount_key(market, token, account)`; long-token and short-token amounts therefore have separate entries. `update_funding_state` updates the signed `i128` per-size accumulators, and `settle_funding_fees` credits this claimable balance when a position is owed funding. Funding owed by a position follows the separate signed `funding_fee_amount` path rather than being stored as a negative claimable amount. `claim_funding_fees` reduces the entry as it pays the account. |
| **Borrowing fee** | A fee charged to position holders for consuming pool liquidity. Proportional to position size and the fraction of the pool that is "borrowed" by open interest. Accumulated as a cumulative factor and settled on position change. |
| **ADL (Auto-Deleveraging)** | A risk-management mechanism that partially closes the most profitable positions on the winning side when the pool's total unrealised PnL exceeds a safe threshold. Triggered by an ADL keeper via `adl_handler`. |
| **Keeper** | An off-chain bot that submits transactions to execute pending requests (orders, deposits, withdrawals) and perform risk operations (liquidations, ADL). Keepers hold role-store roles such as `ORDER_KEEPER`, `LIQUIDATION_KEEPER`, and `ADL_KEEPER`. |
| **Controller** | A privileged role (`CONTROLLER`) granted to handler contracts, allowing them to write to `data_store` and withdraw from market pools. Assigned by the admin during deployment. |
| **Position** | An open leveraged trade, stored in `order_handler`'s persistent storage under `PositionStorageKey::Position(key)`. Tracks size, collateral, entry price, accumulated fee indices, and direction (long/short). |
| **Collateral token** | The token deposited by a trader to back a position. For longs, this is the long token; for shorts, the short token. Determines which pool the collateral is drawn from. |
| **Execution price** | The price at which an order is filled, adjusted for price impact. Derived from the oracle mid-price plus or minus the price impact of the trade on pool balance. |
| **Execution delta** (`execution_delta`) | The signed difference between an order's execution price and its oracle mid-price, expressed as an `i128` USD price at `FLOAT_PRECISION` scaling. A positive trader-relative delta denotes a worse fill (price-impact cost); a negative delta denotes a better fill (impact rebate). The delta is not stored under its own `data_store` key: its price-impact value is accounted for through `swap_impact_pool_amount_key(market, token)` or `position_impact_pool_amount_key(market)`, increasing the relevant pool for costs and drawing from it for rebates. |
| **Price impact** | A fee/rebate that incentivises trades that balance pool OI. Negative impact (paid by trader) is added to the impact pool; positive impact (rebate to trader) is drawn from the impact pool. |
| **Swap path** | An ordered list of market addresses through which tokens are routed in a multi-hop swap. Each hop transfers a token into one market pool and out of another. |
| **Trigger price** | The oracle price level at which a limit or stop order becomes eligible for execution. Checked by the keeper before calling `execute_order`. |
| **Acceptable price** | The worst execution price a trader will accept for a market order. Orders revert if the execution price exceeds this bound. |
| **Min collateral factor** | A market-level parameter (stored in `data_store`) that defines the minimum collateral-to-size ratio below which a position is liquidatable. Example: 0.01 means a position is liquidatable when remaining collateral falls below 1 % of its USD size. |
| **Max open interest (OI cap)** | A per-market, per-side limit on total notional USD open interest. Stored under `max_open_interest_key(market, is_long)`. Any position increase that would push OI beyond this cap reverts. A cap of 0 means uncapped. |
| **Keeper execution window** | The time gap between when a user creates a pending order and when a keeper executes it. Keepers control timing within this window; the `acceptable_price` field is the user's primary defence against adverse timing. |
| **Realised PnL** | PnL that has been settled to the trader's account upon a decrease or liquidation. Distinct from unrealised PnL, which is the mark-to-market gain/loss on an open position. |
| **Oracle** | The `oracle` contract that stores keeper-submitted, ed25519-verified price pairs (min/max) for each token. Prices are ledger-scoped — keepers must submit fresh prices each time they call an execution function. |
| **Instance storage** | Soroban storage bucket for small, frequently-accessed values (admin address, contract addresses). Subject to TTL rent — the protocol bumps TTL on every interaction. |
| **Persistent storage** | Soroban storage bucket for user-specific long-lived data (positions, orders, deposits). Also subject to TTL rent but with a longer minimum TTL. |
| **FLOAT_PRECISION** | The fixed-point scaling factor `10^30` used for all USD values and rate accumulators in the protocol. |
| **TOKEN_PRECISION** | `10^7` — Stellar's 7-decimal standard for token amounts. |

---

## Contributing

SO4.market is being built in the open. All nine implementation phases are complete — the full protocol logic is live in Rust/Soroban. See the issue tracker for integration tests, optimisation tasks, and frontend work.

See [CONTRIBUTING.md](CONTRIBUTING.md) for deployment, upgrade workflow rules, and the PR checklist.
For the post-issue-campaign cleanup map, see [docs/PROJECT_CLEANUP.md](docs/PROJECT_CLEANUP.md).

---

## License

MIT
