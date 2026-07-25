# Exchange Router API Reference

**Issue:** [#291](https://github.com/SO4-Markets/contracts/issues/291)  
**Contract:** `contracts/exchange_router/src/lib.rs`  
**Role:** Sole user-facing entry point. Combines token transfers, vault interactions, and handler calls into atomic multicall transactions.

---

## Overview

Users call `multicall` with a `Vec<RouterAction>` to execute one or more actions atomically. A single `caller.require_auth()` covers the entire batch. Any panic inside a sub-action reverts the whole transaction.

Direct single-action helpers (`create_order`, `cancel_order`, …) are also exposed; they are the same functions called by `multicall` internally.

---

## Error Codes

| Code | Variant | When thrown |
|------|---------|-------------|
| 1 | `AlreadyInitialized` | `initialize` called after first-time setup |
| 2 | `NotInitialized` | Any function called before `initialize` |
| 3 | `Unauthorized` | `upgrade` or admin function called by non-admin |

---

## Functions

### 1. `multicall`

Execute a batch of actions atomically.

**Signature**
```
multicall(env: Env, caller: Address, actions: Vec<RouterAction>) -> Vec<BytesN<32>>
```

**Auth:** `caller` must sign. One auth covers all sub-actions.

**Parameters**
| Name | Type | Description |
|------|------|-------------|
| `caller` | `Address` | Transaction signer; authorises all sub-actions |
| `actions` | `Vec<RouterAction>` | Ordered list of actions to execute |

**Returns:** One `BytesN<32>` per action. Create actions return the new object key; all other actions return the zero hash (`[0u8; 32]`).

**Side effects:** Exactly those of each sub-action; see individual entries below.

**Errors:** Propagates any error from any sub-action (whole batch reverts).

**Example**
```bash
soroban contract invoke \
  --id $EXCHANGE_ROUTER \
  --source alice \
  -- multicall \
  --caller alice \
  --actions '[
    {"SendTokens": {"token": "$USDC", "receiver": "$ORDER_VAULT", "amount": "1000000"}},
    {"CreateOrder": { ... }}
  ]'
```

---

### 2. `create_order`

Submit a new trading order (market/limit increase, decrease, swap, or stop-loss).

**Signature**
```
create_order(env: Env, caller: Address, params: CreateOrderParams) -> BytesN<32>
```

**Auth:** `caller` (position owner or any address on behalf of the account).

**Pre-conditions**
- Collateral must have been transferred to `order_vault` **before** this call for increase/swap orders (use `send_tokens` in the same multicall).
- `params.swap_path` length must not exceed `MAX_SWAP_PATH_LENGTH` (default 5) or the DataStore-configured cap.
- `params.swap_path` must not contain duplicate market addresses.

**Parameters** (`CreateOrderParams`)
| Field | Type | Description |
|-------|------|-------------|
| `receiver` | `Address` | Receives output tokens on execution |
| `market` | `Address` | Market token address identifying the market |
| `initial_collateral_token` | `Address` | Token deposited as collateral (or swap input) |
| `swap_path` | `Vec<Address>` | Ordered market-token addresses for multi-hop swap; empty for position orders |
| `size_delta_usd` | `i128` | Position size change in USD (FLOAT_PRECISION) |
| `collateral_delta_amount` | `i128` | Collateral amount (used for decrease orders; for increase/swap, read from vault snapshot) |
| `trigger_price` | `i128` | Trigger price for limit/stop orders; 0 for market orders |
| `acceptable_price` | `i128` | Worst-case execution price; reverts if not met |
| `execution_fee` | `i128` | Fee paid to keeper for execution |
| `min_output_amount` | `i128` | Minimum output for swap orders |
| `order_type` | `OrderType` | `MarketIncrease`, `LimitIncrease`, `StopIncrease`, `MarketDecrease`, `LimitDecrease`, `StopLossDecrease`, `MarketSwap`, `LimitSwap` |
| `is_long` | `bool` | `true` for long, `false` for short |

**Returns:** `BytesN<32>` — the new order key.

**Storage written:** `OrderStorageKey::Order(key)` in `order_handler` persistent storage; order key added to global and per-account index sets in `data_store`.

**Events emitted:** `ord_crt` → `(key, caller, market)`

**Errors** (from `order_handler`)
| Code | Condition |
|------|-----------|
| `Unauthorized` | Caller lacks required role |
| `ZeroCollateral` | Increase/swap order with no collateral in vault |
| `SwapPathTooLong` | `swap_path.len() > max` |
| `DuplicateMarketInPath` | Repeated market address in path |

**Example**
```bash
soroban contract invoke \
  --id $EXCHANGE_ROUTER \
  --source alice \
  -- create_order \
  --caller alice \
  --params '{
    "receiver": "alice",
    "market": "$MARKET_TOKEN",
    "initial_collateral_token": "$USDC",
    "swap_path": [],
    "size_delta_usd": "100000000000000000000000000000000",
    "collateral_delta_amount": "0",
    "trigger_price": "0",
    "acceptable_price": "0",
    "execution_fee": "0",
    "min_output_amount": "0",
    "order_type": "MarketIncrease",
    "is_long": true
  }'
```

---

### 3. `cancel_order`

Cancel a pending order and refund any deposited collateral.

**Signature**
```
cancel_order(env: Env, caller: Address, key: BytesN<32>)
```

**Auth:** `caller` must be the order's account address OR hold `ORDER_KEEPER` role.

**Pre-conditions:** Order with `key` must exist in `order_handler` storage.

**Parameters**
| Name | Type | Description |
|------|------|-------------|
| `caller` | `Address` | Order owner or keeper |
| `key` | `BytesN<32>` | Order key returned by `create_order` |

**Side effects:** Transfers collateral back to the order's account from `order_vault` (increase/swap orders only). Removes order from global and per-account index sets.

**Events emitted:** `ord_can` → `(key, account)`

**Errors**
| Code | Condition |
|------|-----------|
| `OrderNotFound` | No order exists for `key` |
| `Unauthorized` | Caller is not the order owner and not a keeper |

---

### 4. `create_deposit`

Deposit long and/or short tokens into a market to receive LP (market) tokens.

**Signature**
```
create_deposit(env: Env, caller: Address, params: CreateDepositParams) -> BytesN<32>
```

**Auth:** `caller`.

**Pre-conditions:** Tokens must have been transferred to `deposit_vault` before this call (use `send_tokens` in the same multicall).

**Parameters** (`CreateDepositParams`)
| Field | Type | Description |
|-------|------|-------------|
| `receiver` | `Address` | Receives minted LP tokens |
| `market` | `Address` | Market token address |
| `initial_long_token` | `Address` | Long-side token to deposit |
| `initial_short_token` | `Address` | Short-side token to deposit |
| `long_token_amount` | `i128` | Amount of long token |
| `short_token_amount` | `i128` | Amount of short token |
| `min_market_tokens` | `i128` | Minimum LP tokens to mint; reverts if below |
| `execution_fee` | `i128` | Fee paid to keeper |

**Returns:** `BytesN<32>` — the new deposit key.

**Events emitted:** `dep_crt` → `(key, caller, market)`

**Errors**
| Code | Condition |
|------|-----------|
| `ZeroDeposit` | Both token amounts are zero |
| `TokenMismatch` | Tokens don't match market configuration |

---

### 5. `cancel_deposit`

Cancel a pending deposit and refund tokens to the depositor.

**Signature**
```
cancel_deposit(env: Env, caller: Address, key: BytesN<32>)
```

**Auth:** `caller` must be the deposit's account address OR hold `ORDER_KEEPER` role.

**Pre-conditions:** Deposit with `key` must exist.

**Side effects:** Returns long and short tokens from `deposit_vault` to the depositor. Removes deposit from global and per-account indexes.

**Events emitted:** `dep_can` → `(key, account)`

**Errors**
| Code | Condition |
|------|-----------|
| `DepositNotFound` | No deposit exists for `key` |
| `Unauthorized` | Caller is not the depositor and not a keeper |

---

### 6. `create_withdrawal`

Burn LP tokens to receive pro-rata long and short tokens from the pool.

**Signature**
```
create_withdrawal(env: Env, caller: Address, params: CreateWithdrawalParams) -> BytesN<32>
```

**Auth:** `caller`.

**Pre-conditions:** LP tokens (market tokens) must have been transferred to `withdrawal_vault` before this call.

**Parameters** (`CreateWithdrawalParams`)
| Field | Type | Description |
|-------|------|-------------|
| `receiver` | `Address` | Receives pool tokens |
| `market` | `Address` | Market token address (the LP token) |
| `market_token_amount` | `i128` | LP tokens to burn |
| `min_long_token_amount` | `i128` | Minimum long tokens out |
| `min_short_token_amount` | `i128` | Minimum short tokens out |
| `execution_fee` | `i128` | Fee paid to keeper |

**Returns:** `BytesN<32>` — the new withdrawal key.

**Events emitted:** `wth_crt` → `(key, caller, market)`

**Errors**
| Code | Condition |
|------|-----------|
| `ZeroWithdrawal` | `market_token_amount` is zero or negative |
| `InvalidReceiver` | Receiver is the handler contract itself |

---

### 7. `cancel_withdrawal`

Cancel a pending withdrawal and refund LP tokens to the withdrawer.

**Signature**
```
cancel_withdrawal(env: Env, caller: Address, key: BytesN<32>)
```

**Auth:** `caller` must be the withdrawal's account OR hold `ORDER_KEEPER` role.

**Side effects:** Returns LP tokens from `withdrawal_vault` to the withdrawer's account.

**Events emitted:** `wth_can` → `(key, account)`

**Errors**
| Code | Condition |
|------|-----------|
| `WithdrawalNotFound` | No withdrawal for `key` |
| `Unauthorized` | Caller is not the withdrawer and not a keeper |

---

### 8. `claim_funding_fees`

Claim accumulated funding fees across multiple markets in one call.

**Signature**
```
claim_funding_fees(
    env: Env,
    caller: Address,
    markets: Vec<Address>,
    tokens: Vec<Address>,
)
```

**Auth:** `caller`.

**Pre-conditions:** `markets.len() == tokens.len()`. Each `(market, token)` pair must have a non-zero claimable balance for `caller` in `fee_handler`.

**Parameters**
| Name | Type | Description |
|------|------|-------------|
| `markets` | `Vec<Address>` | Market token addresses |
| `tokens` | `Vec<Address>` | Collateral token addresses (parallel to `markets`) |

**Side effects:** For each pair, transfers the claimable funding fee from the market pool to `caller`. Zeroes the claimable balance in `fee_handler`.

**Events emitted:** One `fee_clm` event per market/token pair (emitted by `fee_handler`).

**Errors**
| Code | Condition |
|------|-----------|
| Any fee_handler error | Propagated per-market |

**Example**
```bash
soroban contract invoke \
  --id $EXCHANGE_ROUTER \
  --source alice \
  -- claim_funding_fees \
  --caller alice \
  --markets '["$MARKET_TOKEN"]' \
  --tokens '["$USDC"]'
```

---

### 9. `send_tokens`

Transfer tokens from caller to a receiver (typically a vault).

**Signature**
```
send_tokens(env: Env, caller: Address, token: Address, receiver: Address, amount: i128)
```

**Auth:** `caller`.

**Pre-conditions:** Caller must have approved the exchange router for at least `amount` of `token` via the SEP-41 `approve` function.

**Side effects:** Calls `token.transfer(caller, receiver, amount)` — moves tokens on-chain.

**Errors:** Any SEP-41 transfer error (insufficient balance, insufficient allowance).

**Example**
```bash
# Fund the order vault before creating an order
soroban contract invoke \
  --id $EXCHANGE_ROUTER \
  --source alice \
  -- send_tokens \
  --caller alice \
  --token $USDC \
  --receiver $ORDER_VAULT \
  --amount 1000000
```

---

### 10. `upgrade`

Upgrade the router's WASM bytecode to a new hash.

**Signature**
```
upgrade(env: Env, new_wasm_hash: BytesN<32>)
```

**Auth:** Stored admin address only.

**Pre-conditions:** `initialize` must have been called. `new_wasm_hash` must be a previously uploaded WASM hash on Stellar.

**Side effects:** Replaces the contract's WASM code with the new hash via `env.deployer().update_current_contract_wasm(new_wasm_hash)`.

**Errors**
| Code | Condition |
|------|-----------|
| `NotInitialized` | Contract not yet initialized |
| `Unauthorized` | Caller is not the stored admin |

---

## RouterAction Enum

`multicall` accepts `Vec<RouterAction>`:

```rust
pub enum RouterAction {
    SendTokens(SendTokensParams),
    CreateDeposit(CreateDepositParams),
    CancelDeposit(BytesN<32>),
    CreateWithdrawal(CreateWithdrawalParams),
    CancelWithdrawal(BytesN<32>),
    CreateOrder(CreateOrderParams),
    UpdateOrder(UpdateOrderParams),
    CancelOrder(BytesN<32>),
    ClaimFundingFees(ClaimFundingFeesParams),
}
```

### `UpdateOrderParams`

Modify a pending order's parameters before execution:

| Field | Type | Description |
|-------|------|-------------|
| `key` | `BytesN<32>` | Order key to modify |
| `size_delta_usd` | `i128` | New size delta |
| `acceptable_price` | `i128` | New acceptable price |
| `trigger_price` | `i128` | New trigger price |
| `min_output_amount` | `i128` | New minimum output |

Updating also clears any frozen flag on the order.

---

## Typical Flow: Open a Long Position

```bash
# 1. Approve exchange_router to move your USDC
soroban contract invoke --id $USDC --source alice \
  -- approve --from alice --spender $EXCHANGE_ROUTER \
  --amount 1000000 --expiration_ledger 99999999

# 2. Atomic multicall: send collateral + create order
soroban contract invoke --id $EXCHANGE_ROUTER --source alice \
  -- multicall --caller alice --actions '[
    {"SendTokens": {"token": "$USDC", "receiver": "$ORDER_VAULT", "amount": "1000000"}},
    {"CreateOrder": {
      "receiver": "alice",
      "market": "$MARKET_TOKEN",
      "initial_collateral_token": "$USDC",
      "swap_path": [],
      "size_delta_usd": "500000000000000000000000000000000",
      "collateral_delta_amount": "0",
      "trigger_price": "0",
      "acceptable_price": "0",
      "execution_fee": "0",
      "min_output_amount": "0",
      "order_type": "MarketIncrease",
      "is_long": true
    }}
  ]'
```

The keeper then calls `order_handler.execute_order` to fill the order at the current oracle price.
