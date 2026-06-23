# Pools Review: GMX Consistency Notes

This note is for backend/contracts work around SO4 GM pools, test tokens, and the faucet.

Baseline reference: GMX Synthetics models a market as:

```text
Market.Props {
  marketToken,
  indexToken,
  longToken,
  shortToken
}
```

`MarketFactory.createMarket(indexToken, longToken, shortToken, marketType)` deploys a new GM market token and stores the market fields. A GM pool consists of an index price feed plus long and short backing tokens. Fully backed markets usually use `indexToken == longToken` and a stable `shortToken`, for example ETH/USD backed by WETH-USDC.

## What Is Consistent

- `contracts/market_factory` follows the GMX shape: it takes `index_token`, `long_token`, `short_token`, and `market_type`; deploys a deterministic market token; and stores market token, index token, long token, and short token in `data_store`.
- `contracts/market_token` represents the GM LP token and also acts as the pool custodian for backing tokens after deposits are executed.
- `contracts/deposit_handler` follows the GMX two-step flow: user creates a deposit, keeper executes it with oracle prices, backing tokens move into the market token, and GM tokens are minted to the receiver.
- Pool accounting is keyed by `(market_token, token)`, which matches GMX's risk-isolated per-market accounting model.
- Fully backed test markets should be modeled as:

```text
TWBTC/USD:
  index_token = TWBTC
  long_token  = TWBTC
  short_token = TUSDC

TETH/USD:
  index_token = TETH
  long_token  = TETH
  short_token = TUSDC

TXLM/USD:
  index_token = TXLM
  long_token  = TXLM
  short_token = TUSDC
```

## Inconsistencies / Gaps To Fix

### 1. `bootstrap.sh` does not match `MarketFactory.create_market`

The deployed contract interface requires:

```text
create_market(caller, index_token, long_token, short_token, market_type)
```

but `scripts/bootstrap.sh` currently calls `create_market` without `market_type`.

Fix: compute/pass a `BytesN<32>` market type, such as `sha256("DEFAULT")`, and pass it in every market creation command. Without this, the scripted pool bootstrap is not consistent with the current contract ABI or GMX's market factory model.

### 2. Bootstrap config writes appear incomplete/stale

GMX markets require many per-market risk/config keys: max pool amounts, max open interest, reserve factors, swap/position impact factors, fee factors, borrowing factors, funding factors, and PnL caps.

Current `bootstrap.sh` mostly prints or attempts one config path and even calls a `pool_amount_key` helper on `MARKET_FACTORY`, which does not appear to exist in the factory contract. This means markets may be created but not safely configured like GMX markets.

Fix: add real key-generation helpers or a config contract/script that writes the actual `gmx_keys` keys into `data_store` for each market.

### 3. Test token approach changed from SAC assets to custom Soroban tokens

Older docs describe test assets as Stellar classic assets wrapped by SACs. The new `contracts/test_token` is a custom mintable SEP-41-like Soroban token, and `contracts/test_faucet` mints through token owner authority.

This is acceptable for testnet UX, but it is not the same as the prior SAC-based plan.

Backend should make this explicit:

- If using `test_token`, update `TEST_ASSETS.md`, Make targets, and frontend env assumptions to stop saying these are SAC-backed classic assets.
- If production-like Stellar asset behavior is desired, keep SAC tokens as the canonical path and use faucet/distributor flows around trustlines.

### 4. Faucet ownership model is test-only and must not leak into production

`test_faucet` expects each `test_token` to be initialized with the faucet contract as owner. The faucet can mint enabled tokens to any claiming user after cooldown.

That is fine for `TUSDC`, `TWBTC`, `TETH`, and `TXLM` on testnet, but production collateral must not use this owner/minter model.

Recommended testnet token list:

```text
TUSDC - stable short token / common collateral
TWBTC - BTC/USD long and index token
TETH  - ETH/USD long and index token
TXLM  - XLM/USD long and index token
```

### 5. Current frontend market IDs are symbolic, not contract addresses

Frontend currently uses symbolic market/token IDs such as `BTC-BTC-USDC`, `BTC`, and `USDC`. Contracts require real Soroban `Address` values:

```text
market = market_token contract ID
initial_long_token = test token contract ID
initial_short_token = test token contract ID
```

After pools are created, write the generated market token IDs and test token IDs into the frontend env/config. Do not call protocol contracts with symbolic strings.

### 6. GM token naming is generic

Every market token initializes with name `GMX Market Token` and symbol `GM`. This matches the broad GM concept but is weaker than production GMX UI/indexer expectations, where markets are identified by their market token address and displayed with pair labels.

Recommendation: keep `GM` as the token symbol if desired, but ensure frontend/indexer displays markets from stored `index_token`, `long_token`, and `short_token`, not only the `GM` metadata.

### 7. Pool value is simplified versus GMX

`market_utils::get_pool_value` includes long USD, short USD, impact pool, and net PnL. It currently sets `total_borrowing_fees: 0` and does not appear to apply the fuller GMX PnL cap / max PnL factor model for deposits, withdrawals, and trader operations.

This is an important difference from GMX pool pricing. It may be acceptable for an MVP, but it should be documented as simplified economics.

### 8. Single-token pools are not separately configured

GMX supports single-token pools where `longToken == shortToken`, with different assumptions around swaps and price impact. The current model may technically allow equal long/short token addresses, but the config layer does not appear to enforce the GMX-specific single-token behavior.

Recommendation: do not advertise single-token GM pools until config and execution paths are reviewed for that case.

### 9. GLV pools are not implemented consistently yet

GM pools are the current consistent piece. GLV pools in GMX aggregate liquidity across supported GM markets and shift liquidity across them based on allocation/utilization rules.

Current frontend has GLV concepts, but deployed contract env still lacks GLV router/vault contracts. Treat GLV as disabled/mock until real GLV contracts exist and are wired to real GM market token IDs.

## Recommended Backend Sequence

1. Decide canonical test token path:
   - custom `test_token` + `test_faucet`, or
   - Stellar classic assets + SAC.
2. Deploy/configure test tokens:
   - `TUSDC`, `TWBTC`, `TETH`, `TXLM`.
3. Patch `bootstrap.sh` to pass `market_type`.
4. Add robust market config writes for all required GMX-style keys.
5. Create one market first:
   - `TWBTC/TUSDC`, with `index == long == TWBTC`.
6. Submit oracle prices for both `TWBTC` and `TUSDC`.
7. Seed liquidity through `deposit_handler`.
8. Export the generated token and market token contract IDs for frontend consumption.
9. Repeat for `TETH/TUSDC` and `TXLM/TUSDC`.

## Short Answer

The market core is directionally GMX-consistent. The main issues are around scripts/config, test-token architecture drift, symbolic frontend IDs, simplified pool economics, and missing GLV support.
