# SO4.market Make Workflows

The root `Makefile` includes the files in this directory. Keep routine operator
commands here instead of growing a single large Makefile.

## Build and test

```sh
make check
make lint
make test
make build
make test-one PACKAGE=deposit-handler
```

## Deploy

```sh
make deploy-all NETWORK=testnet SOURCE=alice
make deploy-contract CONTRACT=reader NETWORK=testnet SOURCE=alice
make deploy-mainnet SOURCE=deployer
make addresses NETWORK=testnet
```

Full deployments write addresses to `.deployed/<network>.env`.
If that file already exists, `make deploy-all` stops and prints upgrade commands so
you do not accidentally create a second protocol deployment. To intentionally
redeploy everything, use:

```sh
make deploy-force NETWORK=testnet SOURCE=alice
```

`deploy-contract` is for standalone debugging. It deploys one Wasm and prints the
new address, but it does not update `.deployed/<network>.env` or wire the
contract into the protocol graph.

## Upgrade

Upgrades are a two-step Soroban flow:

1. Upload the new Wasm and get a `BytesN<32>` Wasm hash.
2. Invoke `upgrade --new_wasm_hash <hash>` on the existing contract address.

The contract address stays the same. The deployed contract must already expose an
admin-gated `upgrade(env, new_wasm_hash)` function that calls
`env.deployer().update_current_contract_wasm(new_wasm_hash)`.

```sh
make upgrade-contract CONTRACT=deposit_handler NETWORK=testnet SOURCE=alice
make upgrade-all NETWORK=testnet SOURCE=alice
make upload CONTRACT=deposit_handler NETWORK=testnet SOURCE=alice
make upgrade-with-hash CONTRACT_ID=C... WASM_HASH=... NETWORK=testnet SOURCE=alice
```

`upgrade-all` upgrades every deployed contract listed in `UPGRADE_CONTRACTS`.
Every listed contract must already expose the admin-gated `upgrade` entrypoint.

## Test Tokens

Use Stellar Asset Contracts for test assets such as test WBTC. A Stellar asset is
identified by `CODE:ISSUER`, and its SAC gives your contracts a normal SEP-41
token address.

```sh
make token-bootstrap CODE=TWBTC TO=alice NETWORK=testnet SOURCE=alice
make token-bootstrap CODE=TUSDC TO=alice NETWORK=testnet SOURCE=alice
make tokens NETWORK=testnet
```

Amounts are raw 7-decimal units for this protocol. For example:

```sh
make token-mint CODE=TWBTC TO=alice AMOUNT=100000000 NETWORK=testnet SOURCE=alice
```

That mints `10.0000000` TWBTC.

For mainnet, do not use `token-bootstrap` for real assets. Deploy or look up the
existing SAC for the real Stellar asset and configure markets with that address.

## Oracle — submitting prices

The keeper calls `set_prices_simple` on the oracle contract before each execution.
All four env vars below are **required** — the script exits non-zero immediately if
any are unset.  There is no placeholder fallback.

```sh
export ORACLE=C...          # deployed oracle contract address
export TOKEN=C...           # token contract address to price
export MIN_PRICE=500000000000000000000000000000000   # FLOAT_PRECISION (10^30) scaled
export MAX_PRICE=500500000000000000000000000000000
make submit-prices NETWORK=testnet SOURCE=alice
```

`SOURCE` must be a key that holds the `ORDER_KEEPER` role in `role_store`.
Grant the role with:

```sh
make grant-keeper KEEPER=alice NETWORK=testnet SOURCE=admin
```

Where `SOURCE` (admin) holds the `ROLE_ADMIN` role and `KEEPER` is the key being
promoted.  Both default to `alice` so a single-operator testnet setup works
without extra flags.

You can also call the script directly:

```sh
ORACLE=C... TOKEN=C... MIN_PRICE=... MAX_PRICE=... \
  bash scripts/submit_prices.sh testnet alice
```
