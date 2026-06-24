## Pre-deploy local checks

Run these before any deploy or upgrade:

```sh
cargo fmt
make check
make test
stellar contract build --optimize
```

Optional strict lint:

```sh
make lint
```

`make lint` runs `cargo clippy --workspace -- -D warnings`, so it can fail on
warnings that do not block `make test`.

---

## Testnet fresh deployment

Create and fund the deployer:

```sh
stellar keys generate steins-testnet --network testnet --fund
stellar keys address steins-testnet
```

Create test assets, deploy a fresh protocol graph, bootstrap the market, then
print addresses:

```sh
make market-tokens NETWORK=testnet SOURCE=steins-testnet LONG_CODE=TWBTC SHORT_CODE=TUSDC
make deploy-force NETWORK=testnet SOURCE=steins-testnet
make bootstrap NETWORK=testnet SOURCE=steins-testnet LONG_CODE=TWBTC SHORT_CODE=TUSDC
make addresses NETWORK=testnet
make testnet-smoke NETWORK=testnet SOURCE=steins-testnet
```

Use `deploy-force` only when you intentionally want new contract IDs and want to
overwrite `.deployed/testnet.env`. For normal code changes, use upgrades below.

### Testnet with faucet-owned native tokens

Use this path when you want users to claim demo collateral themselves. It deploys
the repo-local `test_token` contracts and a `test_faucet` contract before the
GMX-style protocol graph:

```sh
make test-tokens-with-faucet NETWORK=testnet SOURCE=steins-testnet LONG_CODE=TWBTC SHORT_CODE=TUSDC
make deploy-force NETWORK=testnet SOURCE=steins-testnet
make bootstrap NETWORK=testnet SOURCE=steins-testnet LONG_CODE=TWBTC SHORT_CODE=TUSDC
make addresses NETWORK=testnet
```

After that, users can claim:

```sh
make faucet-claim-market NETWORK=testnet SOURCE=steins-testnet TO=<stellar-key-or-G-address> \
  LONG_CODE=TWBTC SHORT_CODE=TUSDC
```

The faucet deployment writes `FAUCET`, `TWBTC`, and `TUSDC` to
`.deployed/tokens-testnet.env`, which `bootstrap` already reads.

---

## Testnet upgrade

Upgrade one contract:

```sh
make upgrade-contract CONTRACT=order_handler NETWORK=testnet SOURCE=steins-testnet
make upgrade-contract CONTRACT=deposit_handler NETWORK=testnet SOURCE=steins-testnet
make upgrade-contract CONTRACT=exchange_router NETWORK=testnet SOURCE=steins-testnet
```

Upgrade all contracts that currently expose a Rust `upgrade` entrypoint:

```sh
make upgrade-all NETWORK=testnet SOURCE=steins-testnet \
  UPGRADE_CONTRACTS="deposit_handler order_handler liquidation_handler fee_handler referral_storage reader exchange_router"
```

Upload only, then manually upgrade with a known hash:

```sh
make upload CONTRACT=order_handler NETWORK=testnet SOURCE=steins-testnet

make upgrade-with-hash \
  CONTRACT_ID=C... \
  WASM_HASH=... \
  NETWORK=testnet \
  SOURCE=steins-testnet
```

---

## Mainnet fresh deployment

Create a mainnet deployer identity:

```sh
stellar keys generate steins-mainnet --network mainnet
stellar keys address steins-mainnet
```

Fund `steins-mainnet` manually with real XLM before deploying.

For mainnet, do not use `make market-tokens`; that creates test assets. Use real
Stellar Asset Contract IDs for the market tokens:

```sh
export LONG_TOKEN=C...
export SHORT_TOKEN=C...
export INDEX_TOKEN=$LONG_TOKEN
```

Deploy the protocol graph:

```sh
make deploy-all NETWORK=mainnet SOURCE=steins-mainnet
```

If you are intentionally replacing a previous mainnet deployment with brand-new
contract IDs:

```sh
make deploy-force NETWORK=mainnet SOURCE=steins-mainnet
```

Bootstrap with real asset contract IDs and a keeper account:

```sh
make bootstrap NETWORK=mainnet SOURCE=steins-mainnet KEEPER=steins-keeper-mainnet \
  LONG_TOKEN=$LONG_TOKEN \
  SHORT_TOKEN=$SHORT_TOKEN \
  INDEX_TOKEN=$INDEX_TOKEN \
  SKIP_SEED=1

make addresses NETWORK=mainnet
```

Mainnet liquidity seeding should be done manually after oracle/keeper setup and
after reviewing config values.

---

## Mainnet upgrade

Upgrade one contract:

```sh
make upgrade-contract CONTRACT=order_handler NETWORK=mainnet SOURCE=steins-mainnet
make upgrade-contract CONTRACT=deposit_handler NETWORK=mainnet SOURCE=steins-mainnet
make upgrade-contract CONTRACT=exchange_router NETWORK=mainnet SOURCE=steins-mainnet
```

Upgrade all contracts that currently expose a Rust `upgrade` entrypoint:

```sh
make upgrade-all NETWORK=mainnet SOURCE=steins-mainnet \
  UPGRADE_CONTRACTS="deposit_handler order_handler liquidation_handler fee_handler referral_storage reader exchange_router"
```

Upload only, then manually upgrade with a known hash:

```sh
make upload CONTRACT=order_handler NETWORK=mainnet SOURCE=steins-mainnet

make upgrade-with-hash \
  CONTRACT_ID=C... \
  WASM_HASH=... \
  NETWORK=mainnet \
  SOURCE=steins-mainnet
```

---

## Useful inspection commands

```sh
make list-contracts
make addresses NETWORK=testnet
make addresses NETWORK=mainnet

make inspect CONTRACT=order_handler
make inspect CONTRACT=exchange_router
```

Current upgradeable contracts:

```txt
deposit_handler
order_handler
liquidation_handler
fee_handler
referral_storage
reader
exchange_router
```

Do not include immutable contracts in `UPGRADE_CONTRACTS` unless their Rust code
has first been changed to expose an admin-gated `upgrade` function.
