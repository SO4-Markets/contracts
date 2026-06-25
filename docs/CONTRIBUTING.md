# Contributing

SO4.market contracts are deployed as a connected protocol graph. Treat
`.deployed/<network>.env` as the source of truth for a network's current
deployment.

## Deployment Rules

- Use `make deploy-all NETWORK=testnet SOURCE=<key>` only for the first full
  deployment of a network.
- If `.deployed/<network>.env` already exists, do not redeploy the protocol graph
  just to test code changes. Use an upgrade command.
- Use `make deploy-force NETWORK=<network> SOURCE=<key>` only when you
  intentionally want a brand-new protocol deployment with new addresses.
- Use `make deploy-contract CONTRACT=<name> NETWORK=<network> SOURCE=<key>` only
  for standalone debugging. It does not update `.deployed/<network>.env`, does
  not initialize dependencies, and does not wire the contract into the protocol.

## Upgrade Rules

- Use `make upgrade-contract CONTRACT=<name> NETWORK=<network> SOURCE=<key>` for
  normal in-place contract changes.
- Use `make upgrade-all NETWORK=<network> SOURCE=<key>` only when every contract
  listed in `UPGRADE_CONTRACTS` exposes the required upgrade entrypoint.
- Upgradeable contracts must implement an admin-gated function equivalent to:

```rust
pub fn upgrade(env: Env, new_wasm_hash: BytesN<32>) {
    let admin: Address = env.storage().instance().get(&InstanceKey::Admin).unwrap();
    admin.require_auth();
    env.deployer().update_current_contract_wasm(new_wasm_hash);
}
```

- Do not change storage keys, enum variant order, or stored value types in an
  upgrade unless you also write and test an explicit migration path.
- Keep initialization separate from upgrades. `initialize` should run once;
  `upgrade` should preserve existing instance and persistent storage.

## Address Files

Full deployments write:

```sh
.deployed/testnet.env
.deployed/mainnet.env
.deployed/local.env
```

Test token setup writes:

```sh
.deployed/tokens-testnet.env
```

Use `make addresses NETWORK=<network>` to inspect the active deployment before
running any upgrade.

## PR Checklist

Before opening a review request, confirm every item below. Reviewers will use this list to decide whether to merge or request changes.

### Scope

- [ ] The PR addresses **one logical change** — a single issue, bug fix, or tightly related set of concerns.
- [ ] No unrelated refactors, formatting fixes, or drive-by cleanups are included. Open a separate PR for those.
- [ ] Public function signatures and storage key names are backward-compatible unless an explicit migration path is documented and tested.

### Tests

- [ ] Every new or modified function has at least one test that covers the happy path.
- [ ] Every new or modified function that can revert has at least one `#[should_panic]` (or `try_*`) test that exercises the revert condition.
- [ ] `cargo test --workspace` passes locally with no failures or ignored tests introduced by this PR.

### Build

- [ ] `cargo check --workspace` produces zero errors.
- [ ] `cargo clippy --workspace -- -D warnings` produces zero warnings.
- [ ] `stellar contract build` completes successfully (wasm artefacts are produced, not committed).

### Documentation

- [ ] Public functions have a doc comment if their behaviour is non-obvious.
- [ ] If observable behaviour changes (new entrypoints, changed error codes, new storage keys), `README.md` is updated to match.
- [ ] New domain terms introduced by the PR are added to the [glossary](README.md#protocol-glossary).

### Storage Safety (for handler changes)

- [ ] No existing `#[contracttype]` enum has had variants reordered or removed — only appended.
- [ ] No existing persistent-storage value type has changed without an explicit migration.
- [ ] New handler state follows the **local persistent storage model** (see [Local Storage Policy](#local-storage-policy)).

### Upgrade Safety (for contracts with an `upgrade` entrypoint)

- [ ] The `upgrade` function follows the admin-gated pattern in [Upgrade Rules](#upgrade-rules).
- [ ] A test verifies that an unauthorized caller reverts and that storage is intact after a successful upgrade.

## Architectural Guidelines

### Local Storage Policy
When implementing or modifying handlers (e.g. deposit, withdrawal, order, liquidation, or ADL logic), follow the **local persistent storage model** (Issue #2). Transient request states and position records must be stored in the contract's own persistent storage using unique enum keys, rather than in the shared global `data_store`. This maintains Soroban rent (TTL) isolation, enforces strict access boundaries, and optimizes CPU instruction consumption.

### Contract Responsibility Matrix
Before introducing new contract types or modifying existing ones, consult the **Contract Responsibility Matrix** in [README.md](README.md#contract-responsibility-matrix) (Issue #4). Ensure all new code complies with the specified storage access rules, caller roles, and upgrade capabilities.

## Adding a New Market

Use this guide when the protocol is already deployed and you want to add a second (or nth) perpetuals market. All steps run against a network where `.deployed/<NETWORK>.env` already exists.

The fastest path is `bash scripts/create_market.sh [NETWORK] [SOURCE_KEY]`, which automates Steps 1–3. The manual steps below describe exactly what the script does.

### Step 1: Deploy the Market Token (SEP-41)

Call `market_factory::create_market` with the three constituent token addresses. The caller must hold the `MARKET_KEEPER` role.

```bash
MARKET_TYPE=$(python3 scripts/compute_key.py market_type_default)

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

echo "market_token: $MARKET_TOKEN"
```

`create_market` deploys the GM token contract deterministically (salt = `sha256("GMX_MARKET" ‖ index ‖ long ‖ short ‖ type)`) and registers the market in `data_store` automatically. Record the returned address.

### Step 2: Configure Market Parameters in data_store

Run `configure_market.sh` to write all required keys. Every key is a `u128` stored via `data_store.set_u128`. The caller must hold the `CONTROLLER` role (the admin address set during deployment already has it).

```bash
export DATA_STORE MARKET_TOKEN LONG_TOKEN SHORT_TOKEN
bash scripts/configure_market.sh "$NETWORK" "$SOURCE"
```

The script writes these keys (all names match the functions in `libs/keys/src/lib.rs`):

| Key function | Side | Default |
|---|---|---|
| `pool_amount_key(market, long_token)` | long | 0 (set by deposits) |
| `max_pool_amount_key(market, long_token)` | long | 1 000 000 tokens |
| `max_open_interest_key(market, is_long=true)` | long | 500 000 USD |
| `max_open_interest_key(market, is_long=false)` | short | 500 000 USD |
| `min_collateral_factor_key(market)` | both | 1 % |
| `borrowing_factor_key(market, is_long=true)` | long | 10^24 |
| `borrowing_factor_key(market, is_long=false)` | short | 10^24 |
| `borrowing_exponent_factor_key(market, is_long=true)` | long | 1.0 |
| `borrowing_exponent_factor_key(market, is_long=false)` | short | 1.0 |
| `funding_exponent_factor_key(market)` | both | 1.0 |
| `position_fee_factor_key(market, for_positive_impact=true)` | both | 0.1 % |
| `position_fee_factor_key(market, for_positive_impact=false)` | both | 0.1 % |
| `position_impact_factor_key(market, is_positive=true)` | both | 10^23 |
| `position_impact_factor_key(market, is_positive=false)` | both | 2×10^23 |

All values are in `FLOAT_PRECISION` units (10^30) except pool/token amounts (10^7 stroops). Tune these values for production before launch.

### Step 3: Register the Oracle Keeper Public Key

Prices for the index token must be signed by a registered keeper. See [docs/oracle.md](oracle.md) for the full key-registration flow.

```bash
# Compute the data_store key for keeper index 0
KEY_HEX=$(python3 scripts/compute_key.py keeper_pubkey_key 0)

# Register the 32-byte ed25519 public key (CONTROLLER role required)
stellar contract invoke \
  --id   "$DATA_STORE" \
  --source "$SOURCE" \
  --network "$NETWORK" \
  -- set_bytes32 \
  --caller "$ADMIN" \
  --key    "$KEY_HEX" \
  --value  "<32-byte-pubkey-hex>"
```

Verify with:

```bash
bash scripts/submit_prices.sh "$NETWORK" my-keeper
```

### Step 4: Handler Role Grants

No additional CONTROLLER grants are needed. `market_factory` itself holds CONTROLLER and writes all market metadata to `data_store` during `create_market`. Handlers (deposit, withdrawal, order, liquidation) hold their own persistent storage and do not require any new grants per market.

If you are adding a brand-new handler contract (not adding a market), follow the role-grant steps in `scripts/bootstrap.sh`.

### Step 5: Smoke Test

Make a small initial deposit to verify the market is correctly wired:

1. Approve tokens:
   ```bash
   stellar contract invoke --id "$LONG_TOKEN"  -- approve --from "$ADMIN" --spender "$DEPOSIT_HANDLER" --amount 10000000 --expiration_ledger 999999
   stellar contract invoke --id "$SHORT_TOKEN" -- approve --from "$ADMIN" --spender "$DEPOSIT_HANDLER" --amount 10000000 --expiration_ledger 999999
   ```

2. Submit oracle prices (required before execute_deposit):
   ```bash
   bash scripts/submit_prices.sh "$NETWORK" my-keeper
   ```

3. Create and execute a deposit:
   ```bash
   bash scripts/seed_liquidity.sh "$NETWORK" "$SOURCE"
   ```

4. Assert the caller's GM token balance is greater than zero:
   ```bash
   stellar contract invoke --id "$MARKET_TOKEN" -- balance --id "$ADMIN"
   ```

If the balance is non-zero the market is live.

