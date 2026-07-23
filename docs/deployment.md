# Testnet deployment runbook

This runbook deploys SO4.market to the Stellar testnet, creates one market, and
checks it with a real deposit. Run every command from the repository root.

> The current CLI is named `stellar`. Older documentation calls the same tool
> `soroban`. This repository requires Stellar CLI 23 or newer and uses
> `wasm32v1-none`; the Rust contracts use `soroban-sdk` 25.3.1.

## 1. Prerequisites

Install Rust, the WebAssembly target, Stellar CLI, Binaryen, Python 3, `curl`,
and GNU Make:

```sh
rustup target add wasm32v1-none
cargo install --locked stellar-cli --features opt
stellar --version
wasm-opt --version
```

Create and fund a testnet identity. `--fund` calls Friendbot; the two-step form
is useful when the identity already exists:

```sh
export NETWORK=testnet
export SOURCE=so4-testnet

stellar keys generate --global "$SOURCE" --network "$NETWORK" --fund
# Existing identity only:
# stellar keys fund "$SOURCE" --network "$NETWORK"

export ADMIN=$(stellar keys address "$SOURCE")
curl "https://friendbot.stellar.org/?addr=$ADMIN" # alternative Friendbot call
```

The scripts use these variables:

| Variable | Required | Meaning |
|---|---:|---|
| `NETWORK` | Yes | Stellar network name; use `testnet` here. |
| `SOURCE` | Yes | Locally configured Stellar identity that signs deployment transactions. |
| `ADMIN` | Derived | Account address returned by `stellar keys address "$SOURCE"`. |
| `KEEPER` | No | Keeper identity; defaults to `SOURCE`. |
| `LONG_TOKEN`, `SHORT_TOKEN`, `INDEX_TOKEN` | During bootstrap | Token contract IDs for the first market. `INDEX_TOKEN` defaults to `LONG_TOKEN`. |
| `ORACLE_URL` | For signed prices/smoke test | Keeper service exposing a `/prices` response for the deployed token IDs. |
| `ORACLE_KEEPER_PUBLIC_KEY` | For signed prices | The keeper's raw 32-byte ed25519 public key as 64 hexadecimal characters. |

Keep `.deployed/*.env` private if you add operational metadata. The generated
files contain addresses, not secret keys; Stellar identity secrets remain in
the CLI's local key store.

## 2. Build all contracts

The canonical build is:

```sh
stellar contract build --release
```

It writes WASM files to `target/wasm32v1-none/release`. To make a separate set
of Binaryen `-O3` artifacts, run:

```sh
mkdir -p contracts/optimised
for contract in \
  role_store data_store oracle market_token market_factory \
  deposit_vault deposit_handler withdrawal_vault withdrawal_handler \
  order_vault order_handler liquidation_handler adl_handler fee_handler \
  referral_storage reader exchange_router
do
  wasm-opt -O3 \
    -o "contracts/optimised/$contract.wasm" \
    "target/wasm32v1-none/release/$contract.wasm"
done
```

`test_token` and `test_faucet` are test infrastructure, not part of the 17
contract protocol graph. The repository automation builds and deploys all
protocol contracts in dependency order:

```sh
make check
make test
make deploy-all NETWORK="$NETWORK" SOURCE="$SOURCE"
```

Use `make deploy-force` only when intentionally replacing an existing
`.deployed/testnet.env` with a fresh protocol graph.

## 3. Deployment order and exact initialization calls

The supported path is `scripts/deploy.sh`, invoked by `make deploy-all`. The
following expansion documents its exact order and arguments for auditing or a
manual recovery. There are no Rust constructors in this graph: deploy each
WASM first, then call its `initialize` entrypoint.

Set a helper and upload/deploy the contracts:

```sh
export WASM_DIR=target/wasm32v1-none/release

deploy() {
  name=$1
  hash=$(stellar contract upload \
    --wasm "$WASM_DIR/$name.wasm" --source "$SOURCE" --network "$NETWORK")
  stellar contract deploy \
    --wasm-hash "$hash" --source "$SOURCE" --network "$NETWORK"
}

invoke() {
  id=$1
  shift
  stellar contract invoke \
    --id "$id" --source "$SOURCE" --network "$NETWORK" -- "$@"
}

ROLE_STORE=$(deploy role_store)
invoke "$ROLE_STORE" initialize --admin "$ADMIN"

DATA_STORE=$(deploy data_store)
invoke "$DATA_STORE" initialize --admin "$ADMIN" --role_store "$ROLE_STORE"

ORACLE=$(deploy oracle)
TESTNET_PASSPHRASE_HEX=$(printf '%s' 'Test SDF Network ; September 2015' | xxd -p -c 256)
invoke "$ORACLE" initialize \
  --admin "$ADMIN" --role_store "$ROLE_STORE" --data_store "$DATA_STORE" \
  --network_passphrase "$TESTNET_PASSPHRASE_HEX"

# market_token is uploaded but deployed later by market_factory.create_market.
MARKET_TOKEN_WASM_HASH=$(stellar contract upload \
  --wasm "$WASM_DIR/market_token.wasm" --source "$SOURCE" --network "$NETWORK")
MARKET_FACTORY=$(deploy market_factory)
invoke "$MARKET_FACTORY" initialize \
  --admin "$ADMIN" --role_store "$ROLE_STORE" --data_store "$DATA_STORE"
invoke "$MARKET_FACTORY" set_market_token_wasm_hash \
  --caller "$ADMIN" --wasm_hash "$MARKET_TOKEN_WASM_HASH"

DEPOSIT_VAULT=$(deploy deposit_vault)
invoke "$DEPOSIT_VAULT" initialize --admin "$ADMIN" --role_store "$ROLE_STORE"
DEPOSIT_HANDLER=$(deploy deposit_handler)
invoke "$DEPOSIT_HANDLER" initialize \
  --admin "$ADMIN" --role_store "$ROLE_STORE" --data_store "$DATA_STORE" \
  --oracle "$ORACLE" --deposit_vault "$DEPOSIT_VAULT"

WITHDRAWAL_VAULT=$(deploy withdrawal_vault)
invoke "$WITHDRAWAL_VAULT" initialize --admin "$ADMIN" --role_store "$ROLE_STORE"
WITHDRAWAL_HANDLER=$(deploy withdrawal_handler)
invoke "$WITHDRAWAL_HANDLER" initialize \
  --admin "$ADMIN" --role_store "$ROLE_STORE" --data_store "$DATA_STORE" \
  --oracle "$ORACLE" --withdrawal_vault "$WITHDRAWAL_VAULT"

ORDER_VAULT=$(deploy order_vault)
invoke "$ORDER_VAULT" initialize --admin "$ADMIN" --role_store "$ROLE_STORE"
ORDER_HANDLER=$(deploy order_handler)
invoke "$ORDER_HANDLER" initialize \
  --admin "$ADMIN" --role_store "$ROLE_STORE" --data_store "$DATA_STORE" \
  --oracle "$ORACLE" --order_vault "$ORDER_VAULT"

LIQUIDATION_HANDLER=$(deploy liquidation_handler)
invoke "$LIQUIDATION_HANDLER" initialize \
  --admin "$ADMIN" --role_store "$ROLE_STORE" --data_store "$DATA_STORE" \
  --oracle "$ORACLE" --order_handler "$ORDER_HANDLER"

ADL_HANDLER=$(deploy adl_handler)
invoke "$ADL_HANDLER" initialize \
  --admin "$ADMIN" --role_store "$ROLE_STORE" --data_store "$DATA_STORE" \
  --oracle "$ORACLE" --order_handler "$ORDER_HANDLER"

FEE_HANDLER=$(deploy fee_handler)
invoke "$FEE_HANDLER" initialize \
  --admin "$ADMIN" --role_store "$ROLE_STORE" --data_store "$DATA_STORE"

REFERRAL_STORAGE=$(deploy referral_storage)
invoke "$REFERRAL_STORAGE" initialize --admin "$ADMIN"

READER=$(deploy reader)
invoke "$READER" initialize --admin "$ADMIN"

EXCHANGE_ROUTER=$(deploy exchange_router)
invoke "$EXCHANGE_ROUTER" initialize \
  --admin "$ADMIN" --role_store "$ROLE_STORE" --data_store "$DATA_STORE" \
  --deposit_handler "$DEPOSIT_HANDLER" \
  --withdrawal_handler "$WITHDRAWAL_HANDLER" \
  --order_handler "$ORDER_HANDLER" --fee_handler "$FEE_HANDLER"
```

Persist these IDs in `.deployed/testnet.env` if deploying manually. The
automated script writes that file and JSON manifests under `.stellar/`.

## 4. Post-deploy initialization

### Grant `CONTROLLER`

The role hash below is `gmx_keys::roles::controller()`. Grant it to the admin,
factory, state-changing handlers, router, and the oracle contract (its
circuit-breaker pauses a market by writing a market-pause flag with itself as
the caller, which is CONTROLLER-gated — omitting this grant means the very
first circuit-breaker trip panics with an authorization error):

```sh
CONTROLLER=33bf87be601326e21a8a7f573f265a6b8ab0174b8c8ec58239c8e524e4587b6a
for account in \
  "$ADMIN" "$MARKET_FACTORY" "$DEPOSIT_HANDLER" "$WITHDRAWAL_HANDLER" \
  "$ORDER_HANDLER" "$LIQUIDATION_HANDLER" "$ADL_HANDLER" \
  "$FEE_HANDLER" "$EXCHANGE_ROUTER" "$ORACLE"
do
  invoke "$ROLE_STORE" grant_role \
    --caller "$ADMIN" --account "$account" --role "$CONTROLLER"
done
```

`scripts/deploy.sh` performs these grants automatically.

### Create and configure the first market

For disposable assets, deploy the repository's faucet and two test tokens:

```sh
make test-tokens-with-faucet \
  NETWORK="$NETWORK" SOURCE="$SOURCE" LONG_CODE=TWBTC SHORT_CODE=TUSDC
```

Then create the market, grant keeper roles, and write the starter configuration:

```sh
make bootstrap \
  NETWORK="$NETWORK" SOURCE="$SOURCE" KEEPER="$SOURCE" \
  LONG_CODE=TWBTC SHORT_CODE=TUSDC SKIP_SEED=1
```

Under the hood, `bootstrap.sh` grants `MARKET_KEEPER`, `ORDER_KEEPER`,
`LIQUIDATION_KEEPER`, `ADL_KEEPER`, and `FEE_KEEPER`, then calls:

```sh
MARKET_TYPE=$(python3 scripts/compute_key.py market_type_default)
invoke "$MARKET_FACTORY" create_market \
  --caller "$ADMIN" --index_token "$INDEX_TOKEN" \
  --long_token "$LONG_TOKEN" --short_token "$SHORT_TOKEN" \
  --market_type "$MARKET_TYPE"
```

Do not skip `scripts/configure_market.sh`: deposits read its pool caps, fee
factors, impact factors, and first-deposit minimum.

### Register the oracle signer

`oracle.set_prices` reads a 32-byte ed25519 key from `data_store`. Register the
keeper at the same zero-based index emitted by the price service:

```sh
export KEEPER_INDEX=0
export ORACLE_KEEPER_PUBLIC_KEY=<64-hex-character-ed25519-public-key>
KEEPER_KEY=$(python3 scripts/compute_key.py keeper_public_key "$KEEPER_INDEX")

invoke "$DATA_STORE" set_bytes32 \
  --caller "$ADMIN" --key "$KEEPER_KEY" --value "$ORACLE_KEEPER_PUBLIC_KEY"
```

The account submitting `oracle.set_prices` must also have `ORDER_KEEPER`; the
bootstrap step grants that role to `KEEPER`. Public keys and account addresses
are different values—do not pass a Stellar `G...` address as the ed25519 key.

## 5. Smoke test

The smoke test fetches fresh signed prices, calls `create_deposit`, refreshes the
temporary oracle prices, calls `execute_deposit`, and asserts that both the
account's GM balance and market-token total supply are positive.

First claim test tokens from the faucet, then export the selected market IDs:

```sh
make faucet-claim-market \
  NETWORK="$NETWORK" SOURCE="$SOURCE" TO="$SOURCE" \
  LONG_CODE=TWBTC SHORT_CODE=TUSDC

source .deployed/testnet.env
source .deployed/tokens-testnet.env
export LONG_TOKEN=$TWBTC
export SHORT_TOKEN=$TUSDC
export INDEX_TOKEN=$LONG_TOKEN
export MARKET_TOKEN
export ORACLE_URL=https://your-keeper.example/prices

scripts/smoke_test.sh "$NETWORK" "$SOURCE"
```

The `/prices` response must contain fresh signed entries for the exact token
contract IDs above, and those signatures must match the registered keeper key.
Override `SMOKE_LONG_AMOUNT` and `SMOKE_SHORT_AMOUNT` to change the raw 7-decimal
token amounts (defaults are `100000`, or 0.01 token each).

## 6. Upgrade workflow

Build, upload the new WASM, and invoke the existing contract's admin-gated
`upgrade` entrypoint:

```sh
stellar contract build --release
NEW_WASM_HASH=$(stellar contract upload \
  --wasm target/wasm32v1-none/release/order_handler.wasm \
  --source "$SOURCE" --network "$NETWORK")

stellar contract invoke \
  --id "$ORDER_HANDLER" --source "$SOURCE" --network "$NETWORK" -- \
  upgrade --new_wasm_hash "$NEW_WASM_HASH"
```

The Make wrapper performs the same operation using `.deployed/testnet.env`:

```sh
make upgrade-contract CONTRACT=order_handler NETWORK="$NETWORK" SOURCE="$SOURCE"
```

Currently upgradeable contracts are `oracle`, `market_factory`,
`deposit_handler`, `withdrawal_handler`, `order_handler`,
`liquidation_handler`, `fee_handler`, `referral_storage`, `reader`, and
`exchange_router`. The role store, data store, vaults, ADL handler, and market
token do not expose `upgrade`; redeploying one of those requires a planned
migration and rewiring its dependants.

## Troubleshooting

- `MissingValue` or `NotInitialized`: check that every explicit `initialize`
  call above succeeded, especially `reader.initialize`.
- `Unauthorized`: verify both the required keeper role and the `CONTROLLER`
  grants.
- `StalePrice`: fetch the worker response again; signed prices are accepted for
  roughly five minutes and 60 ledgers.
- Signature failure: check `KEEPER_INDEX`, the registered raw public key, the
  network passphrase supplied to `oracle.initialize`, and token contract IDs.
- Existing `.deployed/testnet.env`: use upgrades for normal changes or
  `make deploy-force` only for an intentional fresh graph.
