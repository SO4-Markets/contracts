# Pools Review — Implementation Plan

Derived from `docs/POOLS_REVIEEW.md`. Addresses all 9 inconsistencies against GMX Synthetics in priority order.

---

## Summary of Issues

| # | Issue | Severity | Workstream |
|---|-------|----------|------------|
| 1 | `bootstrap.sh` omits `market_type` arg | **Blocker** | Scripts |
| 2 | Bootstrap config writes are incomplete/stale | **Blocker** | Scripts |
| 3 | Test token architecture drifted from SAC to custom Soroban tokens | High | Token Architecture |
| 4 | Faucet owner model is test-only but must not leak | High | Token Architecture |
| 5 | Frontend uses symbolic IDs, not real contract addresses | High | Frontend / Config |
| 6 | GM token naming is generic (`GMX Market Token` / `GM`) | Medium | Frontend / Config |
| 7 | Pool value is simplified — missing full PnL cap model | Medium | Pool Economics |
| 8 | Single-token pools have no config or execution guard | Medium | Pool Config |
| 9 | GLV pools are wired in frontend but not deployed | Low | GLV |

---

## Current Progress

As of June 5, 2026, the testnet bootstrap blockers are resolved for the active custom-token path.

- `bootstrap.sh` now passes `market_type = sha256("DEFAULT")` to `market_factory.create_market`.
- `configure_market.sh` writes the implemented GMX-style config keys that exist in `libs/keys`.
- `market_factory` registers markets through the factory address so `MARKET_KEEPER` does not also need `CONTROLLER`.
- `deploy.sh` grants `CONTROLLER` to the admin and market factory for testnet/operator configuration.
- Single-token pools are explicitly rejected until the single-token execution paths are audited.
- Active testnet tokens are custom Soroban faucet tokens: `TUSDC`, `TWBTC`, `TETH`, and `TXLM`.
- Frontend config export now emits real Soroban contract addresses for all bootstrapped markets.

Bootstrapped testnet markets:

```text
TWBTC/TUSDC  market_token = CDDVSLBGGDV2UOFN5W72R4LW7ABYL7H7ZWVSFHGMXXB3D52ZYANC5G3L
TETH/TUSDC   market_token = CCBUUSYZJTGVA6PYUNQDFPZFHTBZ2QSHOUO7YAGRQVA46T3ZLSIYULS4
TXLM/TUSDC   market_token = CDIBR7BDCDWGAG3CC6PBKRSLMISPYKNDGE57DCZO5TMTLZK34TMGKFQQ
```

Generated frontend files:

```text
.deployed/frontend-testnet.env
.deployed/frontend-testnet.ts
```

Remaining MVP work after this pass: submit/verify oracle prices, seed initial liquidity, and run an end-to-end deposit/withdraw/order smoke test against these real market token IDs.

---

## Workstream 1 — Scripts & Bootstrap Fixes

### Issue 1: `bootstrap.sh` missing `market_type`

**Root cause.** `market_factory::create_market` signature is:
```
create_market(caller, index_token, long_token, short_token, market_type: BytesN<32>)
```
The script calls it without `market_type`, making every scripted market creation fail at the ABI boundary.

**Fix steps:**
1. In `scripts/bootstrap.sh`, compute the market type hash before the `create_market` call:
   ```bash
   MARKET_TYPE=$(stellar contract invoke \
     --id "$DATA_STORE" \
     -- sha256 "DEFAULT" 2>/dev/null \
     || python3 -c "import hashlib, sys; print(hashlib.sha256(b'DEFAULT').hexdigest())")
   ```
   Or, if a helper already exists in the contract, call it. Otherwise hardcode the known SHA-256 of `"DEFAULT"` as a 32-byte hex string.
2. Pass `--market-type "$MARKET_TYPE"` to every `create_market` invocation in the script.
3. Add the same `market_type` argument to any `make bootstrap` Make target that shells out to `bootstrap.sh`.
4. Write a smoke test: `make bootstrap` on a fresh testnet deployment must succeed end-to-end without error.

**Files to touch:**
- `scripts/bootstrap.sh`
- `mx/common.mk` or `Makefile` (any Make targets that call bootstrap)

---

### Issue 2: Bootstrap config writes are incomplete

**Root cause.** GMX markets require a full set of per-market config keys written into `data_store`. The current script attempts `pool_amount_key` on `MARKET_FACTORY` (wrong contract) and leaves most risk/fee keys unset.

**Required config keys per market** (mirroring `gmx_keys` in the Rust codebase):

| Key category | Keys |
|---|---|
| Pool size | `max_pool_amount`, `max_pool_amount_for_deposit` |
| Open interest | `max_open_interest` (long + short) |
| Reserve | `reserve_factor` (long + short), `open_interest_reserve_factor` |
| Borrowing | `borrowing_factor`, `borrowing_exponent_factor` (long + short) |
| Funding | `funding_factor`, `funding_exponent_factor`, `funding_increase_factor_per_second`, `funding_decrease_factor_per_second`, `max_funding_factor_per_second`, `min_funding_factor_per_second` |
| Swap impact | `swap_impact_factor` (positive + negative), `swap_impact_exponent_factor` |
| Position impact | `position_impact_factor` (positive + negative), `position_impact_exponent_factor`, `max_position_impact_factor`, `max_position_impact_factor_for_liquidations` |
| Fees | `swap_fee_factor`, `position_fee_factor` |
| PnL | `max_pnl_factor` (deposit/withdrawal/trader — long + short), `min_collateral_factor`, `min_collateral_factor_for_open_interest` |
| Price impact | `price_impact_pool_amount` |

**Fix steps:**
1. Create `scripts/configure_market.sh <MARKET_TOKEN> <NETWORK> <SOURCE>`:
   - Reads a TOML/JSON config file (e.g. `config/markets/TWBTC-TUSDC.toml`) for default values.
   - Iterates over all required keys and calls `data_store set_*` for each.
2. Add a `config/markets/` directory with per-market TOML files:
   ```
   config/markets/default.toml         # shared base values
   config/markets/TWBTC-TUSDC.toml     # overrides for BTC/USD
   config/markets/TETH-TUSDC.toml
   config/markets/TXLM-TUSDC.toml
   ```
3. Update `scripts/bootstrap.sh` to call `configure_market.sh` after each `create_market`.
4. Add a `make configure-markets` Make target for re-running config writes without full redeploy.
5. Remove the broken `pool_amount_key` call on `MARKET_FACTORY`.

**Files to touch:**
- `scripts/bootstrap.sh`
- `scripts/configure_market.sh` (new)
- `config/markets/` (new directory + TOML files)
- `Makefile` / `mx/common.mk`

---

## Workstream 2 — Token Architecture

### Issue 3: SAC vs custom Soroban test tokens

**Decision required.** Choose one canonical path and enforce it everywhere:

| Path | When to use | Tradeoffs |
|---|---|---|
| `test_token` + `test_faucet` | Demos, UX self-service, frontend testing | Simpler ops; not real Stellar asset plumbing |
| Stellar classic SAC | Production-like behavior, trustlines, real asset semantics | More ops overhead; matches real collateral flow |

**Fix steps (if choosing `test_token` path — the current deployed reality):**
1. Update `TEST_ASSETS.md` to remove any claim that these are SAC-wrapped classic assets; describe them as custom mintable Soroban tokens.
2. Search and remove all references to SAC-specific tooling (`wrap`, `ASSET=`, trustline steps) from the primary bootstrap flow.
3. Keep the SAC section in `TEST_ASSETS.md` but mark it `## SAC Path (Alternative)` and note it is not the active testnet path.
4. Update all Make targets in `mx/tokens.mk` to reflect which path is active and avoid mixing both in a single `bootstrap` run.

**Fix steps (if choosing SAC path — revert to canonical):**
1. Remove `contracts/test_token` and `contracts/test_faucet` from active bootstrap path.
2. Restore SAC deploy steps as the default `make market-tokens` target.
3. Keep `test_token`/`test_faucet` as an optional "demo mode" Make target.

**Files to touch:**
- `TEST_ASSETS.md` (→ `docs/TEST_ASSETS.md`)
- `mx/tokens.mk`
- `Makefile`
- `docs/FRONTEND_TESTNET_FAUCET.md` (update deployed IDs if path changes)

---

### Issue 4: Faucet ownership model must not leak to production

**Root cause.** Each `test_token` is initialized with the faucet contract as owner. This allows the faucet to mint to any user. This model is only correct for testnet.

**Fix steps:**
1. Add a compile-time or runtime guard in `contracts/test_token/src/lib.rs`:
   ```rust
   // Reject initialization on mainnet network passphrase
   const MAINNET_PASSPHRASE: &str = "Public Global Stellar Network ; September 2015";
   ```
   Or, enforce at the Make/CI level: the `test-tokens-with-faucet` target must only run with `NETWORK=testnet`.
2. Document the token list to deploy on testnet:
   ```
   TUSDC - stable short/collateral
   TWBTC - BTC/USD long + index
   TETH  - ETH/USD long + index
   TXLM  - XLM/USD long + index
   ```
3. Add `TETH` and `TXLM` to the faucet deployment script and Make targets (currently only `TWBTC` + `TUSDC` are deployed).
4. Add a `faucet-disable-token` Make target so any token can be disabled in the faucet without redeployment.
5. Write CI check: if `NETWORK=mainnet` is passed, Make must exit non-zero before deploying test contracts.

**Files to touch:**
- `contracts/test_token/src/lib.rs`
- `contracts/test_faucet/src/lib.rs`
- `mx/tokens.mk`
- `Makefile`

---

## Workstream 3 — Frontend / Config Plumbing

### Issue 5: Frontend uses symbolic IDs, not contract addresses

**Root cause.** Frontend uses strings like `BTC-BTC-USDC`, `BTC`, `USDC`. Protocol contracts require real Soroban `Address` values (32-byte C-prefixed strings).

**Fix steps:**
1. After each `make bootstrap` run, `bootstrap.sh` must emit all generated IDs into a frontend-consumable env file:
   ```bash
   # scripts/bootstrap.sh — at the end
   cat > .deployed/frontend-testnet.env <<EOF
   MARKET_TWBTC_TUSDC=$MARKET_TOKEN_TWBTC
   TOKEN_TWBTC=$TWBTC
   TOKEN_TUSDC=$TUSDC
   TOKEN_TETH=$TETH
   TOKEN_TXLM=$TXLM
   MARKET_TETH_TUSDC=$MARKET_TOKEN_TETH
   MARKET_TXLM_TUSDC=$MARKET_TOKEN_TXLM
   EOF
   ```
2. Create `scripts/export_frontend_config.sh` that reads `.deployed/testnet.env` + `.deployed/tokens-testnet.env` and produces a typed TypeScript constants file:
   ```ts
   // generated — do not edit
   export const MARKETS = {
     "TWBTC/TUSDC": { marketToken: "C...", indexToken: "C...", longToken: "C...", shortToken: "C..." },
     ...
   } as const;
   ```
3. Add a `make export-frontend-config` Make target that runs this script.
4. Communicate to the frontend team: symbolic strings must be replaced with values from `MARKETS` before any contract call.
5. Add validation in the frontend SDK layer that throws if a non-address string is passed to a contract method.

**Files to touch:**
- `scripts/bootstrap.sh`
- `scripts/export_frontend_config.sh` (new)
- `Makefile`

---

### Issue 6: GM token naming is generic

**Root cause.** Every `market_token` is initialized with `name = "GMX Market Token"` and `symbol = "GM"`. The frontend/indexer cannot distinguish markets by token metadata alone.

**Fix steps:**
1. Change `market_token` initialization to accept `name` and `symbol` from `market_factory`:
   ```rust
   // market_factory: pass descriptive name and symbol
   let name = format!("{}/{} Market", index_symbol, short_symbol);
   let symbol = format!("GM-{}", index_symbol);
   // e.g. "TWBTC/TUSDC Market", symbol "GM-TWBTC"
   ```
   Or keep `GM` as the symbol but pass a name like `"SO4 TWBTC/TUSDC"`.
2. Alternatively (lower-risk), keep the current generic metadata but ensure the frontend resolves market identity exclusively from `data_store` fields (`index_token`, `long_token`, `short_token`), never from the token `name`/`symbol`.
3. In `reader` or a new `market_reader` helper, expose a `get_market_label(market_token) -> String` view that derives the display label from stored token addresses.
4. Update `docs/POOLS_REVIEEW.md` to mark this resolved once one of the above approaches is implemented.

**Files to touch:**
- `contracts/market_factory/src/lib.rs`
- `contracts/market_token/src/lib.rs`
- Frontend config / SDK layer

---

## Workstream 4 — Pool Economics

### Issue 7: Pool value is simplified — missing full PnL cap model

**Root cause.** `market_utils::get_pool_value` sets `total_borrowing_fees: 0` and does not implement GMX's `max_pnl_factor` model, which applies different PnL cap factors for:
- deposit operations
- withdrawal operations
- trader operations

This affects pool token price calculation and therefore deposit/withdrawal output amounts.

**Fix steps (MVP documentation):**
1. Add a `// SIMPLIFIED: total_borrowing_fees always 0; full borrowing fee accrual not yet implemented` comment in `market_utils.rs` at the relevant line.
2. Add a `// SIMPLIFIED: PnL cap factor not applied; pool value may diverge from GMX model under large open interest` comment.
3. Create `docs/POOL_ECONOMICS_GAPS.md` listing exactly what is simplified and the intended full implementation (see below).

**Fix steps (full implementation — target after MVP):**
1. Implement borrowing fee accrual: track `cumulative_borrowing_factor` per side in `data_store` and update it on every position event.
2. Apply `max_pnl_factor` in `get_pool_value`:
   ```rust
   let pnl_factor_type = match op {
       PoolValueOp::Deposit => PnlFactorType::MaxPnlFactorForDeposits,
       PoolValueOp::Withdrawal => PnlFactorType::MaxPnlFactorForWithdrawals,
       PoolValueOp::Trader => PnlFactorType::MaxPnlFactorForTraders,
   };
   let capped_pnl = apply_pnl_cap(raw_pnl, pool_usd, pnl_factor_type, market);
   ```
3. Wire `get_pool_value` callers (`deposit_handler`, `withdrawal_handler`, `reader`) to pass the correct `PoolValueOp` variant.
4. Add property tests comparing pool value output before and after a sequence of deposits and withdrawals against expected invariants.

**Files to touch:**
- `contracts/market_utils/src/lib.rs`
- `contracts/deposit_handler/src/lib.rs`
- `contracts/withdrawal_handler/src/lib.rs`
- `docs/POOL_ECONOMICS_GAPS.md` (new)

---

## Workstream 5 — Advanced Pool Types

### Issue 8: Single-token pools not guarded

**Root cause.** GMX supports `longToken == shortToken` (single-token pools) with dedicated swap and price-impact logic. SO4 may technically allow equal token addresses in `create_market` but the config and execution paths are not validated for this case.

**Fix steps:**
1. In `market_factory::create_market`, add an explicit check:
   ```rust
   if long_token == short_token {
       // Either reject until single-token logic is proven:
       return Err(Error::SingleTokenPoolNotSupported);
       // Or set a flag in data_store marking this market as single-token
   }
   ```
2. If rejecting for now, document in the error enum with a `// TODO: single-token pool support` note.
3. If supporting it, audit `deposit_handler`, `withdrawal_handler`, `order_handler`, and `market_utils` for all code paths that assume `long_token != short_token` and add `is_single_token_pool` branches accordingly.
4. Do not advertise single-token GM pools in the frontend until this is complete.

**Files to touch:**
- `contracts/market_factory/src/lib.rs`
- `contracts/market_utils/src/lib.rs` (if implementing support)
- Frontend market config

---

### Issue 9: GLV pools are not deployed

**Root cause.** GLV aggregates liquidity across multiple GM markets and rebalances based on allocation/utilization rules. Frontend has GLV concepts but no GLV contracts are deployed.

**Fix steps (short-term — disable GLV cleanly):**
1. In the frontend env/config, add a feature flag:
   ```ts
   export const FEATURES = {
     glv: false, // GLV contracts not yet deployed
   } as const;
   ```
2. Guard all GLV UI paths behind `FEATURES.glv`.
3. In `docs/`, add a `GLV_IMPLEMENTATION_PLAN.md` noting GLV is out of scope for the current milestone.

**Fix steps (full GLV implementation — future milestone):**
1. Design and implement `glv_token` contract (aggregated LP token).
2. Design and implement `glv_router` contract with:
   - `deposit_to_glv(glv, gm_markets[], amounts[])` — allocates proportionally
   - `withdraw_from_glv(glv, shares)` — redeems from underlying markets
   - `rebalance(glv)` — shifts liquidity per utilization targets
3. Add `glv_factory` to deploy new GLV vaults for sets of GM markets.
4. Wire GMX-style allocation/utilization rules: max allocation per market, utilization thresholds.
5. Deploy on testnet and connect to real GM market token addresses.

**Files to touch:**
- `contracts/glv_token/` (new)
- `contracts/glv_router/` (new)
- `contracts/glv_factory/` (new)
- Frontend feature flag config

---

## Execution Sequence

Follow this order to unblock frontend and testnet usage quickly:

```
Phase 1 — Blockers (do these first, they break everything else)
  [ ] Issue 1: Fix bootstrap.sh to pass market_type
  [ ] Issue 2: Add configure_market.sh with full config key writes

Phase 2 — Token Architecture (unblocks end-to-end testing)
  [ ] Issue 3: Decide and document canonical test token path (SAC vs custom)
  [ ] Issue 4: Deploy TUSDC + TWBTC + TETH + TXLM via faucet; add mainnet guard

Phase 3 — Frontend / Config (unblocks frontend integration)
  [ ] Issue 5: Export real contract addresses from bootstrap into frontend env
  [ ] Issue 6: Fix GM token naming or add market label resolver in reader

Phase 4 — Pool Economics (unblocks accurate LP pricing)
  [ ] Issue 7 (MVP): Add simplified-economics comments and create POOL_ECONOMICS_GAPS.md
  [ ] Issue 7 (full): Implement borrowing fee accrual + PnL cap model

Phase 5 — Advanced Pool Types (gates future feature work)
  [ ] Issue 8: Guard or implement single-token pool logic
  [ ] Issue 9: Disable GLV in frontend with feature flag; plan GLV contracts
```

---

## Market Configuration Reference

For each of the three target markets, the canonical shape is:

```
TWBTC/TUSDC
  index_token  = TWBTC contract ID
  long_token   = TWBTC contract ID
  short_token  = TUSDC contract ID
  market_type  = sha256("DEFAULT")

TETH/TUSDC
  index_token  = TETH contract ID
  long_token   = TETH contract ID
  short_token  = TUSDC contract ID
  market_type  = sha256("DEFAULT")

TXLM/TUSDC
  index_token  = TXLM contract ID
  long_token   = TXLM contract ID
  short_token  = TUSDC contract ID
  market_type  = sha256("DEFAULT")
```

All three are fully-backed markets (`index == long`). No single-token or synthetic markets until Phase 5 is complete.

---

## Related Documents

- [POOLS_REVIEEW.md](POOLS_REVIEEW.md) — original review notes this plan is derived from
- [TEST_ASSETS.md](TEST_ASSETS.md) — test token deployment and configuration
- [FRONTEND_TESTNET_FAUCET.md](FRONTEND_TESTNET_FAUCET.md) — frontend faucet integration guide
- [DEPLOYMENT_CMD.md](DEPLOYMENT_CMD.md) — deployment command reference
