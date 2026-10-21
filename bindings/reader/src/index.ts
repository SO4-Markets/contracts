import { Buffer } from "buffer";
import { Address } from "@stellar/stellar-sdk";
import {
  AssembledTransaction,
  Client as ContractClient,
  ClientOptions as ContractClientOptions,
  MethodOptions,
  Result,
  Spec as ContractSpec,
} from "@stellar/stellar-sdk/contract";
import type {
  u32,
  i32,
  u64,
  i64,
  u128,
  i128,
  u256,
  i256,
  Option,
  Timepoint,
  Duration,
} from "@stellar/stellar-sdk/contract";
export * from "@stellar/stellar-sdk";
export * as contract from "@stellar/stellar-sdk/contract";
export * as rpc from "@stellar/stellar-sdk/rpc";

if (typeof window !== "undefined") {
  //@ts-ignore Buffer exists
  window.Buffer = window.Buffer || Buffer;
}




export const Errors = {
  1: {message:"NotInitialized"},
  2: {message:"AlreadyInitialized"},
  /**
   * `get_protocol_stats` was passed more than `MAX_STATS_MARKETS` markets.
   */
  4: {message:"TooManyMarkets"},
  /**
   * The supplied market address is not registered in data_store —
   * one or more of its token addresses (index/long/short) returned None.
   * Mirrors the InvalidMarket pattern in deposit_handler/withdrawal_handler
   * (issue #371) so callers can match this condition as a typed error
   * instead of a generic execution failure.
   */
  5: {message:"InvalidMarket"}
}

/**
 * Mirrors GMX's Order.OrderType.
 */
export type OrderType = {tag: "MarketSwap", values: void} | {tag: "LimitSwap", values: void} | {tag: "MarketIncrease", values: void} | {tag: "LimitIncrease", values: void} | {tag: "MarketDecrease", values: void} | {tag: "LimitDecrease", values: void} | {tag: "StopLossDecrease", values: void} | {tag: "Liquidation", values: void} | {tag: "StopIncrease", values: void};


/**
 * Mirrors GMX's Order.Props.
 */
export interface OrderProps {
  acceptable_price: i128;
  account: string;
  collateral_delta_amount: i128;
  execution_fee: i128;
  initial_collateral_token: string;
  is_long: boolean;
  market: string;
  min_output_amount: i128;
  order_type: OrderType;
  receiver: string;
  size_delta_usd: i128;
  swap_path: Array<string>;
  trigger_price: i128;
  updated_at_time: u64;
}


/**
 * USD price with min/max spread; 30-decimal FLOAT_PRECISION. Mirrors GMX's Price.Props.
 */
export interface PriceProps {
  max: i128;
  min: i128;
}


/**
 * Used by keepers to submit prices to the oracle contract.
 */
export interface TokenPrice {
  max: i128;
  min: i128;
  token: string;
}


/**
 * Aggregate funding information for a market (used by Reader).
 */
export interface FundingInfo {
  funding_factor_per_second: i128;
  long_funding_amount_per_size: i128;
  short_funding_amount_per_size: i128;
}


/**
 * Mirrors GMX's Market.Props.
 */
export interface MarketProps {
  index_token: string;
  long_token: string;
  market_token: string;
  short_token: string;
}


/**
 * ADL-eligible position candidate for auto-deleveraging.
 * Returned by reader::get_adl_eligible_positions.
 */
export interface AdlCandidate {
  key: Buffer;
  owner: string;
  pnl_to_size_ratio_bps: u32;
  size_usd: u128;
  unrealised_pnl_usd: u128;
}


/**
 * Mirrors GMX's Deposit.Props.
 */
export interface DepositProps {
  account: string;
  execution_fee: i128;
  initial_long_token: string;
  initial_short_token: string;
  long_token_amount: i128;
  market: string;
  min_market_tokens: i128;
  receiver: string;
  short_token_amount: i128;
  updated_at_time: u64;
}


/**
 * Lightweight pending-order summary returned by reader::get_pending_orders (issue #202).
 * 
 * Carries only the fields a keeper bot needs to decide execution order:
 * owner, type, size, fee, last-update time, and direction.
 * Full details can always be fetched via reader::get_order if needed.
 */
export interface PendingOrder {
  execution_fee: i128;
  is_long: boolean;
  market: string;
  order_type: OrderType;
  owner: string;
  size_delta_usd: i128;
  updated_at_time: u64;
}


/**
 * Position fee breakdown.
 */
export interface PositionFees {
  borrowing_fee_amount: i128;
  funding_fee_amount: i128;
  position_fee_amount: i128;
  total_cost_amount: i128;
}


/**
 * Rich position info including computed PnL and fees (returned by Reader).
 */
export interface PositionInfo {
  /**
 * Issue #260: weighted average entry price across all position increases.
 * Computed as size_in_usd / size_in_tokens × TOKEN_PRECISION (FLOAT_PRECISION units).
 * Zero when size_in_tokens is zero (no open position).
 */
avg_entry_price: i128;
  borrowing_fee_usd: i128;
  funding_fee_usd: i128;
  liquidation_price: i128;
  pnl_usd: i128;
  position: PositionProps;
  position_fee_usd: i128;
  uncapped_pnl_usd: i128;
}


/**
 * Estimated swap output for dry-run queries.
 * Returned by reader::estimate_swap_output.
 */
export interface SwapEstimate {
  amount_out: u128;
  execution_price: u128;
  price_impact_usd: i128;
  reverts_if_executed: boolean;
  token_out: string;
}


/**
 * Full pool value breakdown returned by market_utils::get_pool_value.
 */
export interface PoolValueInfo {
  impact_pool_amount: i128;
  long_pnl: i128;
  long_token_amount: i128;
  long_token_usd: i128;
  net_pnl: i128;
  pool_value: i128;
  short_pnl: i128;
  short_token_amount: i128;
  short_token_usd: i128;
}


/**
 * Mirrors GMX's Position.Props.
 * Field name abbreviations (30-char limit in #[contracttype]):
 * long_claim_fnd_per_size  = longTokenClaimableFundingAmountPerSize
 * short_claim_fnd_per_size = shortTokenClaimableFundingAmountPerSize
 */
export interface PositionProps {
  account: string;
  borrowing_factor: i128;
  collateral_amount: i128;
  collateral_token: string;
  decreased_at_time: u64;
  funding_fee_amount_per_size: i128;
  increased_at_time: u64;
  is_long: boolean;
  long_claim_fnd_per_size: i128;
  market: string;
  pending_impact_amount: i128;
  short_claim_fnd_per_size: i128;
  size_in_tokens: i128;
  size_in_usd: i128;
}


/**
 * Aggregated protocol-wide statistics across a set of markets (returned by Reader).
 * 
 * Issue #251: lets the frontend fetch headline numbers (TVL, OI, accumulated
 * fees) in a single call instead of N per-market round-trips. All USD figures
 * use 30-decimal FLOAT_PRECISION and reflect the oracle prices at the ledger
 * in which the call executes — the result is a snapshot, valid only for
 * `computed_at_ledger`.
 */
export interface ProtocolStats {
  computed_at_ledger: u64;
  market_count: u32;
  total_accumulated_fees_usd: i128;
  total_long_open_interest_usd: i128;
  total_pool_value_usd: i128;
  total_short_open_interest_usd: i128;
}


/**
 * Detailed per-hour funding rate view for frontend display (issue #207).
 */
export interface FundingRateInfo {
  funding_updated_at_ledger: u64;
  long_funding_amount_per_size: i128;
  long_funding_rate_per_hour: i128;
  long_open_interest_usd: u128;
  short_funding_amount_per_size: i128;
  short_funding_rate_per_hour: i128;
  short_open_interest_usd: u128;
}


/**
 * Mirrors GMX's Withdrawal.Props.
 */
export interface WithdrawalProps {
  account: string;
  execution_fee: i128;
  market: string;
  market_token_amount: i128;
  min_long_token_amount: i128;
  min_short_token_amount: i128;
  receiver: string;
  updated_at_time: u64;
}


/**
 * Position leverage breakdown returned by reader::get_position_leverage.
 * effective_leverage_x100 = size_usd * 100 / net_collateral_usd (e.g. 2000 = 20×).
 * This is NOT basis points (BPS_DIVISOR = 10_000) — genuine bps would represent
 * 20× as 200_000, not 2000. It's a leverage-times-100 scale.
 * If net_collateral_usd == 0, effective_leverage_x100 == u32::MAX.
 */
export interface PositionLeverage {
  effective_leverage_x100: u32;
  is_liquidatable: boolean;
  net_collateral_usd: u128;
  position_size_usd: u128;
}


/**
 * User-supplied parameters for creating an order. Mirrors GMX BaseOrderUtils.CreateOrderParams.
 */
export interface CreateOrderParams {
  acceptable_price: i128;
  collateral_delta_amount: i128;
  execution_fee: i128;
  /**
 * Optional ledger-sequence deadline. If `Some(n)` the order auto-cancels
 * (with full refund) when the keeper calls `execute_order` after sequence `n`.
 * `None` means the order never expires via this mechanism.
 */
expiry_ledger: Option<u64>;
  initial_collateral_token: string;
  is_long: boolean;
  market: string;
  min_output_amount: i128;
  /**
 * Issue #385/#535: when a registered position manager calls on behalf of
 * an owner, the owner is named here explicitly. `Some(owner)` requires
 * `data_store::get_position_manager(owner, market) == Some(caller)`;
 * `None` means the caller is acting for themselves.
 */
on_behalf_of: Option<string>;
  order_type: OrderType;
  receiver: string;
  size_delta_usd: i128;
  swap_path: Array<string>;
  trigger_price: i128;
}


/**
 * User-supplied parameters for creating a deposit.
 */
export interface CreateDepositParams {
  execution_fee: i128;
  initial_long_token: string;
  initial_short_token: string;
  long_token_amount: i128;
  market: string;
  min_market_tokens: i128;
  receiver: string;
  short_token_amount: i128;
}


/**
 * Pending (not-yet-settled) funding for a single position — issue #275.
 * Positive = amount the holder would receive; negative = amount the holder
 * would pay, in each of the position's long_token / short_token units.
 */
export interface FundingAmountResult {
  at_ledger: u64;
  long_token_amount: i128;
  short_token_amount: i128;
}


/**
 * Liquidatable position entry returned by `reader::get_liquidatable_positions` (issue #283).
 * 
 * `health_factor_bps < 10000` means the position is below the min-collateral threshold
 * (i.e., currently eligible for liquidation). Sorted ascending so the most
 * under-collateralised positions appear first.
 */
export interface LiquidatablePosition {
  collateral_usd: u128;
  health_factor_bps: u32;
  key: Buffer;
  owner: string;
  size_usd: u128;
}


/**
 * Liveness status for a keeper role (issue #249), returned by Reader.
 * 
 * A keeper that has gone permanently offline leaves orders unexecuted. This
 * gives the admin an on-chain signal: when `is_stale` is true the gap since the
 * last successful execution has exceeded the configured heartbeat timeout, and
 * the keeper's role can be revoked.
 */
export interface KeeperHeartbeatStatus {
  is_stale: boolean;
  last_active_ledger: u64;
  ledgers_since_last_activity: u64;
}


/**
 * User-supplied parameters for creating a withdrawal.
 */
export interface CreateWithdrawalParams {
  execution_fee: i128;
  market: string;
  market_token_amount: i128;
  min_long_token_amount: i128;
  min_short_token_amount: i128;
  receiver: string;
}


/**
 * Result of executing a position decrease (partial or full close).
 */
export interface DecreasePositionResult {
  execution_price: i128;
  is_fully_closed: boolean;
  output_amount: i128;
  pnl_usd: i128;
  remaining_collateral: i128;
  secondary_output_amount: i128;
}

export interface Client {
  /**
   * Construct and simulate a upgrade transaction. Returns an `AssembledTransaction` object which will have a `result` field containing the result of the simulation. If this transaction changes contract state, you will need to call `signAndSend()` on the returned object.
   * Upgrade the contract wasm. Only the stored admin may call this.
   */
  upgrade: ({new_wasm_hash}: {new_wasm_hash: Buffer}, options?: MethodOptions) => Promise<AssembledTransaction<null>>

  /**
   * Construct and simulate a get_order transaction. Returns an `AssembledTransaction` object which will have a `result` field containing the result of the simulation. If this transaction changes contract state, you will need to call `signAndSend()` on the returned object.
   * Return a stored order by key, or None if not found.
   */
  get_order: ({order_handler, key}: {order_handler: string, key: Buffer}, options?: MethodOptions) => Promise<AssembledTransaction<Option<OrderProps>>>

  /**
   * Construct and simulate a get_market transaction. Returns an `AssembledTransaction` object which will have a `result` field containing the result of the simulation. If this transaction changes contract state, you will need to call `signAndSend()` on the returned object.
   * Load full MarketProps for a given market_token address from data_store.
   */
  get_market: ({data_store, market_token}: {data_store: string, market_token: string}, options?: MethodOptions) => Promise<AssembledTransaction<MarketProps>>

  /**
   * Construct and simulate a initialize transaction. Returns an `AssembledTransaction` object which will have a `result` field containing the result of the simulation. If this transaction changes contract state, you will need to call `signAndSend()` on the returned object.
   * One-time setup — store the admin address.
   */
  initialize: ({admin}: {admin: string}, options?: MethodOptions) => Promise<AssembledTransaction<null>>

  /**
   * Construct and simulate a get_deposit transaction. Returns an `AssembledTransaction` object which will have a `result` field containing the result of the simulation. If this transaction changes contract state, you will need to call `signAndSend()` on the returned object.
   * Get a deposit by key (delegates to deposit_handler).
   */
  get_deposit: ({deposit_handler, key}: {deposit_handler: string, key: Buffer}, options?: MethodOptions) => Promise<AssembledTransaction<Option<DepositProps>>>

  /**
   * Construct and simulate a get_order_keys transaction. Returns an `AssembledTransaction` object which will have a `result` field containing the result of the simulation. If this transaction changes contract state, you will need to call `signAndSend()` on the returned object.
   * Paginated list of all order keys (raw BytesN<32>).
   */
  get_order_keys: ({data_store, start, end}: {data_store: string, start: u32, end: u32}, options?: MethodOptions) => Promise<AssembledTransaction<Array<Buffer>>>

  /**
   * Construct and simulate a get_withdrawal transaction. Returns an `AssembledTransaction` object which will have a `result` field containing the result of the simulation. If this transaction changes contract state, you will need to call `signAndSend()` on the returned object.
   * Get a withdrawal by key (delegates to withdrawal_handler).
   */
  get_withdrawal: ({withdrawal_handler, key}: {withdrawal_handler: string, key: Buffer}, options?: MethodOptions) => Promise<AssembledTransaction<Option<WithdrawalProps>>>

  /**
   * Construct and simulate a get_order_count transaction. Returns an `AssembledTransaction` object which will have a `result` field containing the result of the simulation. If this transaction changes contract state, you will need to call `signAndSend()` on the returned object.
   * Count of all pending order keys in DataStore.
   */
  get_order_count: ({data_store}: {data_store: string}, options?: MethodOptions) => Promise<AssembledTransaction<u32>>

  /**
   * Construct and simulate a get_deposit_keys transaction. Returns an `AssembledTransaction` object which will have a `result` field containing the result of the simulation. If this transaction changes contract state, you will need to call `signAndSend()` on the returned object.
   * Paginated list of all deposit keys (raw BytesN<32>).
   */
  get_deposit_keys: ({data_store, start, end}: {data_store: string, start: u32, end: u32}, options?: MethodOptions) => Promise<AssembledTransaction<Array<Buffer>>>

  /**
   * Construct and simulate a get_funding_info transaction. Returns an `AssembledTransaction` object which will have a `result` field containing the result of the simulation. If this transaction changes contract state, you will need to call `signAndSend()` on the returned object.
   * Get the aggregate funding state for a market.
   */
  get_funding_info: ({data_store, market_token}: {data_store: string, market_token: string}, options?: MethodOptions) => Promise<AssembledTransaction<FundingInfo>>

  /**
   * Construct and simulate a get_deposit_count transaction. Returns an `AssembledTransaction` object which will have a `result` field containing the result of the simulation. If this transaction changes contract state, you will need to call `signAndSend()` on the returned object.
   * Count of all pending deposit keys in DataStore.
   */
  get_deposit_count: ({data_store}: {data_store: string}, options?: MethodOptions) => Promise<AssembledTransaction<u32>>

  /**
   * Construct and simulate a get_open_interest transaction. Returns an `AssembledTransaction` object which will have a `result` field containing the result of the simulation. If this transaction changes contract state, you will need to call `signAndSend()` on the returned object.
   * Get open interest for both sides of a market.
   * Returns (long_oi_usd, short_oi_usd).
   */
  get_open_interest: ({data_store, market_token}: {data_store: string, market_token: string}, options?: MethodOptions) => Promise<AssembledTransaction<readonly [i128, i128]>>

  /**
   * Construct and simulate a get_position_info transaction. Returns an `AssembledTransaction` object which will have a `result` field containing the result of the simulation. If this transaction changes contract state, you will need to call `signAndSend()` on the returned object.
   * Get a single position enriched with PnL, fees, and liquidation price.
   * 
   * Reads position from the canonical location (order_handler storage) via cross-contract call.
   * This ensures all consumers (liquidation_handler, adl_handler, reader) agree on position state.
   */
  get_position_info: ({data_store, oracle, order_handler, account, market, collateral_token, is_long}: {data_store: string, oracle: string, order_handler: string, account: string, market: string, collateral_token: string, is_long: boolean}, options?: MethodOptions) => Promise<AssembledTransaction<Option<PositionInfo>>>

  /**
   * Construct and simulate a get_account_orders transaction. Returns an `AssembledTransaction` object which will have a `result` field containing the result of the simulation. If this transaction changes contract state, you will need to call `signAndSend()` on the returned object.
   * Return paginated orders for an account.
   */
  get_account_orders: ({data_store, order_handler, account, page, page_size}: {data_store: string, order_handler: string, account: string, page: u32, page_size: u32}, options?: MethodOptions) => Promise<AssembledTransaction<Array<OrderProps>>>

  /**
   * Construct and simulate a get_pending_orders transaction. Returns an `AssembledTransaction` object which will have a `result` field containing the result of the simulation. If this transaction changes contract state, you will need to call `signAndSend()` on the returned object.
   * Return paginated pending orders for a given market.
   */
  get_pending_orders: ({data_store, order_handler, market, offset, limit}: {data_store: string, order_handler: string, market: string, offset: u32, limit: u32}, options?: MethodOptions) => Promise<AssembledTransaction<Array<PendingOrder>>>

  /**
   * Construct and simulate a get_protocol_stats transaction. Returns an `AssembledTransaction` object which will have a `result` field containing the result of the simulation. If this transaction changes contract state, you will need to call `signAndSend()` on the returned object.
   * View-only: reads `data_store` and `oracle`, writes nothing.
   * 
   * Panics with `TooManyMarkets` if `markets.len() > MAX_STATS_MARKETS`, which
   * bounds the per-call compute cost. An empty `markets` vec is valid and
   * returns all-zero stats (with `market_count = 0`). A market whose pool value
   * is zero simply contributes zero — no special-casing, no panic.
   */
  get_protocol_stats: ({data_store, oracle, markets}: {data_store: string, oracle: string, markets: Array<string>}, options?: MethodOptions) => Promise<AssembledTransaction<ProtocolStats>>

  /**
   * Construct and simulate a get_withdrawal_keys transaction. Returns an `AssembledTransaction` object which will have a `result` field containing the result of the simulation. If this transaction changes contract state, you will need to call `signAndSend()` on the returned object.
   * Paginated list of all withdrawal keys (raw BytesN<32>).
   */
  get_withdrawal_keys: ({data_store, start, end}: {data_store: string, start: u32, end: u32}, options?: MethodOptions) => Promise<AssembledTransaction<Array<Buffer>>>

  /**
   * Construct and simulate a estimate_swap_output transaction. Returns an `AssembledTransaction` object which will have a `result` field containing the result of the simulation. If this transaction changes contract state, you will need to call `signAndSend()` on the returned object.
   * Estimate the output of a swap without modifying state.
   * 
   * Returns the estimated output token amount, cumulative price impact, and
   * whether execution would likely revert due to paused markets or insufficient liquidity.
   * 
   * This is a read-only view that mirrors swap execution logic for frontend preview.
   */
  estimate_swap_output: ({data_store, oracle, token_in, amount_in, swap_path}: {data_store: string, oracle: string, token_in: string, amount_in: u128, swap_path: Array<string>}, options?: MethodOptions) => Promise<AssembledTransaction<SwapEstimate>>

  /**
   * Construct and simulate a get_withdrawal_count transaction. Returns an `AssembledTransaction` object which will have a `result` field containing the result of the simulation. If this transaction changes contract state, you will need to call `signAndSend()` on the returned object.
   * Count of all pending withdrawal keys in DataStore.
   */
  get_withdrawal_count: ({data_store}: {data_store: string}, options?: MethodOptions) => Promise<AssembledTransaction<u32>>

  /**
   * Construct and simulate a get_account_positions transaction. Returns an `AssembledTransaction` object which will have a `result` field containing the result of the simulation. If this transaction changes contract state, you will need to call `signAndSend()` on the returned object.
   * Return paginated account positions as enriched `PositionInfo` entries.
   */
  get_account_positions: ({data_store, oracle, order_handler, account, page, page_size}: {data_store: string, oracle: string, order_handler: string, account: string, page: u32, page_size: u32}, options?: MethodOptions) => Promise<AssembledTransaction<Array<PositionInfo>>>

  /**
   * Construct and simulate a get_funding_rate_info transaction. Returns an `AssembledTransaction` object which will have a `result` field containing the result of the simulation. If this transaction changes contract state, you will need to call `signAndSend()` on the returned object.
   * Issue #207: per-hour funding rate view for the frontend.
   * 
   * For **historical** funding rates, use the off-chain event indexer: `execute_order`
   * emits a `FundingRateSnapshot` Soroban event (topic: `"fund_snap"`) after every
   * position order execution. Filter by topic and market address to reconstruct the
   * funding rate time-series. Historical data is not stored on-chain to avoid
   * Soroban storage cost at every position execution (issue #286).
   */
  get_funding_rate_info: ({data_store, market_token}: {data_store: string, market_token: string}, options?: MethodOptions) => Promise<AssembledTransaction<FundingRateInfo>>

  /**
   * Construct and simulate a get_position_leverage transaction. Returns an `AssembledTransaction` object which will have a `result` field containing the result of the simulation. If this transaction changes contract state, you will need to call `signAndSend()` on the returned object.
   * Compute effective leverage for an open position at current oracle prices (issue #218).
   * 
   * Net collateral = gross collateral − pending borrowing fee − pending funding fee.
   * Returns `None` when the position key does not exist.
   * Returns `effective_leverage_x100 = u32::MAX` when net collateral has been fully
   * consumed by fees (net ≤ 0), signalling imminent liquidation.
   */
  get_position_leverage: ({data_store, oracle, order_handler, position_key}: {data_store: string, oracle: string, order_handler: string, position_key: Buffer}, options?: MethodOptions) => Promise<AssembledTransaction<Option<PositionLeverage>>>

  /**
   * Construct and simulate a check_keeper_heartbeat transaction. Returns an `AssembledTransaction` object which will have a `result` field containing the result of the simulation. If this transaction changes contract state, you will need to call `signAndSend()` on the returned object.
   * Read a keeper role's liveness status from data_store (issue #249).
   * 
   * View-only mirror of the heartbeat check so the frontend / monitoring can
   * surface stale keepers without calling into order_handler. Returns the
   * last-active ledger, the gap since then, and whether that gap exceeds the
   * role's configured heartbeat timeout (falling back to the 2880-ledger
   * default when unset). A role with no recorded activity reports
   * `last_active_ledger = 0` and is treated as stale.
   */
  check_keeper_heartbeat: ({data_store, role}: {data_store: string, role: Buffer}, options?: MethodOptions) => Promise<AssembledTransaction<KeeperHeartbeatStatus>>

  /**
   * Construct and simulate a get_account_order_keys transaction. Returns an `AssembledTransaction` object which will have a `result` field containing the result of the simulation. If this transaction changes contract state, you will need to call `signAndSend()` on the returned object.
   * Paginated list of order keys for a specific account.
   */
  get_account_order_keys: ({data_store, account, start, end}: {data_store: string, account: string, start: u32, end: u32}, options?: MethodOptions) => Promise<AssembledTransaction<Array<Buffer>>>

  /**
   * Construct and simulate a get_market_token_price transaction. Returns an `AssembledTransaction` object which will have a `result` field containing the result of the simulation. If this transaction changes contract state, you will need to call `signAndSend()` on the returned object.
   * Issue #276: USD price per 1 market (GM) token, FLOAT_PRECISION scaled.
   * `maximize = true` uses oracle max prices (conservative for minting);
   * `maximize = false` uses oracle min prices (conservative for burning).
   * Zero-supply pools return a 1.00 USD seed price rather than panicking.
   */
  get_market_token_price: ({data_store, oracle, market, maximize}: {data_store: string, oracle: string, market: string, maximize: boolean}, options?: MethodOptions) => Promise<AssembledTransaction<u128>>

  /**
   * Construct and simulate a get_account_order_count transaction. Returns an `AssembledTransaction` object which will have a `result` field containing the result of the simulation. If this transaction changes contract state, you will need to call `signAndSend()` on the returned object.
   * Count of pending order keys for a specific account.
   */
  get_account_order_count: ({data_store, account}: {data_store: string, account: string}, options?: MethodOptions) => Promise<AssembledTransaction<u32>>

  /**
   * Construct and simulate a get_account_deposit_keys transaction. Returns an `AssembledTransaction` object which will have a `result` field containing the result of the simulation. If this transaction changes contract state, you will need to call `signAndSend()` on the returned object.
   * Paginated list of deposit keys for a specific account.
   */
  get_account_deposit_keys: ({data_store, account, start, end}: {data_store: string, account: string, start: u32, end: u32}, options?: MethodOptions) => Promise<AssembledTransaction<Array<Buffer>>>

  /**
   * Construct and simulate a get_position_info_by_key transaction. Returns an `AssembledTransaction` object which will have a `result` field containing the result of the simulation. If this transaction changes contract state, you will need to call `signAndSend()` on the returned object.
   * Get position info by canonical position key (BytesN<32>), returning enriched `PositionInfo`.
   */
  get_position_info_by_key: ({data_store, oracle, order_handler, position_key}: {data_store: string, oracle: string, order_handler: string, position_key: Buffer}, options?: MethodOptions) => Promise<AssembledTransaction<Option<PositionInfo>>>

  /**
   * Construct and simulate a is_position_liquidatable transaction. Returns an `AssembledTransaction` object which will have a `result` field containing the result of the simulation. If this transaction changes contract state, you will need to call `signAndSend()` on the returned object.
   * Return whether a position is currently liquidatable at oracle prices.
   * 
   * Reads position from the canonical location (order_handler storage) via cross-contract call.
   */
  is_position_liquidatable: ({data_store, oracle, order_handler, account, market, collateral_token, is_long}: {data_store: string, oracle: string, order_handler: string, account: string, market: string, collateral_token: string, is_long: boolean}, options?: MethodOptions) => Promise<AssembledTransaction<boolean>>

  /**
   * Construct and simulate a get_account_deposit_count transaction. Returns an `AssembledTransaction` object which will have a `result` field containing the result of the simulation. If this transaction changes contract state, you will need to call `signAndSend()` on the returned object.
   * Count of pending deposit keys for a specific account.
   */
  get_account_deposit_count: ({data_store, account}: {data_store: string, account: string}, options?: MethodOptions) => Promise<AssembledTransaction<u32>>

  /**
   * Construct and simulate a get_adl_eligible_positions transaction. Returns an `AssembledTransaction` object which will have a `result` field containing the result of the simulation. If this transaction changes contract state, you will need to call `signAndSend()` on the returned object.
   * Get all profitable positions eligible for auto-deleveraging on a market side.
   * 
   * Returns only positions with positive unrealised PnL, sorted by profitability ratio
   * (highest first). Use `limit` to bound iteration cost; keepers call multiple times
   * if many positions qualify.
   */
  get_adl_eligible_positions: ({data_store, oracle, order_handler, market, is_long, limit}: {data_store: string, oracle: string, order_handler: string, market: string, is_long: boolean, limit: u32}, options?: MethodOptions) => Promise<AssembledTransaction<Array<AdlCandidate>>>

  /**
   * Construct and simulate a get_liquidatable_positions transaction. Returns an `AssembledTransaction` object which will have a `result` field containing the result of the simulation. If this transaction changes contract state, you will need to call `signAndSend()` on the returned object.
   * Return positions currently eligible for liquidation on a market side, sorted by
   * health_factor_bps ascending (most under-collateralised first).
   * 
   * `health_factor_bps = collateral_usd * 10000 / size_usd` — any value below
   * 10000 means the position is below the minimum-collateral threshold.
   * Positions where `is_position_liquidatable` returns false are excluded.
   * 
   * Pagination: the `offset` first matching entries are skipped; at most `limit`
   * entries are returned. No state is written.
   */
  get_liquidatable_positions: ({data_store, oracle, order_handler, market, is_long, limit, offset}: {data_store: string, oracle: string, order_handler: string, market: string, is_long: boolean, limit: u32, offset: u32}, options?: MethodOptions) => Promise<AssembledTransaction<Array<LiquidatablePosition>>>

  /**
   * Construct and simulate a get_market_pool_value_info transaction. Returns an `AssembledTransaction` object which will have a `result` field containing the result of the simulation. If this transaction changes contract state, you will need to call `signAndSend()` on the returned object.
   * Get the full pool value breakdown for a market at current oracle prices.
   */
  get_market_pool_value_info: ({data_store, oracle, market_token, maximize}: {data_store: string, oracle: string, market_token: string, maximize: boolean}, options?: MethodOptions) => Promise<AssembledTransaction<PoolValueInfo>>

  /**
   * Construct and simulate a get_account_withdrawal_keys transaction. Returns an `AssembledTransaction` object which will have a `result` field containing the result of the simulation. If this transaction changes contract state, you will need to call `signAndSend()` on the returned object.
   * Paginated list of withdrawal keys for a specific account.
   */
  get_account_withdrawal_keys: ({data_store, account, start, end}: {data_store: string, account: string, start: u32, end: u32}, options?: MethodOptions) => Promise<AssembledTransaction<Array<Buffer>>>

  /**
   * Construct and simulate a get_execution_price_preview transaction. Returns an `AssembledTransaction` object which will have a `result` field containing the result of the simulation. If this transaction changes contract state, you will need to call `signAndSend()` on the returned object.
   * Compute the execution price a user would get for a given size and order direction.
   * 
   * Useful for the UI to preview slippage before placing an order.
   */
  get_execution_price_preview: ({data_store, oracle, market_token, is_long, is_increase, size_delta_usd}: {data_store: string, oracle: string, market_token: string, is_long: boolean, is_increase: boolean, size_delta_usd: i128}, options?: MethodOptions) => Promise<AssembledTransaction<i128>>

  /**
   * Construct and simulate a get_account_withdrawal_count transaction. Returns an `AssembledTransaction` object which will have a `result` field containing the result of the simulation. If this transaction changes contract state, you will need to call `signAndSend()` on the returned object.
   * Count of pending withdrawal keys for a specific account.
   */
  get_account_withdrawal_count: ({data_store, account}: {data_store: string, account: string}, options?: MethodOptions) => Promise<AssembledTransaction<u32>>

  /**
   * Construct and simulate a get_claimable_funding_amount transaction. Returns an `AssembledTransaction` object which will have a `result` field containing the result of the simulation. If this transaction changes contract state, you will need to call `signAndSend()` on the returned object.
   * Issue #275: read-only view of a position's pending (not-yet-settled) funding,
   * without mutating any state. Mirrors the exact math `settle_funding_fees`
   * (claimable side) and `get_position_fees` (owed side) apply during a real
   * decrease, so the returned amounts match what would actually be
   * credited/debited if the position were decreased right now.
   */
  get_claimable_funding_amount: ({data_store, oracle, order_handler, account, market, collateral_token, is_long}: {data_store: string, oracle: string, order_handler: string, account: string, market: string, collateral_token: string, is_long: boolean}, options?: MethodOptions) => Promise<AssembledTransaction<Option<FundingAmountResult>>>

}
export class Client extends ContractClient {
  static async deploy<T = Client>(
    /** Options for initializing a Client as well as for calling a method, with extras specific to deploying. */
    options: MethodOptions &
      Omit<ContractClientOptions, "contractId"> & {
        /** The hash of the Wasm blob, which must already be installed on-chain. */
        wasmHash: Buffer | string;
        /** Salt used to generate the contract's ID. Passed through to {@link Operation.createCustomContract}. Default: random. */
        salt?: Buffer | Uint8Array;
        /** The format used to decode `wasmHash`, if it's provided as a string. */
        format?: "hex" | "base64";
      }
  ): Promise<AssembledTransaction<T>> {
    return ContractClient.deploy(null, options)
  }
  constructor(public readonly options: ContractClientOptions) {
    super(
      new ContractSpec([ "AAAABAAAAAAAAAAAAAAABUVycm9yAAAAAAAABAAAAAAAAAAOTm90SW5pdGlhbGl6ZWQAAAAAAAEAAAAAAAAAEkFscmVhZHlJbml0aWFsaXplZAAAAAAAAgAAAEZgZ2V0X3Byb3RvY29sX3N0YXRzYCB3YXMgcGFzc2VkIG1vcmUgdGhhbiBgTUFYX1NUQVRTX01BUktFVFNgIG1hcmtldHMuAAAAAAAOVG9vTWFueU1hcmtldHMAAAAAAAQAAAE2VGhlIHN1cHBsaWVkIG1hcmtldCBhZGRyZXNzIGlzIG5vdCByZWdpc3RlcmVkIGluIGRhdGFfc3RvcmUg4oCUCm9uZSBvciBtb3JlIG9mIGl0cyB0b2tlbiBhZGRyZXNzZXMgKGluZGV4L2xvbmcvc2hvcnQpIHJldHVybmVkIE5vbmUuCk1pcnJvcnMgdGhlIEludmFsaWRNYXJrZXQgcGF0dGVybiBpbiBkZXBvc2l0X2hhbmRsZXIvd2l0aGRyYXdhbF9oYW5kbGVyCihpc3N1ZSAjMzcxKSBzbyBjYWxsZXJzIGNhbiBtYXRjaCB0aGlzIGNvbmRpdGlvbiBhcyBhIHR5cGVkIGVycm9yCmluc3RlYWQgb2YgYSBnZW5lcmljIGV4ZWN1dGlvbiBmYWlsdXJlLgAAAAAADUludmFsaWRNYXJrZXQAAAAAAAAF",
        "AAAAAAAAAD9VcGdyYWRlIHRoZSBjb250cmFjdCB3YXNtLiBPbmx5IHRoZSBzdG9yZWQgYWRtaW4gbWF5IGNhbGwgdGhpcy4AAAAAB3VwZ3JhZGUAAAAAAQAAAAAAAAANbmV3X3dhc21faGFzaAAAAAAAA+4AAAAgAAAAAA==",
        "AAAAAAAAADNSZXR1cm4gYSBzdG9yZWQgb3JkZXIgYnkga2V5LCBvciBOb25lIGlmIG5vdCBmb3VuZC4AAAAACWdldF9vcmRlcgAAAAAAAAIAAAAAAAAADW9yZGVyX2hhbmRsZXIAAAAAAAATAAAAAAAAAANrZXkAAAAD7gAAACAAAAABAAAD6AAAB9AAAAAKT3JkZXJQcm9wcwAA",
        "AAAAAAAAAEdMb2FkIGZ1bGwgTWFya2V0UHJvcHMgZm9yIGEgZ2l2ZW4gbWFya2V0X3Rva2VuIGFkZHJlc3MgZnJvbSBkYXRhX3N0b3JlLgAAAAAKZ2V0X21hcmtldAAAAAAAAgAAAAAAAAAKZGF0YV9zdG9yZQAAAAAAEwAAAAAAAAAMbWFya2V0X3Rva2VuAAAAEwAAAAEAAAfQAAAAC01hcmtldFByb3BzAA==",
        "AAAAAAAAACtPbmUtdGltZSBzZXR1cCDigJQgc3RvcmUgdGhlIGFkbWluIGFkZHJlc3MuAAAAAAppbml0aWFsaXplAAAAAAABAAAAAAAAAAVhZG1pbgAAAAAAABMAAAAA",
        "AAAAAAAAADRHZXQgYSBkZXBvc2l0IGJ5IGtleSAoZGVsZWdhdGVzIHRvIGRlcG9zaXRfaGFuZGxlcikuAAAAC2dldF9kZXBvc2l0AAAAAAIAAAAAAAAAD2RlcG9zaXRfaGFuZGxlcgAAAAATAAAAAAAAAANrZXkAAAAD7gAAACAAAAABAAAD6AAAB9AAAAAMRGVwb3NpdFByb3Bz",
        "AAAAAAAAADJQYWdpbmF0ZWQgbGlzdCBvZiBhbGwgb3JkZXIga2V5cyAocmF3IEJ5dGVzTjwzMj4pLgAAAAAADmdldF9vcmRlcl9rZXlzAAAAAAADAAAAAAAAAApkYXRhX3N0b3JlAAAAAAATAAAAAAAAAAVzdGFydAAAAAAAAAQAAAAAAAAAA2VuZAAAAAAEAAAAAQAAA+oAAAPuAAAAIA==",
        "AAAAAAAAADpHZXQgYSB3aXRoZHJhd2FsIGJ5IGtleSAoZGVsZWdhdGVzIHRvIHdpdGhkcmF3YWxfaGFuZGxlcikuAAAAAAAOZ2V0X3dpdGhkcmF3YWwAAAAAAAIAAAAAAAAAEndpdGhkcmF3YWxfaGFuZGxlcgAAAAAAEwAAAAAAAAADa2V5AAAAA+4AAAAgAAAAAQAAA+gAAAfQAAAAD1dpdGhkcmF3YWxQcm9wcwA=",
        "AAAAAAAAAC1Db3VudCBvZiBhbGwgcGVuZGluZyBvcmRlciBrZXlzIGluIERhdGFTdG9yZS4AAAAAAAAPZ2V0X29yZGVyX2NvdW50AAAAAAEAAAAAAAAACmRhdGFfc3RvcmUAAAAAABMAAAABAAAABA==",
        "AAAAAAAAADRQYWdpbmF0ZWQgbGlzdCBvZiBhbGwgZGVwb3NpdCBrZXlzIChyYXcgQnl0ZXNOPDMyPikuAAAAEGdldF9kZXBvc2l0X2tleXMAAAADAAAAAAAAAApkYXRhX3N0b3JlAAAAAAATAAAAAAAAAAVzdGFydAAAAAAAAAQAAAAAAAAAA2VuZAAAAAAEAAAAAQAAA+oAAAPuAAAAIA==",
        "AAAAAAAAAC1HZXQgdGhlIGFnZ3JlZ2F0ZSBmdW5kaW5nIHN0YXRlIGZvciBhIG1hcmtldC4AAAAAAAAQZ2V0X2Z1bmRpbmdfaW5mbwAAAAIAAAAAAAAACmRhdGFfc3RvcmUAAAAAABMAAAAAAAAADG1hcmtldF90b2tlbgAAABMAAAABAAAH0AAAAAtGdW5kaW5nSW5mbwA=",
        "AAAAAAAAAC9Db3VudCBvZiBhbGwgcGVuZGluZyBkZXBvc2l0IGtleXMgaW4gRGF0YVN0b3JlLgAAAAARZ2V0X2RlcG9zaXRfY291bnQAAAAAAAABAAAAAAAAAApkYXRhX3N0b3JlAAAAAAATAAAAAQAAAAQ=",
        "AAAAAAAAAFJHZXQgb3BlbiBpbnRlcmVzdCBmb3IgYm90aCBzaWRlcyBvZiBhIG1hcmtldC4KUmV0dXJucyAobG9uZ19vaV91c2QsIHNob3J0X29pX3VzZCkuAAAAAAARZ2V0X29wZW5faW50ZXJlc3QAAAAAAAACAAAAAAAAAApkYXRhX3N0b3JlAAAAAAATAAAAAAAAAAxtYXJrZXRfdG9rZW4AAAATAAAAAQAAA+0AAAACAAAACwAAAAs=",
        "AAAAAAAAAQFHZXQgYSBzaW5nbGUgcG9zaXRpb24gZW5yaWNoZWQgd2l0aCBQbkwsIGZlZXMsIGFuZCBsaXF1aWRhdGlvbiBwcmljZS4KClJlYWRzIHBvc2l0aW9uIGZyb20gdGhlIGNhbm9uaWNhbCBsb2NhdGlvbiAob3JkZXJfaGFuZGxlciBzdG9yYWdlKSB2aWEgY3Jvc3MtY29udHJhY3QgY2FsbC4KVGhpcyBlbnN1cmVzIGFsbCBjb25zdW1lcnMgKGxpcXVpZGF0aW9uX2hhbmRsZXIsIGFkbF9oYW5kbGVyLCByZWFkZXIpIGFncmVlIG9uIHBvc2l0aW9uIHN0YXRlLgAAAAAAABFnZXRfcG9zaXRpb25faW5mbwAAAAAAAAcAAAAAAAAACmRhdGFfc3RvcmUAAAAAABMAAAAAAAAABm9yYWNsZQAAAAAAEwAAAAAAAAANb3JkZXJfaGFuZGxlcgAAAAAAABMAAAAAAAAAB2FjY291bnQAAAAAEwAAAAAAAAAGbWFya2V0AAAAAAATAAAAAAAAABBjb2xsYXRlcmFsX3Rva2VuAAAAEwAAAAAAAAAHaXNfbG9uZwAAAAABAAAAAQAAA+gAAAfQAAAADFBvc2l0aW9uSW5mbw==",
        "AAAAAAAAACdSZXR1cm4gcGFnaW5hdGVkIG9yZGVycyBmb3IgYW4gYWNjb3VudC4AAAAAEmdldF9hY2NvdW50X29yZGVycwAAAAAABQAAAAAAAAAKZGF0YV9zdG9yZQAAAAAAEwAAAAAAAAANb3JkZXJfaGFuZGxlcgAAAAAAABMAAAAAAAAAB2FjY291bnQAAAAAEwAAAAAAAAAEcGFnZQAAAAQAAAAAAAAACXBhZ2Vfc2l6ZQAAAAAAAAQAAAABAAAD6gAAB9AAAAAKT3JkZXJQcm9wcwAA",
        "AAAAAAAAADNSZXR1cm4gcGFnaW5hdGVkIHBlbmRpbmcgb3JkZXJzIGZvciBhIGdpdmVuIG1hcmtldC4AAAAAEmdldF9wZW5kaW5nX29yZGVycwAAAAAABQAAAAAAAAAKZGF0YV9zdG9yZQAAAAAAEwAAAAAAAAANb3JkZXJfaGFuZGxlcgAAAAAAABMAAAAAAAAABm1hcmtldAAAAAAAEwAAAAAAAAAGb2Zmc2V0AAAAAAAEAAAAAAAAAAVsaW1pdAAAAAAAAAQAAAABAAAD6gAAB9AAAAAMUGVuZGluZ09yZGVy",
        "AAAAAAAAAVpWaWV3LW9ubHk6IHJlYWRzIGBkYXRhX3N0b3JlYCBhbmQgYG9yYWNsZWAsIHdyaXRlcyBub3RoaW5nLgoKUGFuaWNzIHdpdGggYFRvb01hbnlNYXJrZXRzYCBpZiBgbWFya2V0cy5sZW4oKSA+IE1BWF9TVEFUU19NQVJLRVRTYCwgd2hpY2gKYm91bmRzIHRoZSBwZXItY2FsbCBjb21wdXRlIGNvc3QuIEFuIGVtcHR5IGBtYXJrZXRzYCB2ZWMgaXMgdmFsaWQgYW5kCnJldHVybnMgYWxsLXplcm8gc3RhdHMgKHdpdGggYG1hcmtldF9jb3VudCA9IDBgKS4gQSBtYXJrZXQgd2hvc2UgcG9vbCB2YWx1ZQppcyB6ZXJvIHNpbXBseSBjb250cmlidXRlcyB6ZXJvIOKAlCBubyBzcGVjaWFsLWNhc2luZywgbm8gcGFuaWMuAAAAAAASZ2V0X3Byb3RvY29sX3N0YXRzAAAAAAADAAAAAAAAAApkYXRhX3N0b3JlAAAAAAATAAAAAAAAAAZvcmFjbGUAAAAAABMAAAAAAAAAB21hcmtldHMAAAAD6gAAABMAAAABAAAH0AAAAA1Qcm90b2NvbFN0YXRzAAAA",
        "AAAAAAAAADdQYWdpbmF0ZWQgbGlzdCBvZiBhbGwgd2l0aGRyYXdhbCBrZXlzIChyYXcgQnl0ZXNOPDMyPikuAAAAABNnZXRfd2l0aGRyYXdhbF9rZXlzAAAAAAMAAAAAAAAACmRhdGFfc3RvcmUAAAAAABMAAAAAAAAABXN0YXJ0AAAAAAAABAAAAAAAAAADZW5kAAAAAAQAAAABAAAD6gAAA+4AAAAg",
        "AAAAAAAAAShFc3RpbWF0ZSB0aGUgb3V0cHV0IG9mIGEgc3dhcCB3aXRob3V0IG1vZGlmeWluZyBzdGF0ZS4KClJldHVybnMgdGhlIGVzdGltYXRlZCBvdXRwdXQgdG9rZW4gYW1vdW50LCBjdW11bGF0aXZlIHByaWNlIGltcGFjdCwgYW5kCndoZXRoZXIgZXhlY3V0aW9uIHdvdWxkIGxpa2VseSByZXZlcnQgZHVlIHRvIHBhdXNlZCBtYXJrZXRzIG9yIGluc3VmZmljaWVudCBsaXF1aWRpdHkuCgpUaGlzIGlzIGEgcmVhZC1vbmx5IHZpZXcgdGhhdCBtaXJyb3JzIHN3YXAgZXhlY3V0aW9uIGxvZ2ljIGZvciBmcm9udGVuZCBwcmV2aWV3LgAAABRlc3RpbWF0ZV9zd2FwX291dHB1dAAAAAUAAAAAAAAACmRhdGFfc3RvcmUAAAAAABMAAAAAAAAABm9yYWNsZQAAAAAAEwAAAAAAAAAIdG9rZW5faW4AAAATAAAAAAAAAAlhbW91bnRfaW4AAAAAAAAKAAAAAAAAAAlzd2FwX3BhdGgAAAAAAAPqAAAAEwAAAAEAAAfQAAAADFN3YXBFc3RpbWF0ZQ==",
        "AAAAAAAAADJDb3VudCBvZiBhbGwgcGVuZGluZyB3aXRoZHJhd2FsIGtleXMgaW4gRGF0YVN0b3JlLgAAAAAAFGdldF93aXRoZHJhd2FsX2NvdW50AAAAAQAAAAAAAAAKZGF0YV9zdG9yZQAAAAAAEwAAAAEAAAAE",
        "AAAAAAAAAEZSZXR1cm4gcGFnaW5hdGVkIGFjY291bnQgcG9zaXRpb25zIGFzIGVucmljaGVkIGBQb3NpdGlvbkluZm9gIGVudHJpZXMuAAAAAAAVZ2V0X2FjY291bnRfcG9zaXRpb25zAAAAAAAABgAAAAAAAAAKZGF0YV9zdG9yZQAAAAAAEwAAAAAAAAAGb3JhY2xlAAAAAAATAAAAAAAAAA1vcmRlcl9oYW5kbGVyAAAAAAAAEwAAAAAAAAAHYWNjb3VudAAAAAATAAAAAAAAAARwYWdlAAAABAAAAAAAAAAJcGFnZV9zaXplAAAAAAAABAAAAAEAAAPqAAAH0AAAAAxQb3NpdGlvbkluZm8=",
        "AAAAAAAAAbRJc3N1ZSAjMjA3OiBwZXItaG91ciBmdW5kaW5nIHJhdGUgdmlldyBmb3IgdGhlIGZyb250ZW5kLgoKRm9yICoqaGlzdG9yaWNhbCoqIGZ1bmRpbmcgcmF0ZXMsIHVzZSB0aGUgb2ZmLWNoYWluIGV2ZW50IGluZGV4ZXI6IGBleGVjdXRlX29yZGVyYAplbWl0cyBhIGBGdW5kaW5nUmF0ZVNuYXBzaG90YCBTb3JvYmFuIGV2ZW50ICh0b3BpYzogYCJmdW5kX3NuYXAiYCkgYWZ0ZXIgZXZlcnkKcG9zaXRpb24gb3JkZXIgZXhlY3V0aW9uLiBGaWx0ZXIgYnkgdG9waWMgYW5kIG1hcmtldCBhZGRyZXNzIHRvIHJlY29uc3RydWN0IHRoZQpmdW5kaW5nIHJhdGUgdGltZS1zZXJpZXMuIEhpc3RvcmljYWwgZGF0YSBpcyBub3Qgc3RvcmVkIG9uLWNoYWluIHRvIGF2b2lkClNvcm9iYW4gc3RvcmFnZSBjb3N0IGF0IGV2ZXJ5IHBvc2l0aW9uIGV4ZWN1dGlvbiAoaXNzdWUgIzI4NikuAAAAFWdldF9mdW5kaW5nX3JhdGVfaW5mbwAAAAAAAAIAAAAAAAAACmRhdGFfc3RvcmUAAAAAABMAAAAAAAAADG1hcmtldF90b2tlbgAAABMAAAABAAAH0AAAAA9GdW5kaW5nUmF0ZUluZm8A",
        "AAAAAAAAAXBDb21wdXRlIGVmZmVjdGl2ZSBsZXZlcmFnZSBmb3IgYW4gb3BlbiBwb3NpdGlvbiBhdCBjdXJyZW50IG9yYWNsZSBwcmljZXMgKGlzc3VlICMyMTgpLgoKTmV0IGNvbGxhdGVyYWwgPSBncm9zcyBjb2xsYXRlcmFsIOKIkiBwZW5kaW5nIGJvcnJvd2luZyBmZWUg4oiSIHBlbmRpbmcgZnVuZGluZyBmZWUuClJldHVybnMgYE5vbmVgIHdoZW4gdGhlIHBvc2l0aW9uIGtleSBkb2VzIG5vdCBleGlzdC4KUmV0dXJucyBgZWZmZWN0aXZlX2xldmVyYWdlX3gxMDAgPSB1MzI6Ok1BWGAgd2hlbiBuZXQgY29sbGF0ZXJhbCBoYXMgYmVlbiBmdWxseQpjb25zdW1lZCBieSBmZWVzIChuZXQg4omkIDApLCBzaWduYWxsaW5nIGltbWluZW50IGxpcXVpZGF0aW9uLgAAABVnZXRfcG9zaXRpb25fbGV2ZXJhZ2UAAAAAAAAEAAAAAAAAAApkYXRhX3N0b3JlAAAAAAATAAAAAAAAAAZvcmFjbGUAAAAAABMAAAAAAAAADW9yZGVyX2hhbmRsZXIAAAAAAAATAAAAAAAAAAxwb3NpdGlvbl9rZXkAAAPuAAAAIAAAAAEAAAPoAAAH0AAAABBQb3NpdGlvbkxldmVyYWdl",
        "AAAAAAAAAdBSZWFkIGEga2VlcGVyIHJvbGUncyBsaXZlbmVzcyBzdGF0dXMgZnJvbSBkYXRhX3N0b3JlIChpc3N1ZSAjMjQ5KS4KClZpZXctb25seSBtaXJyb3Igb2YgdGhlIGhlYXJ0YmVhdCBjaGVjayBzbyB0aGUgZnJvbnRlbmQgLyBtb25pdG9yaW5nIGNhbgpzdXJmYWNlIHN0YWxlIGtlZXBlcnMgd2l0aG91dCBjYWxsaW5nIGludG8gb3JkZXJfaGFuZGxlci4gUmV0dXJucyB0aGUKbGFzdC1hY3RpdmUgbGVkZ2VyLCB0aGUgZ2FwIHNpbmNlIHRoZW4sIGFuZCB3aGV0aGVyIHRoYXQgZ2FwIGV4Y2VlZHMgdGhlCnJvbGUncyBjb25maWd1cmVkIGhlYXJ0YmVhdCB0aW1lb3V0IChmYWxsaW5nIGJhY2sgdG8gdGhlIDI4ODAtbGVkZ2VyCmRlZmF1bHQgd2hlbiB1bnNldCkuIEEgcm9sZSB3aXRoIG5vIHJlY29yZGVkIGFjdGl2aXR5IHJlcG9ydHMKYGxhc3RfYWN0aXZlX2xlZGdlciA9IDBgIGFuZCBpcyB0cmVhdGVkIGFzIHN0YWxlLgAAABZjaGVja19rZWVwZXJfaGVhcnRiZWF0AAAAAAACAAAAAAAAAApkYXRhX3N0b3JlAAAAAAATAAAAAAAAAARyb2xlAAAD7gAAACAAAAABAAAH0AAAABVLZWVwZXJIZWFydGJlYXRTdGF0dXMAAAA=",
        "AAAAAAAAADRQYWdpbmF0ZWQgbGlzdCBvZiBvcmRlciBrZXlzIGZvciBhIHNwZWNpZmljIGFjY291bnQuAAAAFmdldF9hY2NvdW50X29yZGVyX2tleXMAAAAAAAQAAAAAAAAACmRhdGFfc3RvcmUAAAAAABMAAAAAAAAAB2FjY291bnQAAAAAEwAAAAAAAAAFc3RhcnQAAAAAAAAEAAAAAAAAAANlbmQAAAAABAAAAAEAAAPqAAAD7gAAACA=",
        "AAAAAAAAARdJc3N1ZSAjMjc2OiBVU0QgcHJpY2UgcGVyIDEgbWFya2V0IChHTSkgdG9rZW4sIEZMT0FUX1BSRUNJU0lPTiBzY2FsZWQuCmBtYXhpbWl6ZSA9IHRydWVgIHVzZXMgb3JhY2xlIG1heCBwcmljZXMgKGNvbnNlcnZhdGl2ZSBmb3IgbWludGluZyk7CmBtYXhpbWl6ZSA9IGZhbHNlYCB1c2VzIG9yYWNsZSBtaW4gcHJpY2VzIChjb25zZXJ2YXRpdmUgZm9yIGJ1cm5pbmcpLgpaZXJvLXN1cHBseSBwb29scyByZXR1cm4gYSAxLjAwIFVTRCBzZWVkIHByaWNlIHJhdGhlciB0aGFuIHBhbmlja2luZy4AAAAAFmdldF9tYXJrZXRfdG9rZW5fcHJpY2UAAAAAAAQAAAAAAAAACmRhdGFfc3RvcmUAAAAAABMAAAAAAAAABm9yYWNsZQAAAAAAEwAAAAAAAAAGbWFya2V0AAAAAAATAAAAAAAAAAhtYXhpbWl6ZQAAAAEAAAABAAAACg==",
        "AAAAAAAAADNDb3VudCBvZiBwZW5kaW5nIG9yZGVyIGtleXMgZm9yIGEgc3BlY2lmaWMgYWNjb3VudC4AAAAAF2dldF9hY2NvdW50X29yZGVyX2NvdW50AAAAAAIAAAAAAAAACmRhdGFfc3RvcmUAAAAAABMAAAAAAAAAB2FjY291bnQAAAAAEwAAAAEAAAAE",
        "AAAAAAAAADZQYWdpbmF0ZWQgbGlzdCBvZiBkZXBvc2l0IGtleXMgZm9yIGEgc3BlY2lmaWMgYWNjb3VudC4AAAAAABhnZXRfYWNjb3VudF9kZXBvc2l0X2tleXMAAAAEAAAAAAAAAApkYXRhX3N0b3JlAAAAAAATAAAAAAAAAAdhY2NvdW50AAAAABMAAAAAAAAABXN0YXJ0AAAAAAAABAAAAAAAAAADZW5kAAAAAAQAAAABAAAD6gAAA+4AAAAg",
        "AAAAAAAAAFxHZXQgcG9zaXRpb24gaW5mbyBieSBjYW5vbmljYWwgcG9zaXRpb24ga2V5IChCeXRlc048MzI+KSwgcmV0dXJuaW5nIGVucmljaGVkIGBQb3NpdGlvbkluZm9gLgAAABhnZXRfcG9zaXRpb25faW5mb19ieV9rZXkAAAAEAAAAAAAAAApkYXRhX3N0b3JlAAAAAAATAAAAAAAAAAZvcmFjbGUAAAAAABMAAAAAAAAADW9yZGVyX2hhbmRsZXIAAAAAAAATAAAAAAAAAAxwb3NpdGlvbl9rZXkAAAPuAAAAIAAAAAEAAAPoAAAH0AAAAAxQb3NpdGlvbkluZm8=",
        "AAAAAAAAAKJSZXR1cm4gd2hldGhlciBhIHBvc2l0aW9uIGlzIGN1cnJlbnRseSBsaXF1aWRhdGFibGUgYXQgb3JhY2xlIHByaWNlcy4KClJlYWRzIHBvc2l0aW9uIGZyb20gdGhlIGNhbm9uaWNhbCBsb2NhdGlvbiAob3JkZXJfaGFuZGxlciBzdG9yYWdlKSB2aWEgY3Jvc3MtY29udHJhY3QgY2FsbC4AAAAAABhpc19wb3NpdGlvbl9saXF1aWRhdGFibGUAAAAHAAAAAAAAAApkYXRhX3N0b3JlAAAAAAATAAAAAAAAAAZvcmFjbGUAAAAAABMAAAAAAAAADW9yZGVyX2hhbmRsZXIAAAAAAAATAAAAAAAAAAdhY2NvdW50AAAAABMAAAAAAAAABm1hcmtldAAAAAAAEwAAAAAAAAAQY29sbGF0ZXJhbF90b2tlbgAAABMAAAAAAAAAB2lzX2xvbmcAAAAAAQAAAAEAAAAB",
        "AAAAAAAAADVDb3VudCBvZiBwZW5kaW5nIGRlcG9zaXQga2V5cyBmb3IgYSBzcGVjaWZpYyBhY2NvdW50LgAAAAAAABlnZXRfYWNjb3VudF9kZXBvc2l0X2NvdW50AAAAAAAAAgAAAAAAAAAKZGF0YV9zdG9yZQAAAAAAEwAAAAAAAAAHYWNjb3VudAAAAAATAAAAAQAAAAQ=",
        "AAAAAAAAAQ5HZXQgYWxsIHByb2ZpdGFibGUgcG9zaXRpb25zIGVsaWdpYmxlIGZvciBhdXRvLWRlbGV2ZXJhZ2luZyBvbiBhIG1hcmtldCBzaWRlLgoKUmV0dXJucyBvbmx5IHBvc2l0aW9ucyB3aXRoIHBvc2l0aXZlIHVucmVhbGlzZWQgUG5MLCBzb3J0ZWQgYnkgcHJvZml0YWJpbGl0eSByYXRpbwooaGlnaGVzdCBmaXJzdCkuIFVzZSBgbGltaXRgIHRvIGJvdW5kIGl0ZXJhdGlvbiBjb3N0OyBrZWVwZXJzIGNhbGwgbXVsdGlwbGUgdGltZXMKaWYgbWFueSBwb3NpdGlvbnMgcXVhbGlmeS4AAAAAABpnZXRfYWRsX2VsaWdpYmxlX3Bvc2l0aW9ucwAAAAAABgAAAAAAAAAKZGF0YV9zdG9yZQAAAAAAEwAAAAAAAAAGb3JhY2xlAAAAAAATAAAAAAAAAA1vcmRlcl9oYW5kbGVyAAAAAAAAEwAAAAAAAAAGbWFya2V0AAAAAAATAAAAAAAAAAdpc19sb25nAAAAAAEAAAAAAAAABWxpbWl0AAAAAAAABAAAAAEAAAPqAAAH0AAAAAxBZGxDYW5kaWRhdGU=",
        "AAAAAAAAAd9SZXR1cm4gcG9zaXRpb25zIGN1cnJlbnRseSBlbGlnaWJsZSBmb3IgbGlxdWlkYXRpb24gb24gYSBtYXJrZXQgc2lkZSwgc29ydGVkIGJ5CmhlYWx0aF9mYWN0b3JfYnBzIGFzY2VuZGluZyAobW9zdCB1bmRlci1jb2xsYXRlcmFsaXNlZCBmaXJzdCkuCgpgaGVhbHRoX2ZhY3Rvcl9icHMgPSBjb2xsYXRlcmFsX3VzZCAqIDEwMDAwIC8gc2l6ZV91c2RgIOKAlCBhbnkgdmFsdWUgYmVsb3cKMTAwMDAgbWVhbnMgdGhlIHBvc2l0aW9uIGlzIGJlbG93IHRoZSBtaW5pbXVtLWNvbGxhdGVyYWwgdGhyZXNob2xkLgpQb3NpdGlvbnMgd2hlcmUgYGlzX3Bvc2l0aW9uX2xpcXVpZGF0YWJsZWAgcmV0dXJucyBmYWxzZSBhcmUgZXhjbHVkZWQuCgpQYWdpbmF0aW9uOiB0aGUgYG9mZnNldGAgZmlyc3QgbWF0Y2hpbmcgZW50cmllcyBhcmUgc2tpcHBlZDsgYXQgbW9zdCBgbGltaXRgCmVudHJpZXMgYXJlIHJldHVybmVkLiBObyBzdGF0ZSBpcyB3cml0dGVuLgAAAAAaZ2V0X2xpcXVpZGF0YWJsZV9wb3NpdGlvbnMAAAAAAAcAAAAAAAAACmRhdGFfc3RvcmUAAAAAABMAAAAAAAAABm9yYWNsZQAAAAAAEwAAAAAAAAANb3JkZXJfaGFuZGxlcgAAAAAAABMAAAAAAAAABm1hcmtldAAAAAAAEwAAAAAAAAAHaXNfbG9uZwAAAAABAAAAAAAAAAVsaW1pdAAAAAAAAAQAAAAAAAAABm9mZnNldAAAAAAABAAAAAEAAAPqAAAH0AAAABRMaXF1aWRhdGFibGVQb3NpdGlvbg==",
        "AAAAAAAAAEhHZXQgdGhlIGZ1bGwgcG9vbCB2YWx1ZSBicmVha2Rvd24gZm9yIGEgbWFya2V0IGF0IGN1cnJlbnQgb3JhY2xlIHByaWNlcy4AAAAaZ2V0X21hcmtldF9wb29sX3ZhbHVlX2luZm8AAAAAAAQAAAAAAAAACmRhdGFfc3RvcmUAAAAAABMAAAAAAAAABm9yYWNsZQAAAAAAEwAAAAAAAAAMbWFya2V0X3Rva2VuAAAAEwAAAAAAAAAIbWF4aW1pemUAAAABAAAAAQAAB9AAAAANUG9vbFZhbHVlSW5mbwAAAA==",
        "AAAAAAAAADlQYWdpbmF0ZWQgbGlzdCBvZiB3aXRoZHJhd2FsIGtleXMgZm9yIGEgc3BlY2lmaWMgYWNjb3VudC4AAAAAAAAbZ2V0X2FjY291bnRfd2l0aGRyYXdhbF9rZXlzAAAAAAQAAAAAAAAACmRhdGFfc3RvcmUAAAAAABMAAAAAAAAAB2FjY291bnQAAAAAEwAAAAAAAAAFc3RhcnQAAAAAAAAEAAAAAAAAAANlbmQAAAAABAAAAAEAAAPqAAAD7gAAACA=",
        "AAAAAAAAAJJDb21wdXRlIHRoZSBleGVjdXRpb24gcHJpY2UgYSB1c2VyIHdvdWxkIGdldCBmb3IgYSBnaXZlbiBzaXplIGFuZCBvcmRlciBkaXJlY3Rpb24uCgpVc2VmdWwgZm9yIHRoZSBVSSB0byBwcmV2aWV3IHNsaXBwYWdlIGJlZm9yZSBwbGFjaW5nIGFuIG9yZGVyLgAAAAAAG2dldF9leGVjdXRpb25fcHJpY2VfcHJldmlldwAAAAAGAAAAAAAAAApkYXRhX3N0b3JlAAAAAAATAAAAAAAAAAZvcmFjbGUAAAAAABMAAAAAAAAADG1hcmtldF90b2tlbgAAABMAAAAAAAAAB2lzX2xvbmcAAAAAAQAAAAAAAAALaXNfaW5jcmVhc2UAAAAAAQAAAAAAAAAOc2l6ZV9kZWx0YV91c2QAAAAAAAsAAAABAAAACw==",
        "AAAAAAAAADhDb3VudCBvZiBwZW5kaW5nIHdpdGhkcmF3YWwga2V5cyBmb3IgYSBzcGVjaWZpYyBhY2NvdW50LgAAABxnZXRfYWNjb3VudF93aXRoZHJhd2FsX2NvdW50AAAAAgAAAAAAAAAKZGF0YV9zdG9yZQAAAAAAEwAAAAAAAAAHYWNjb3VudAAAAAATAAAAAQAAAAQ=",
        "AAAAAAAAAVlJc3N1ZSAjMjc1OiByZWFkLW9ubHkgdmlldyBvZiBhIHBvc2l0aW9uJ3MgcGVuZGluZyAobm90LXlldC1zZXR0bGVkKSBmdW5kaW5nLAp3aXRob3V0IG11dGF0aW5nIGFueSBzdGF0ZS4gTWlycm9ycyB0aGUgZXhhY3QgbWF0aCBgc2V0dGxlX2Z1bmRpbmdfZmVlc2AKKGNsYWltYWJsZSBzaWRlKSBhbmQgYGdldF9wb3NpdGlvbl9mZWVzYCAob3dlZCBzaWRlKSBhcHBseSBkdXJpbmcgYSByZWFsCmRlY3JlYXNlLCBzbyB0aGUgcmV0dXJuZWQgYW1vdW50cyBtYXRjaCB3aGF0IHdvdWxkIGFjdHVhbGx5IGJlCmNyZWRpdGVkL2RlYml0ZWQgaWYgdGhlIHBvc2l0aW9uIHdlcmUgZGVjcmVhc2VkIHJpZ2h0IG5vdy4AAAAAAAAcZ2V0X2NsYWltYWJsZV9mdW5kaW5nX2Ftb3VudAAAAAcAAAAAAAAACmRhdGFfc3RvcmUAAAAAABMAAAAAAAAABm9yYWNsZQAAAAAAEwAAAAAAAAANb3JkZXJfaGFuZGxlcgAAAAAAABMAAAAAAAAAB2FjY291bnQAAAAAEwAAAAAAAAAGbWFya2V0AAAAAAATAAAAAAAAABBjb2xsYXRlcmFsX3Rva2VuAAAAEwAAAAAAAAAHaXNfbG9uZwAAAAABAAAAAQAAA+gAAAfQAAAAE0Z1bmRpbmdBbW91bnRSZXN1bHQA",
        "AAAAAgAAAB5NaXJyb3JzIEdNWCdzIE9yZGVyLk9yZGVyVHlwZS4AAAAAAAAAAAAJT3JkZXJUeXBlAAAAAAAACQAAAAAAAAAAAAAACk1hcmtldFN3YXAAAAAAAAAAAAAAAAAACUxpbWl0U3dhcAAAAAAAAAAAAAAAAAAADk1hcmtldEluY3JlYXNlAAAAAAAAAAAAAAAAAA1MaW1pdEluY3JlYXNlAAAAAAAAAAAAAAAAAAAOTWFya2V0RGVjcmVhc2UAAAAAAAAAAAAAAAAADUxpbWl0RGVjcmVhc2UAAAAAAAAAAAAAAAAAABBTdG9wTG9zc0RlY3JlYXNlAAAAAAAAAAAAAAALTGlxdWlkYXRpb24AAAAAAAAAAAAAAAAMU3RvcEluY3JlYXNl",
        "AAAAAQAAABpNaXJyb3JzIEdNWCdzIE9yZGVyLlByb3BzLgAAAAAAAAAAAApPcmRlclByb3BzAAAAAAAOAAAAAAAAABBhY2NlcHRhYmxlX3ByaWNlAAAACwAAAAAAAAAHYWNjb3VudAAAAAATAAAAAAAAABdjb2xsYXRlcmFsX2RlbHRhX2Ftb3VudAAAAAALAAAAAAAAAA1leGVjdXRpb25fZmVlAAAAAAAACwAAAAAAAAAYaW5pdGlhbF9jb2xsYXRlcmFsX3Rva2VuAAAAEwAAAAAAAAAHaXNfbG9uZwAAAAABAAAAAAAAAAZtYXJrZXQAAAAAABMAAAAAAAAAEW1pbl9vdXRwdXRfYW1vdW50AAAAAAAACwAAAAAAAAAKb3JkZXJfdHlwZQAAAAAH0AAAAAlPcmRlclR5cGUAAAAAAAAAAAAACHJlY2VpdmVyAAAAEwAAAAAAAAAOc2l6ZV9kZWx0YV91c2QAAAAAAAsAAAAAAAAACXN3YXBfcGF0aAAAAAAAA+oAAAATAAAAAAAAAA10cmlnZ2VyX3ByaWNlAAAAAAAACwAAAAAAAAAPdXBkYXRlZF9hdF90aW1lAAAAAAY=",
        "AAAAAQAAAFVVU0QgcHJpY2Ugd2l0aCBtaW4vbWF4IHNwcmVhZDsgMzAtZGVjaW1hbCBGTE9BVF9QUkVDSVNJT04uIE1pcnJvcnMgR01YJ3MgUHJpY2UuUHJvcHMuAAAAAAAAAAAAAApQcmljZVByb3BzAAAAAAACAAAAAAAAAANtYXgAAAAACwAAAAAAAAADbWluAAAAAAs=",
        "AAAAAQAAADhVc2VkIGJ5IGtlZXBlcnMgdG8gc3VibWl0IHByaWNlcyB0byB0aGUgb3JhY2xlIGNvbnRyYWN0LgAAAAAAAAAKVG9rZW5QcmljZQAAAAAAAwAAAAAAAAADbWF4AAAAAAsAAAAAAAAAA21pbgAAAAALAAAAAAAAAAV0b2tlbgAAAAAAABM=",
        "AAAAAQAAADxBZ2dyZWdhdGUgZnVuZGluZyBpbmZvcm1hdGlvbiBmb3IgYSBtYXJrZXQgKHVzZWQgYnkgUmVhZGVyKS4AAAAAAAAAC0Z1bmRpbmdJbmZvAAAAAAMAAAAAAAAAGWZ1bmRpbmdfZmFjdG9yX3Blcl9zZWNvbmQAAAAAAAALAAAAAAAAABxsb25nX2Z1bmRpbmdfYW1vdW50X3Blcl9zaXplAAAACwAAAAAAAAAdc2hvcnRfZnVuZGluZ19hbW91bnRfcGVyX3NpemUAAAAAAAAL",
        "AAAAAQAAABtNaXJyb3JzIEdNWCdzIE1hcmtldC5Qcm9wcy4AAAAAAAAAAAtNYXJrZXRQcm9wcwAAAAAEAAAAAAAAAAtpbmRleF90b2tlbgAAAAATAAAAAAAAAApsb25nX3Rva2VuAAAAAAATAAAAAAAAAAxtYXJrZXRfdG9rZW4AAAATAAAAAAAAAAtzaG9ydF90b2tlbgAAAAAT",
        "AAAAAQAAAGZBREwtZWxpZ2libGUgcG9zaXRpb24gY2FuZGlkYXRlIGZvciBhdXRvLWRlbGV2ZXJhZ2luZy4KUmV0dXJuZWQgYnkgcmVhZGVyOjpnZXRfYWRsX2VsaWdpYmxlX3Bvc2l0aW9ucy4AAAAAAAAAAAAMQWRsQ2FuZGlkYXRlAAAABQAAAAAAAAADa2V5AAAAA+4AAAAgAAAAAAAAAAVvd25lcgAAAAAAABMAAAAAAAAAFXBubF90b19zaXplX3JhdGlvX2JwcwAAAAAAAAQAAAAAAAAACHNpemVfdXNkAAAACgAAAAAAAAASdW5yZWFsaXNlZF9wbmxfdXNkAAAAAAAK",
        "AAAAAQAAABxNaXJyb3JzIEdNWCdzIERlcG9zaXQuUHJvcHMuAAAAAAAAAAxEZXBvc2l0UHJvcHMAAAAKAAAAAAAAAAdhY2NvdW50AAAAABMAAAAAAAAADWV4ZWN1dGlvbl9mZWUAAAAAAAALAAAAAAAAABJpbml0aWFsX2xvbmdfdG9rZW4AAAAAABMAAAAAAAAAE2luaXRpYWxfc2hvcnRfdG9rZW4AAAAAEwAAAAAAAAARbG9uZ190b2tlbl9hbW91bnQAAAAAAAALAAAAAAAAAAZtYXJrZXQAAAAAABMAAAAAAAAAEW1pbl9tYXJrZXRfdG9rZW5zAAAAAAAACwAAAAAAAAAIcmVjZWl2ZXIAAAATAAAAAAAAABJzaG9ydF90b2tlbl9hbW91bnQAAAAAAAsAAAAAAAAAD3VwZGF0ZWRfYXRfdGltZQAAAAAG",
        "AAAAAQAAARpMaWdodHdlaWdodCBwZW5kaW5nLW9yZGVyIHN1bW1hcnkgcmV0dXJuZWQgYnkgcmVhZGVyOjpnZXRfcGVuZGluZ19vcmRlcnMgKGlzc3VlICMyMDIpLgoKQ2FycmllcyBvbmx5IHRoZSBmaWVsZHMgYSBrZWVwZXIgYm90IG5lZWRzIHRvIGRlY2lkZSBleGVjdXRpb24gb3JkZXI6Cm93bmVyLCB0eXBlLCBzaXplLCBmZWUsIGxhc3QtdXBkYXRlIHRpbWUsIGFuZCBkaXJlY3Rpb24uCkZ1bGwgZGV0YWlscyBjYW4gYWx3YXlzIGJlIGZldGNoZWQgdmlhIHJlYWRlcjo6Z2V0X29yZGVyIGlmIG5lZWRlZC4AAAAAAAAAAAAMUGVuZGluZ09yZGVyAAAABwAAAAAAAAANZXhlY3V0aW9uX2ZlZQAAAAAAAAsAAAAAAAAAB2lzX2xvbmcAAAAAAQAAAAAAAAAGbWFya2V0AAAAAAATAAAAAAAAAApvcmRlcl90eXBlAAAAAAfQAAAACU9yZGVyVHlwZQAAAAAAAAAAAAAFb3duZXIAAAAAAAATAAAAAAAAAA5zaXplX2RlbHRhX3VzZAAAAAAACwAAAAAAAAAPdXBkYXRlZF9hdF90aW1lAAAAAAY=",
        "AAAAAQAAABdQb3NpdGlvbiBmZWUgYnJlYWtkb3duLgAAAAAAAAAADFBvc2l0aW9uRmVlcwAAAAQAAAAAAAAAFGJvcnJvd2luZ19mZWVfYW1vdW50AAAACwAAAAAAAAASZnVuZGluZ19mZWVfYW1vdW50AAAAAAALAAAAAAAAABNwb3NpdGlvbl9mZWVfYW1vdW50AAAAAAsAAAAAAAAAEXRvdGFsX2Nvc3RfYW1vdW50AAAAAAAACw==",
        "AAAAAQAAAEhSaWNoIHBvc2l0aW9uIGluZm8gaW5jbHVkaW5nIGNvbXB1dGVkIFBuTCBhbmQgZmVlcyAocmV0dXJuZWQgYnkgUmVhZGVyKS4AAAAAAAAADFBvc2l0aW9uSW5mbwAAAAgAAADRSXNzdWUgIzI2MDogd2VpZ2h0ZWQgYXZlcmFnZSBlbnRyeSBwcmljZSBhY3Jvc3MgYWxsIHBvc2l0aW9uIGluY3JlYXNlcy4KQ29tcHV0ZWQgYXMgc2l6ZV9pbl91c2QgLyBzaXplX2luX3Rva2VucyDDlyBUT0tFTl9QUkVDSVNJT04gKEZMT0FUX1BSRUNJU0lPTiB1bml0cykuClplcm8gd2hlbiBzaXplX2luX3Rva2VucyBpcyB6ZXJvIChubyBvcGVuIHBvc2l0aW9uKS4AAAAAAAAPYXZnX2VudHJ5X3ByaWNlAAAAAAsAAAAAAAAAEWJvcnJvd2luZ19mZWVfdXNkAAAAAAAACwAAAAAAAAAPZnVuZGluZ19mZWVfdXNkAAAAAAsAAAAAAAAAEWxpcXVpZGF0aW9uX3ByaWNlAAAAAAAACwAAAAAAAAAHcG5sX3VzZAAAAAALAAAAAAAAAAhwb3NpdGlvbgAAB9AAAAANUG9zaXRpb25Qcm9wcwAAAAAAAAAAAAAQcG9zaXRpb25fZmVlX3VzZAAAAAsAAAAAAAAAEHVuY2FwcGVkX3BubF91c2QAAAAL",
        "AAAAAQAAAFRFc3RpbWF0ZWQgc3dhcCBvdXRwdXQgZm9yIGRyeS1ydW4gcXVlcmllcy4KUmV0dXJuZWQgYnkgcmVhZGVyOjplc3RpbWF0ZV9zd2FwX291dHB1dC4AAAAAAAAADFN3YXBFc3RpbWF0ZQAAAAUAAAAAAAAACmFtb3VudF9vdXQAAAAAAAoAAAAAAAAAD2V4ZWN1dGlvbl9wcmljZQAAAAAKAAAAAAAAABBwcmljZV9pbXBhY3RfdXNkAAAACwAAAAAAAAATcmV2ZXJ0c19pZl9leGVjdXRlZAAAAAABAAAAAAAAAAl0b2tlbl9vdXQAAAAAAAAT",
        "AAAAAQAAAENGdWxsIHBvb2wgdmFsdWUgYnJlYWtkb3duIHJldHVybmVkIGJ5IG1hcmtldF91dGlsczo6Z2V0X3Bvb2xfdmFsdWUuAAAAAAAAAAANUG9vbFZhbHVlSW5mbwAAAAAAAAkAAAAAAAAAEmltcGFjdF9wb29sX2Ftb3VudAAAAAAACwAAAAAAAAAIbG9uZ19wbmwAAAALAAAAAAAAABFsb25nX3Rva2VuX2Ftb3VudAAAAAAAAAsAAAAAAAAADmxvbmdfdG9rZW5fdXNkAAAAAAALAAAAAAAAAAduZXRfcG5sAAAAAAsAAAAAAAAACnBvb2xfdmFsdWUAAAAAAAsAAAAAAAAACXNob3J0X3BubAAAAAAAAAsAAAAAAAAAEnNob3J0X3Rva2VuX2Ftb3VudAAAAAAACwAAAAAAAAAPc2hvcnRfdG9rZW5fdXNkAAAAAAs=",
        "AAAAAQAAAN9NaXJyb3JzIEdNWCdzIFBvc2l0aW9uLlByb3BzLgpGaWVsZCBuYW1lIGFiYnJldmlhdGlvbnMgKDMwLWNoYXIgbGltaXQgaW4gI1tjb250cmFjdHR5cGVdKToKbG9uZ19jbGFpbV9mbmRfcGVyX3NpemUgID0gbG9uZ1Rva2VuQ2xhaW1hYmxlRnVuZGluZ0Ftb3VudFBlclNpemUKc2hvcnRfY2xhaW1fZm5kX3Blcl9zaXplID0gc2hvcnRUb2tlbkNsYWltYWJsZUZ1bmRpbmdBbW91bnRQZXJTaXplAAAAAAAAAAANUG9zaXRpb25Qcm9wcwAAAAAAAA4AAAAAAAAAB2FjY291bnQAAAAAEwAAAAAAAAAQYm9ycm93aW5nX2ZhY3RvcgAAAAsAAAAAAAAAEWNvbGxhdGVyYWxfYW1vdW50AAAAAAAACwAAAAAAAAAQY29sbGF0ZXJhbF90b2tlbgAAABMAAAAAAAAAEWRlY3JlYXNlZF9hdF90aW1lAAAAAAAABgAAAAAAAAAbZnVuZGluZ19mZWVfYW1vdW50X3Blcl9zaXplAAAAAAsAAAAAAAAAEWluY3JlYXNlZF9hdF90aW1lAAAAAAAABgAAAAAAAAAHaXNfbG9uZwAAAAABAAAAAAAAABdsb25nX2NsYWltX2ZuZF9wZXJfc2l6ZQAAAAALAAAAAAAAAAZtYXJrZXQAAAAAABMAAAAAAAAAFXBlbmRpbmdfaW1wYWN0X2Ftb3VudAAAAAAAAAsAAAAAAAAAGHNob3J0X2NsYWltX2ZuZF9wZXJfc2l6ZQAAAAsAAAAAAAAADnNpemVfaW5fdG9rZW5zAAAAAAALAAAAAAAAAAtzaXplX2luX3VzZAAAAAAL",
        "AAAAAQAAAZJBZ2dyZWdhdGVkIHByb3RvY29sLXdpZGUgc3RhdGlzdGljcyBhY3Jvc3MgYSBzZXQgb2YgbWFya2V0cyAocmV0dXJuZWQgYnkgUmVhZGVyKS4KCklzc3VlICMyNTE6IGxldHMgdGhlIGZyb250ZW5kIGZldGNoIGhlYWRsaW5lIG51bWJlcnMgKFRWTCwgT0ksIGFjY3VtdWxhdGVkCmZlZXMpIGluIGEgc2luZ2xlIGNhbGwgaW5zdGVhZCBvZiBOIHBlci1tYXJrZXQgcm91bmQtdHJpcHMuIEFsbCBVU0QgZmlndXJlcwp1c2UgMzAtZGVjaW1hbCBGTE9BVF9QUkVDSVNJT04gYW5kIHJlZmxlY3QgdGhlIG9yYWNsZSBwcmljZXMgYXQgdGhlIGxlZGdlcgppbiB3aGljaCB0aGUgY2FsbCBleGVjdXRlcyDigJQgdGhlIHJlc3VsdCBpcyBhIHNuYXBzaG90LCB2YWxpZCBvbmx5IGZvcgpgY29tcHV0ZWRfYXRfbGVkZ2VyYC4AAAAAAAAAAAANUHJvdG9jb2xTdGF0cwAAAAAAAAYAAAAAAAAAEmNvbXB1dGVkX2F0X2xlZGdlcgAAAAAABgAAAAAAAAAMbWFya2V0X2NvdW50AAAABAAAAAAAAAAadG90YWxfYWNjdW11bGF0ZWRfZmVlc191c2QAAAAAAAsAAAAAAAAAHHRvdGFsX2xvbmdfb3Blbl9pbnRlcmVzdF91c2QAAAALAAAAAAAAABR0b3RhbF9wb29sX3ZhbHVlX3VzZAAAAAsAAAAAAAAAHXRvdGFsX3Nob3J0X29wZW5faW50ZXJlc3RfdXNkAAAAAAAACw==",
        "AAAAAQAAAEZEZXRhaWxlZCBwZXItaG91ciBmdW5kaW5nIHJhdGUgdmlldyBmb3IgZnJvbnRlbmQgZGlzcGxheSAoaXNzdWUgIzIwNykuAAAAAAAAAAAAD0Z1bmRpbmdSYXRlSW5mbwAAAAAHAAAAAAAAABlmdW5kaW5nX3VwZGF0ZWRfYXRfbGVkZ2VyAAAAAAAABgAAAAAAAAAcbG9uZ19mdW5kaW5nX2Ftb3VudF9wZXJfc2l6ZQAAAAsAAAAAAAAAGmxvbmdfZnVuZGluZ19yYXRlX3Blcl9ob3VyAAAAAAALAAAAAAAAABZsb25nX29wZW5faW50ZXJlc3RfdXNkAAAAAAAKAAAAAAAAAB1zaG9ydF9mdW5kaW5nX2Ftb3VudF9wZXJfc2l6ZQAAAAAAAAsAAAAAAAAAG3Nob3J0X2Z1bmRpbmdfcmF0ZV9wZXJfaG91cgAAAAALAAAAAAAAABdzaG9ydF9vcGVuX2ludGVyZXN0X3VzZAAAAAAK",
        "AAAAAQAAAB9NaXJyb3JzIEdNWCdzIFdpdGhkcmF3YWwuUHJvcHMuAAAAAAAAAAAPV2l0aGRyYXdhbFByb3BzAAAAAAgAAAAAAAAAB2FjY291bnQAAAAAEwAAAAAAAAANZXhlY3V0aW9uX2ZlZQAAAAAAAAsAAAAAAAAABm1hcmtldAAAAAAAEwAAAAAAAAATbWFya2V0X3Rva2VuX2Ftb3VudAAAAAALAAAAAAAAABVtaW5fbG9uZ190b2tlbl9hbW91bnQAAAAAAAALAAAAAAAAABZtaW5fc2hvcnRfdG9rZW5fYW1vdW50AAAAAAALAAAAAAAAAAhyZWNlaXZlcgAAABMAAAAAAAAAD3VwZGF0ZWRfYXRfdGltZQAAAAAG",
        "AAAAAQAAAWVQb3NpdGlvbiBsZXZlcmFnZSBicmVha2Rvd24gcmV0dXJuZWQgYnkgcmVhZGVyOjpnZXRfcG9zaXRpb25fbGV2ZXJhZ2UuCmVmZmVjdGl2ZV9sZXZlcmFnZV94MTAwID0gc2l6ZV91c2QgKiAxMDAgLyBuZXRfY29sbGF0ZXJhbF91c2QgKGUuZy4gMjAwMCA9IDIww5cpLgpUaGlzIGlzIE5PVCBiYXNpcyBwb2ludHMgKEJQU19ESVZJU09SID0gMTBfMDAwKSDigJQgZ2VudWluZSBicHMgd291bGQgcmVwcmVzZW50CjIww5cgYXMgMjAwXzAwMCwgbm90IDIwMDAuIEl0J3MgYSBsZXZlcmFnZS10aW1lcy0xMDAgc2NhbGUuCklmIG5ldF9jb2xsYXRlcmFsX3VzZCA9PSAwLCBlZmZlY3RpdmVfbGV2ZXJhZ2VfeDEwMCA9PSB1MzI6Ok1BWC4AAAAAAAAAAAAAEFBvc2l0aW9uTGV2ZXJhZ2UAAAAEAAAAAAAAABdlZmZlY3RpdmVfbGV2ZXJhZ2VfeDEwMAAAAAAEAAAAAAAAAA9pc19saXF1aWRhdGFibGUAAAAAAQAAAAAAAAASbmV0X2NvbGxhdGVyYWxfdXNkAAAAAAAKAAAAAAAAABFwb3NpdGlvbl9zaXplX3VzZAAAAAAAAAo=",
        "AAAAAQAAAF1Vc2VyLXN1cHBsaWVkIHBhcmFtZXRlcnMgZm9yIGNyZWF0aW5nIGFuIG9yZGVyLiBNaXJyb3JzIEdNWCBCYXNlT3JkZXJVdGlscy5DcmVhdGVPcmRlclBhcmFtcy4AAAAAAAAAAAAAEUNyZWF0ZU9yZGVyUGFyYW1zAAAAAAAADgAAAAAAAAAQYWNjZXB0YWJsZV9wcmljZQAAAAsAAAAAAAAAF2NvbGxhdGVyYWxfZGVsdGFfYW1vdW50AAAAAAsAAAAAAAAADWV4ZWN1dGlvbl9mZWUAAAAAAAALAAAAzE9wdGlvbmFsIGxlZGdlci1zZXF1ZW5jZSBkZWFkbGluZS4gSWYgYFNvbWUobilgIHRoZSBvcmRlciBhdXRvLWNhbmNlbHMKKHdpdGggZnVsbCByZWZ1bmQpIHdoZW4gdGhlIGtlZXBlciBjYWxscyBgZXhlY3V0ZV9vcmRlcmAgYWZ0ZXIgc2VxdWVuY2UgYG5gLgpgTm9uZWAgbWVhbnMgdGhlIG9yZGVyIG5ldmVyIGV4cGlyZXMgdmlhIHRoaXMgbWVjaGFuaXNtLgAAAA1leHBpcnlfbGVkZ2VyAAAAAAAD6AAAAAYAAAAAAAAAGGluaXRpYWxfY29sbGF0ZXJhbF90b2tlbgAAABMAAAAAAAAAB2lzX2xvbmcAAAAAAQAAAAAAAAAGbWFya2V0AAAAAAATAAAAAAAAABFtaW5fb3V0cHV0X2Ftb3VudAAAAAAAAAsAAAEASXNzdWUgIzM4NS8jNTM1OiB3aGVuIGEgcmVnaXN0ZXJlZCBwb3NpdGlvbiBtYW5hZ2VyIGNhbGxzIG9uIGJlaGFsZiBvZgphbiBvd25lciwgdGhlIG93bmVyIGlzIG5hbWVkIGhlcmUgZXhwbGljaXRseS4gYFNvbWUob3duZXIpYCByZXF1aXJlcwpgZGF0YV9zdG9yZTo6Z2V0X3Bvc2l0aW9uX21hbmFnZXIob3duZXIsIG1hcmtldCkgPT0gU29tZShjYWxsZXIpYDsKYE5vbmVgIG1lYW5zIHRoZSBjYWxsZXIgaXMgYWN0aW5nIGZvciB0aGVtc2VsdmVzLgAAAAxvbl9iZWhhbGZfb2YAAAPoAAAAEwAAAAAAAAAKb3JkZXJfdHlwZQAAAAAH0AAAAAlPcmRlclR5cGUAAAAAAAAAAAAACHJlY2VpdmVyAAAAEwAAAAAAAAAOc2l6ZV9kZWx0YV91c2QAAAAAAAsAAAAAAAAACXN3YXBfcGF0aAAAAAAAA+oAAAATAAAAAAAAAA10cmlnZ2VyX3ByaWNlAAAAAAAACw==",
        "AAAAAQAAADBVc2VyLXN1cHBsaWVkIHBhcmFtZXRlcnMgZm9yIGNyZWF0aW5nIGEgZGVwb3NpdC4AAAAAAAAAE0NyZWF0ZURlcG9zaXRQYXJhbXMAAAAACAAAAAAAAAANZXhlY3V0aW9uX2ZlZQAAAAAAAAsAAAAAAAAAEmluaXRpYWxfbG9uZ190b2tlbgAAAAAAEwAAAAAAAAATaW5pdGlhbF9zaG9ydF90b2tlbgAAAAATAAAAAAAAABFsb25nX3Rva2VuX2Ftb3VudAAAAAAAAAsAAAAAAAAABm1hcmtldAAAAAAAEwAAAAAAAAARbWluX21hcmtldF90b2tlbnMAAAAAAAALAAAAAAAAAAhyZWNlaXZlcgAAABMAAAAAAAAAEnNob3J0X3Rva2VuX2Ftb3VudAAAAAAACw==",
        "AAAAAQAAANVQZW5kaW5nIChub3QteWV0LXNldHRsZWQpIGZ1bmRpbmcgZm9yIGEgc2luZ2xlIHBvc2l0aW9uIOKAlCBpc3N1ZSAjMjc1LgpQb3NpdGl2ZSA9IGFtb3VudCB0aGUgaG9sZGVyIHdvdWxkIHJlY2VpdmU7IG5lZ2F0aXZlID0gYW1vdW50IHRoZSBob2xkZXIKd291bGQgcGF5LCBpbiBlYWNoIG9mIHRoZSBwb3NpdGlvbidzIGxvbmdfdG9rZW4gLyBzaG9ydF90b2tlbiB1bml0cy4AAAAAAAAAAAAAE0Z1bmRpbmdBbW91bnRSZXN1bHQAAAAAAwAAAAAAAAAJYXRfbGVkZ2VyAAAAAAAABgAAAAAAAAARbG9uZ190b2tlbl9hbW91bnQAAAAAAAALAAAAAAAAABJzaG9ydF90b2tlbl9hbW91bnQAAAAAAAs=",
        "AAAAAQAAASZMaXF1aWRhdGFibGUgcG9zaXRpb24gZW50cnkgcmV0dXJuZWQgYnkgYHJlYWRlcjo6Z2V0X2xpcXVpZGF0YWJsZV9wb3NpdGlvbnNgIChpc3N1ZSAjMjgzKS4KCmBoZWFsdGhfZmFjdG9yX2JwcyA8IDEwMDAwYCBtZWFucyB0aGUgcG9zaXRpb24gaXMgYmVsb3cgdGhlIG1pbi1jb2xsYXRlcmFsIHRocmVzaG9sZAooaS5lLiwgY3VycmVudGx5IGVsaWdpYmxlIGZvciBsaXF1aWRhdGlvbikuIFNvcnRlZCBhc2NlbmRpbmcgc28gdGhlIG1vc3QKdW5kZXItY29sbGF0ZXJhbGlzZWQgcG9zaXRpb25zIGFwcGVhciBmaXJzdC4AAAAAAAAAAAAUTGlxdWlkYXRhYmxlUG9zaXRpb24AAAAFAAAAAAAAAA5jb2xsYXRlcmFsX3VzZAAAAAAACgAAAAAAAAARaGVhbHRoX2ZhY3Rvcl9icHMAAAAAAAAEAAAAAAAAAANrZXkAAAAD7gAAACAAAAAAAAAABW93bmVyAAAAAAAAEwAAAAAAAAAIc2l6ZV91c2QAAAAK",
        "AAAAAQAAAUtMaXZlbmVzcyBzdGF0dXMgZm9yIGEga2VlcGVyIHJvbGUgKGlzc3VlICMyNDkpLCByZXR1cm5lZCBieSBSZWFkZXIuCgpBIGtlZXBlciB0aGF0IGhhcyBnb25lIHBlcm1hbmVudGx5IG9mZmxpbmUgbGVhdmVzIG9yZGVycyB1bmV4ZWN1dGVkLiBUaGlzCmdpdmVzIHRoZSBhZG1pbiBhbiBvbi1jaGFpbiBzaWduYWw6IHdoZW4gYGlzX3N0YWxlYCBpcyB0cnVlIHRoZSBnYXAgc2luY2UgdGhlCmxhc3Qgc3VjY2Vzc2Z1bCBleGVjdXRpb24gaGFzIGV4Y2VlZGVkIHRoZSBjb25maWd1cmVkIGhlYXJ0YmVhdCB0aW1lb3V0LCBhbmQKdGhlIGtlZXBlcidzIHJvbGUgY2FuIGJlIHJldm9rZWQuAAAAAAAAAAAVS2VlcGVySGVhcnRiZWF0U3RhdHVzAAAAAAAAAwAAAAAAAAAIaXNfc3RhbGUAAAABAAAAAAAAABJsYXN0X2FjdGl2ZV9sZWRnZXIAAAAAAAYAAAAAAAAAG2xlZGdlcnNfc2luY2VfbGFzdF9hY3Rpdml0eQAAAAAG",
        "AAAAAQAAADNVc2VyLXN1cHBsaWVkIHBhcmFtZXRlcnMgZm9yIGNyZWF0aW5nIGEgd2l0aGRyYXdhbC4AAAAAAAAAABZDcmVhdGVXaXRoZHJhd2FsUGFyYW1zAAAAAAAGAAAAAAAAAA1leGVjdXRpb25fZmVlAAAAAAAACwAAAAAAAAAGbWFya2V0AAAAAAATAAAAAAAAABNtYXJrZXRfdG9rZW5fYW1vdW50AAAAAAsAAAAAAAAAFW1pbl9sb25nX3Rva2VuX2Ftb3VudAAAAAAAAAsAAAAAAAAAFm1pbl9zaG9ydF90b2tlbl9hbW91bnQAAAAAAAsAAAAAAAAACHJlY2VpdmVyAAAAEw==",
        "AAAAAQAAAEBSZXN1bHQgb2YgZXhlY3V0aW5nIGEgcG9zaXRpb24gZGVjcmVhc2UgKHBhcnRpYWwgb3IgZnVsbCBjbG9zZSkuAAAAAAAAABZEZWNyZWFzZVBvc2l0aW9uUmVzdWx0AAAAAAAGAAAAAAAAAA9leGVjdXRpb25fcHJpY2UAAAAACwAAAAAAAAAPaXNfZnVsbHlfY2xvc2VkAAAAAAEAAAAAAAAADW91dHB1dF9hbW91bnQAAAAAAAALAAAAAAAAAAdwbmxfdXNkAAAAAAsAAAAAAAAAFHJlbWFpbmluZ19jb2xsYXRlcmFsAAAACwAAAAAAAAAXc2Vjb25kYXJ5X291dHB1dF9hbW91bnQAAAAACw==" ]),
      options
    )
  }
  public readonly fromJSON = {
    upgrade: this.txFromJSON<null>,
        get_order: this.txFromJSON<Option<OrderProps>>,
        get_market: this.txFromJSON<MarketProps>,
        initialize: this.txFromJSON<null>,
        get_deposit: this.txFromJSON<Option<DepositProps>>,
        get_order_keys: this.txFromJSON<Array<Buffer>>,
        get_withdrawal: this.txFromJSON<Option<WithdrawalProps>>,
        get_order_count: this.txFromJSON<u32>,
        get_deposit_keys: this.txFromJSON<Array<Buffer>>,
        get_funding_info: this.txFromJSON<FundingInfo>,
        get_deposit_count: this.txFromJSON<u32>,
        get_open_interest: this.txFromJSON<readonly [i128, i128]>,
        get_position_info: this.txFromJSON<Option<PositionInfo>>,
        get_account_orders: this.txFromJSON<Array<OrderProps>>,
        get_pending_orders: this.txFromJSON<Array<PendingOrder>>,
        get_protocol_stats: this.txFromJSON<ProtocolStats>,
        get_withdrawal_keys: this.txFromJSON<Array<Buffer>>,
        estimate_swap_output: this.txFromJSON<SwapEstimate>,
        get_withdrawal_count: this.txFromJSON<u32>,
        get_account_positions: this.txFromJSON<Array<PositionInfo>>,
        get_funding_rate_info: this.txFromJSON<FundingRateInfo>,
        get_position_leverage: this.txFromJSON<Option<PositionLeverage>>,
        check_keeper_heartbeat: this.txFromJSON<KeeperHeartbeatStatus>,
        get_account_order_keys: this.txFromJSON<Array<Buffer>>,
        get_market_token_price: this.txFromJSON<u128>,
        get_account_order_count: this.txFromJSON<u32>,
        get_account_deposit_keys: this.txFromJSON<Array<Buffer>>,
        get_position_info_by_key: this.txFromJSON<Option<PositionInfo>>,
        is_position_liquidatable: this.txFromJSON<boolean>,
        get_account_deposit_count: this.txFromJSON<u32>,
        get_adl_eligible_positions: this.txFromJSON<Array<AdlCandidate>>,
        get_liquidatable_positions: this.txFromJSON<Array<LiquidatablePosition>>,
        get_market_pool_value_info: this.txFromJSON<PoolValueInfo>,
        get_account_withdrawal_keys: this.txFromJSON<Array<Buffer>>,
        get_execution_price_preview: this.txFromJSON<i128>,
        get_account_withdrawal_count: this.txFromJSON<u32>,
        get_claimable_funding_amount: this.txFromJSON<Option<FundingAmountResult>>
  }
}