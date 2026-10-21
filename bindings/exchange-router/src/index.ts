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
  1: {message:"AlreadyInitialized"},
  2: {message:"NotInitialized"},
  3: {message:"Unauthorized"},
  4: {message:"Paused"},
  5: {message:"BatchSizeLimitExceeded"},
  /**
   * `execute_unpause` was called before the timelock window expired (issue #282).
   */
  6: {message:"TimelockNotExpired"},
  /**
   * `execute_unpause` was called without a prior `schedule_unpause` (issue #282).
   */
  7: {message:"UnpauseNotScheduled"},
  /**
   * `update_withdrawal_handler` was called with a `new_handler` that matches
   * the router itself or one of its other registered handlers (issue #635) —
   * almost certainly a copy-paste mistake that would silently misroute
   * withdrawal calls to a contract with a different interface.
   */
  8: {message:"InvalidWithdrawalHandler"},
  /**
   * claim_funding_fees (or multicall's ClaimFundingFees action) was called
   * with markets.len() != tokens.len() (issue #611) — indexing would
   * otherwise panic with a raw Option::unwrap() on None rather than a
   * typed error.
   */
  9: {message:"MismatchedBatchLength"},
  /**
   * multicall's SendTokens action was given `amount <= 0` (issue #731) —
   * rejected up front with a typed error instead of surfacing whatever the
   * token contract does with a zero or negative transfer.
   */
  10: {message:"InvalidAmount"}
}

/**
 * Each element in a multicall Vec is one action variant.
 */
export type RouterAction = {tag: "SendTokens", values: readonly [SendTokensParams]} | {tag: "CreateDeposit", values: readonly [CreateDepositParams]} | {tag: "CancelDeposit", values: readonly [Buffer]} | {tag: "CreateWithdrawal", values: readonly [CreateWithdrawalParams]} | {tag: "CancelWithdrawal", values: readonly [Buffer]} | {tag: "CreateOrder", values: readonly [CreateOrderParams]} | {tag: "UpdateOrder", values: readonly [UpdateOrderParams]} | {tag: "CancelOrder", values: readonly [Buffer]} | {tag: "ClaimFundingFees", values: readonly [ClaimFundingFeesParams]};


export interface SendTokensParams {
  amount: i128;
  receiver: string;
  token: string;
}


export interface UpdateOrderParams {
  acceptable_price: i128;
  key: Buffer;
  min_output_amount: i128;
  size_delta_usd: i128;
  trigger_price: i128;
}


export interface ClaimFundingFeesParams {
  markets: Array<string>;
  tokens: Array<string>;
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
   * Construct and simulate a multicall transaction. Returns an `AssembledTransaction` object which will have a `result` field containing the result of the simulation. If this transaction changes contract state, you will need to call `signAndSend()` on the returned object.
   * Execute a batch of actions atomically.
   * 
   * Single caller.require_auth() covers all sub-actions (they run inside this invocation).
   * Returns one BytesN<32> result per action (create_* returns a key; others return zero hash).
   * If any action panics, the entire transaction reverts (Soroban atomicity).
   * 
   * Handlers are called directly (not via the self-referential public wrappers) to avoid a
   * double require_auth() within the same invocation frame, which Soroban rejects.
   */
  multicall: ({caller, actions}: {caller: string, actions: Array<RouterAction>}, options?: MethodOptions) => Promise<AssembledTransaction<Array<Buffer>>>

  /**
   * Construct and simulate a initialize transaction. Returns an `AssembledTransaction` object which will have a `result` field containing the result of the simulation. If this transaction changes contract state, you will need to call `signAndSend()` on the returned object.
   * One-time setup — store all handler addresses.
   */
  initialize: ({admin, role_store, data_store, deposit_handler, withdrawal_handler, order_handler, fee_handler}: {admin: string, role_store: string, data_store: string, deposit_handler: string, withdrawal_handler: string, order_handler: string, fee_handler: string}, options?: MethodOptions) => Promise<AssembledTransaction<null>>

  /**
   * Construct and simulate a set_paused transaction. Returns an `AssembledTransaction` object which will have a `result` field containing the result of the simulation. If this transaction changes contract state, you will need to call `signAndSend()` on the returned object.
   * Pause the protocol immediately.
   * 
   * Re-pausing clears any pending `schedule_unpause` so defenders can reset
   * the timelock clock if the threat resurfaces (issue #282).
   */
  set_paused: ({paused}: {paused: boolean}, options?: MethodOptions) => Promise<AssembledTransaction<null>>

  /**
   * Construct and simulate a cancel_order transaction. Returns an `AssembledTransaction` object which will have a `result` field containing the result of the simulation. If this transaction changes contract state, you will need to call `signAndSend()` on the returned object.
   * Forward cancel_order to the order_handler.
   */
  cancel_order: ({caller, key}: {caller: string, key: Buffer}, options?: MethodOptions) => Promise<AssembledTransaction<null>>

  /**
   * Construct and simulate a update_order transaction. Returns an `AssembledTransaction` object which will have a `result` field containing the result of the simulation. If this transaction changes contract state, you will need to call `signAndSend()` on the returned object.
   * Forward update_order to the order_handler.
   */
  update_order: ({caller, params}: {caller: string, params: UpdateOrderParams}, options?: MethodOptions) => Promise<AssembledTransaction<null>>

  /**
   * Construct and simulate a cancel_deposit transaction. Returns an `AssembledTransaction` object which will have a `result` field containing the result of the simulation. If this transaction changes contract state, you will need to call `signAndSend()` on the returned object.
   * Forward cancel_deposit to the deposit_handler.
   */
  cancel_deposit: ({caller, key}: {caller: string, key: Buffer}, options?: MethodOptions) => Promise<AssembledTransaction<null>>

  /**
   * Construct and simulate a create_deposit transaction. Returns an `AssembledTransaction` object which will have a `result` field containing the result of the simulation. If this transaction changes contract state, you will need to call `signAndSend()` on the returned object.
   * Forward create_deposit to the deposit_handler.
   */
  create_deposit: ({caller, params}: {caller: string, params: CreateDepositParams}, options?: MethodOptions) => Promise<AssembledTransaction<Buffer>>

  /**
   * Construct and simulate a execute_unpause transaction. Returns an `AssembledTransaction` object which will have a `result` field containing the result of the simulation. If this transaction changes contract state, you will need to call `signAndSend()` on the returned object.
   * Execute a previously scheduled unpause if the timelock has expired (issue #282).
   * 
   * Reverts with `TimelockNotExpired` if called before the scheduled ledger,
   * and with `UnpauseNotScheduled` if `schedule_unpause` was never called (or was
   * cleared by a re-pause). On success, clears both the pause flag and the schedule.
   */
  execute_unpause: (options?: MethodOptions) => Promise<AssembledTransaction<null>>

  /**
   * Construct and simulate a schedule_unpause transaction. Returns an `AssembledTransaction` object which will have a `result` field containing the result of the simulation. If this transaction changes contract state, you will need to call `signAndSend()` on the returned object.
   * Schedule an unpause after `UNPAUSE_TIMELOCK_LEDGERS` ledgers (issue #282).
   * 
   * Records `current_ledger + UNPAUSE_TIMELOCK_LEDGERS` as the earliest ledger
   * at which `execute_unpause` may succeed. Emits the scheduled ledger as an event
   * so off-chain monitoring can observe the intent. Admin only.
   */
  schedule_unpause: (options?: MethodOptions) => Promise<AssembledTransaction<null>>

  /**
   * Construct and simulate a cancel_withdrawal transaction. Returns an `AssembledTransaction` object which will have a `result` field containing the result of the simulation. If this transaction changes contract state, you will need to call `signAndSend()` on the returned object.
   * Forward cancel_withdrawal to the withdrawal_handler.
   */
  cancel_withdrawal: ({caller, key}: {caller: string, key: Buffer}, options?: MethodOptions) => Promise<AssembledTransaction<null>>

  /**
   * Construct and simulate a create_withdrawal transaction. Returns an `AssembledTransaction` object which will have a `result` field containing the result of the simulation. If this transaction changes contract state, you will need to call `signAndSend()` on the returned object.
   * Forward create_withdrawal to the withdrawal_handler.
   */
  create_withdrawal: ({caller, params}: {caller: string, params: CreateWithdrawalParams}, options?: MethodOptions) => Promise<AssembledTransaction<Buffer>>

  /**
   * Construct and simulate a set_ui_fee_factor transaction. Returns an `AssembledTransaction` object which will have a `result` field containing the result of the simulation. If this transaction changes contract state, you will need to call `signAndSend()` on the returned object.
   * Set the UI fee factor for a receiver. Delegates auth enforcement to fee_handler.
   */
  set_ui_fee_factor: ({ui_receiver, factor}: {ui_receiver: string, factor: u128}, options?: MethodOptions) => Promise<AssembledTransaction<null>>

  /**
   * Construct and simulate a claim_funding_fees transaction. Returns an `AssembledTransaction` object which will have a `result` field containing the result of the simulation. If this transaction changes contract state, you will need to call `signAndSend()` on the returned object.
   * Claim earned funding fees across multiple markets in one call.
   */
  claim_funding_fees: ({caller, markets, tokens}: {caller: string, markets: Array<string>, tokens: Array<string>}, options?: MethodOptions) => Promise<AssembledTransaction<null>>

  /**
   * Construct and simulate a update_fee_handler transaction. Returns an `AssembledTransaction` object which will have a `result` field containing the result of the simulation. If this transaction changes contract state, you will need to call `signAndSend()` on the returned object.
   * Update the fee_handler address. Only the stored admin may call this.
   */
  update_fee_handler: ({caller, new_handler}: {caller: string, new_handler: string}, options?: MethodOptions) => Promise<AssembledTransaction<null>>

  /**
   * Construct and simulate a get_position_manager transaction. Returns an `AssembledTransaction` object which will have a `result` field containing the result of the simulation. If this transaction changes contract state, you will need to call `signAndSend()` on the returned object.
   * Query the current position manager for an account on a specific market.
   */
  get_position_manager: ({owner, market}: {owner: string, market: string}, options?: MethodOptions) => Promise<AssembledTransaction<Option<string>>>

  /**
   * Construct and simulate a set_position_manager transaction. Returns an `AssembledTransaction` object which will have a `result` field containing the result of the simulation. If this transaction changes contract state, you will need to call `signAndSend()` on the returned object.
   * Set a position manager for the caller on a specific market.
   * 
   * A position manager is authorized to create, increase, decrease, or close
   * positions on behalf of the owner, but cannot redirect collateral receipts.
   * The manager cannot override the receiver — funds always go to the owner.
   * 
   * Revoke an existing manager with remove_position_manager.
   */
  set_position_manager: ({caller, market, manager}: {caller: string, market: string, manager: string}, options?: MethodOptions) => Promise<AssembledTransaction<null>>

  /**
   * Construct and simulate a update_order_handler transaction. Returns an `AssembledTransaction` object which will have a `result` field containing the result of the simulation. If this transaction changes contract state, you will need to call `signAndSend()` on the returned object.
   * Update the order_handler address. Only the stored admin may call this.
   */
  update_order_handler: ({caller, new_handler}: {caller: string, new_handler: string}, options?: MethodOptions) => Promise<AssembledTransaction<null>>

  /**
   * Construct and simulate a reset_circuit_breaker transaction. Returns an `AssembledTransaction` object which will have a `result` field containing the result of the simulation. If this transaction changes contract state, you will need to call `signAndSend()` on the returned object.
   */
  reset_circuit_breaker: ({market}: {market: string}, options?: MethodOptions) => Promise<AssembledTransaction<null>>

  /**
   * Construct and simulate a update_deposit_handler transaction. Returns an `AssembledTransaction` object which will have a `result` field containing the result of the simulation. If this transaction changes contract state, you will need to call `signAndSend()` on the returned object.
   * Update the deposit_handler address. Only the stored admin may call this.
   * Rejects the router's own address and its other registered handler addresses
   * as a basic sanity check against copy-paste misconfiguration.
   */
  update_deposit_handler: ({caller, new_handler}: {caller: string, new_handler: string}, options?: MethodOptions) => Promise<AssembledTransaction<null>>

  /**
   * Construct and simulate a remove_position_manager transaction. Returns an `AssembledTransaction` object which will have a `result` field containing the result of the simulation. If this transaction changes contract state, you will need to call `signAndSend()` on the returned object.
   * Revoke the caller's position manager on a specific market. Only the
   * owner can call this; get_position_manager returns None afterwards.
   */
  remove_position_manager: ({caller, market}: {caller: string, market: string}, options?: MethodOptions) => Promise<AssembledTransaction<boolean>>

  /**
   * Construct and simulate a update_withdrawal_handler transaction. Returns an `AssembledTransaction` object which will have a `result` field containing the result of the simulation. If this transaction changes contract state, you will need to call `signAndSend()` on the returned object.
   * Update the withdrawal_handler address. Only the stored admin may call this.
   * Rejects the router's own address and its other registered handler addresses
   * as a basic sanity check against copy-paste misconfiguration (issue #635).
   */
  update_withdrawal_handler: ({caller, new_handler}: {caller: string, new_handler: string}, options?: MethodOptions) => Promise<AssembledTransaction<null>>

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
      new ContractSpec([ "AAAABAAAAAAAAAAAAAAABUVycm9yAAAAAAAACgAAAAAAAAASQWxyZWFkeUluaXRpYWxpemVkAAAAAAABAAAAAAAAAA5Ob3RJbml0aWFsaXplZAAAAAAAAgAAAAAAAAAMVW5hdXRob3JpemVkAAAAAwAAAAAAAAAGUGF1c2VkAAAAAAAEAAAAAAAAABZCYXRjaFNpemVMaW1pdEV4Y2VlZGVkAAAAAAAFAAAATWBleGVjdXRlX3VucGF1c2VgIHdhcyBjYWxsZWQgYmVmb3JlIHRoZSB0aW1lbG9jayB3aW5kb3cgZXhwaXJlZCAoaXNzdWUgIzI4MikuAAAAAAAAElRpbWVsb2NrTm90RXhwaXJlZAAAAAAABgAAAE1gZXhlY3V0ZV91bnBhdXNlYCB3YXMgY2FsbGVkIHdpdGhvdXQgYSBwcmlvciBgc2NoZWR1bGVfdW5wYXVzZWAgKGlzc3VlICMyODIpLgAAAAAAABNVbnBhdXNlTm90U2NoZWR1bGVkAAAAAAcAAAERYHVwZGF0ZV93aXRoZHJhd2FsX2hhbmRsZXJgIHdhcyBjYWxsZWQgd2l0aCBhIGBuZXdfaGFuZGxlcmAgdGhhdCBtYXRjaGVzCnRoZSByb3V0ZXIgaXRzZWxmIG9yIG9uZSBvZiBpdHMgb3RoZXIgcmVnaXN0ZXJlZCBoYW5kbGVycyAoaXNzdWUgIzYzNSkg4oCUCmFsbW9zdCBjZXJ0YWlubHkgYSBjb3B5LXBhc3RlIG1pc3Rha2UgdGhhdCB3b3VsZCBzaWxlbnRseSBtaXNyb3V0ZQp3aXRoZHJhd2FsIGNhbGxzIHRvIGEgY29udHJhY3Qgd2l0aCBhIGRpZmZlcmVudCBpbnRlcmZhY2UuAAAAAAAAGEludmFsaWRXaXRoZHJhd2FsSGFuZGxlcgAAAAgAAADYY2xhaW1fZnVuZGluZ19mZWVzIChvciBtdWx0aWNhbGwncyBDbGFpbUZ1bmRpbmdGZWVzIGFjdGlvbikgd2FzIGNhbGxlZAp3aXRoIG1hcmtldHMubGVuKCkgIT0gdG9rZW5zLmxlbigpIChpc3N1ZSAjNjExKSDigJQgaW5kZXhpbmcgd291bGQKb3RoZXJ3aXNlIHBhbmljIHdpdGggYSByYXcgT3B0aW9uOjp1bndyYXAoKSBvbiBOb25lIHJhdGhlciB0aGFuIGEKdHlwZWQgZXJyb3IuAAAAFU1pc21hdGNoZWRCYXRjaExlbmd0aAAAAAAAAAkAAADDbXVsdGljYWxsJ3MgU2VuZFRva2VucyBhY3Rpb24gd2FzIGdpdmVuIGBhbW91bnQgPD0gMGAgKGlzc3VlICM3MzEpIOKAlApyZWplY3RlZCB1cCBmcm9udCB3aXRoIGEgdHlwZWQgZXJyb3IgaW5zdGVhZCBvZiBzdXJmYWNpbmcgd2hhdGV2ZXIgdGhlCnRva2VuIGNvbnRyYWN0IGRvZXMgd2l0aCBhIHplcm8gb3IgbmVnYXRpdmUgdHJhbnNmZXIuAAAAAA1JbnZhbGlkQW1vdW50AAAAAAAACg==",
        "AAAAAgAAADZFYWNoIGVsZW1lbnQgaW4gYSBtdWx0aWNhbGwgVmVjIGlzIG9uZSBhY3Rpb24gdmFyaWFudC4AAAAAAAAAAAAMUm91dGVyQWN0aW9uAAAACQAAAAEAAAAAAAAAClNlbmRUb2tlbnMAAAAAAAEAAAfQAAAAEFNlbmRUb2tlbnNQYXJhbXMAAAABAAAAAAAAAA1DcmVhdGVEZXBvc2l0AAAAAAAAAQAAB9AAAAATQ3JlYXRlRGVwb3NpdFBhcmFtcwAAAAABAAAAAAAAAA1DYW5jZWxEZXBvc2l0AAAAAAAAAQAAA+4AAAAgAAAAAQAAAAAAAAAQQ3JlYXRlV2l0aGRyYXdhbAAAAAEAAAfQAAAAFkNyZWF0ZVdpdGhkcmF3YWxQYXJhbXMAAAAAAAEAAAAAAAAAEENhbmNlbFdpdGhkcmF3YWwAAAABAAAD7gAAACAAAAABAAAAAAAAAAtDcmVhdGVPcmRlcgAAAAABAAAH0AAAABFDcmVhdGVPcmRlclBhcmFtcwAAAAAAAAEAAAAAAAAAC1VwZGF0ZU9yZGVyAAAAAAEAAAfQAAAAEVVwZGF0ZU9yZGVyUGFyYW1zAAAAAAAAAQAAAAAAAAALQ2FuY2VsT3JkZXIAAAAAAQAAA+4AAAAgAAAAAQAAAAAAAAAQQ2xhaW1GdW5kaW5nRmVlcwAAAAEAAAfQAAAAFkNsYWltRnVuZGluZ0ZlZXNQYXJhbXMAAA==",
        "AAAAAQAAAAAAAAAAAAAAEFNlbmRUb2tlbnNQYXJhbXMAAAADAAAAAAAAAAZhbW91bnQAAAAAAAsAAAAAAAAACHJlY2VpdmVyAAAAEwAAAAAAAAAFdG9rZW4AAAAAAAAT",
        "AAAAAQAAAAAAAAAAAAAAEVVwZGF0ZU9yZGVyUGFyYW1zAAAAAAAABQAAAAAAAAAQYWNjZXB0YWJsZV9wcmljZQAAAAsAAAAAAAAAA2tleQAAAAPuAAAAIAAAAAAAAAARbWluX291dHB1dF9hbW91bnQAAAAAAAALAAAAAAAAAA5zaXplX2RlbHRhX3VzZAAAAAAACwAAAAAAAAANdHJpZ2dlcl9wcmljZQAAAAAAAAs=",
        "AAAAAQAAAAAAAAAAAAAAFkNsYWltRnVuZGluZ0ZlZXNQYXJhbXMAAAAAAAIAAAAAAAAAB21hcmtldHMAAAAD6gAAABMAAAAAAAAABnRva2VucwAAAAAD6gAAABM=",
        "AAAAAAAAAD9VcGdyYWRlIHRoZSBjb250cmFjdCB3YXNtLiBPbmx5IHRoZSBzdG9yZWQgYWRtaW4gbWF5IGNhbGwgdGhpcy4AAAAAB3VwZ3JhZGUAAAAAAQAAAAAAAAANbmV3X3dhc21faGFzaAAAAAAAA+4AAAAgAAAAAA==",
        "AAAAAAAAActFeGVjdXRlIGEgYmF0Y2ggb2YgYWN0aW9ucyBhdG9taWNhbGx5LgoKU2luZ2xlIGNhbGxlci5yZXF1aXJlX2F1dGgoKSBjb3ZlcnMgYWxsIHN1Yi1hY3Rpb25zICh0aGV5IHJ1biBpbnNpZGUgdGhpcyBpbnZvY2F0aW9uKS4KUmV0dXJucyBvbmUgQnl0ZXNOPDMyPiByZXN1bHQgcGVyIGFjdGlvbiAoY3JlYXRlXyogcmV0dXJucyBhIGtleTsgb3RoZXJzIHJldHVybiB6ZXJvIGhhc2gpLgpJZiBhbnkgYWN0aW9uIHBhbmljcywgdGhlIGVudGlyZSB0cmFuc2FjdGlvbiByZXZlcnRzIChTb3JvYmFuIGF0b21pY2l0eSkuCgpIYW5kbGVycyBhcmUgY2FsbGVkIGRpcmVjdGx5IChub3QgdmlhIHRoZSBzZWxmLXJlZmVyZW50aWFsIHB1YmxpYyB3cmFwcGVycykgdG8gYXZvaWQgYQpkb3VibGUgcmVxdWlyZV9hdXRoKCkgd2l0aGluIHRoZSBzYW1lIGludm9jYXRpb24gZnJhbWUsIHdoaWNoIFNvcm9iYW4gcmVqZWN0cy4AAAAACW11bHRpY2FsbAAAAAAAAAIAAAAAAAAABmNhbGxlcgAAAAAAEwAAAAAAAAAHYWN0aW9ucwAAAAPqAAAH0AAAAAxSb3V0ZXJBY3Rpb24AAAABAAAD6gAAA+4AAAAg",
        "AAAAAAAAAC9PbmUtdGltZSBzZXR1cCDigJQgc3RvcmUgYWxsIGhhbmRsZXIgYWRkcmVzc2VzLgAAAAAKaW5pdGlhbGl6ZQAAAAAABwAAAAAAAAAFYWRtaW4AAAAAAAATAAAAAAAAAApyb2xlX3N0b3JlAAAAAAATAAAAAAAAAApkYXRhX3N0b3JlAAAAAAATAAAAAAAAAA9kZXBvc2l0X2hhbmRsZXIAAAAAEwAAAAAAAAASd2l0aGRyYXdhbF9oYW5kbGVyAAAAAAATAAAAAAAAAA1vcmRlcl9oYW5kbGVyAAAAAAAAEwAAAAAAAAALZmVlX2hhbmRsZXIAAAAAEwAAAAA=",
        "AAAAAAAAAKJQYXVzZSB0aGUgcHJvdG9jb2wgaW1tZWRpYXRlbHkuCgpSZS1wYXVzaW5nIGNsZWFycyBhbnkgcGVuZGluZyBgc2NoZWR1bGVfdW5wYXVzZWAgc28gZGVmZW5kZXJzIGNhbiByZXNldAp0aGUgdGltZWxvY2sgY2xvY2sgaWYgdGhlIHRocmVhdCByZXN1cmZhY2VzIChpc3N1ZSAjMjgyKS4AAAAAAApzZXRfcGF1c2VkAAAAAAABAAAAAAAAAAZwYXVzZWQAAAAAAAEAAAAA",
        "AAAAAAAAACpGb3J3YXJkIGNhbmNlbF9vcmRlciB0byB0aGUgb3JkZXJfaGFuZGxlci4AAAAAAAxjYW5jZWxfb3JkZXIAAAACAAAAAAAAAAZjYWxsZXIAAAAAABMAAAAAAAAAA2tleQAAAAPuAAAAIAAAAAA=",
        "AAAAAAAAACpGb3J3YXJkIHVwZGF0ZV9vcmRlciB0byB0aGUgb3JkZXJfaGFuZGxlci4AAAAAAAx1cGRhdGVfb3JkZXIAAAACAAAAAAAAAAZjYWxsZXIAAAAAABMAAAAAAAAABnBhcmFtcwAAAAAH0AAAABFVcGRhdGVPcmRlclBhcmFtcwAAAAAAAAA=",
        "AAAAAAAAAC5Gb3J3YXJkIGNhbmNlbF9kZXBvc2l0IHRvIHRoZSBkZXBvc2l0X2hhbmRsZXIuAAAAAAAOY2FuY2VsX2RlcG9zaXQAAAAAAAIAAAAAAAAABmNhbGxlcgAAAAAAEwAAAAAAAAADa2V5AAAAA+4AAAAgAAAAAA==",
        "AAAAAAAAAC5Gb3J3YXJkIGNyZWF0ZV9kZXBvc2l0IHRvIHRoZSBkZXBvc2l0X2hhbmRsZXIuAAAAAAAOY3JlYXRlX2RlcG9zaXQAAAAAAAIAAAAAAAAABmNhbGxlcgAAAAAAEwAAAAAAAAAGcGFyYW1zAAAAAAfQAAAAE0NyZWF0ZURlcG9zaXRQYXJhbXMAAAAAAQAAA+4AAAAg",
        "AAAAAAAAATlFeGVjdXRlIGEgcHJldmlvdXNseSBzY2hlZHVsZWQgdW5wYXVzZSBpZiB0aGUgdGltZWxvY2sgaGFzIGV4cGlyZWQgKGlzc3VlICMyODIpLgoKUmV2ZXJ0cyB3aXRoIGBUaW1lbG9ja05vdEV4cGlyZWRgIGlmIGNhbGxlZCBiZWZvcmUgdGhlIHNjaGVkdWxlZCBsZWRnZXIsCmFuZCB3aXRoIGBVbnBhdXNlTm90U2NoZWR1bGVkYCBpZiBgc2NoZWR1bGVfdW5wYXVzZWAgd2FzIG5ldmVyIGNhbGxlZCAob3Igd2FzCmNsZWFyZWQgYnkgYSByZS1wYXVzZSkuIE9uIHN1Y2Nlc3MsIGNsZWFycyBib3RoIHRoZSBwYXVzZSBmbGFnIGFuZCB0aGUgc2NoZWR1bGUuAAAAAAAAD2V4ZWN1dGVfdW5wYXVzZQAAAAAAAAAAAA==",
        "AAAAAAAAASFTY2hlZHVsZSBhbiB1bnBhdXNlIGFmdGVyIGBVTlBBVVNFX1RJTUVMT0NLX0xFREdFUlNgIGxlZGdlcnMgKGlzc3VlICMyODIpLgoKUmVjb3JkcyBgY3VycmVudF9sZWRnZXIgKyBVTlBBVVNFX1RJTUVMT0NLX0xFREdFUlNgIGFzIHRoZSBlYXJsaWVzdCBsZWRnZXIKYXQgd2hpY2ggYGV4ZWN1dGVfdW5wYXVzZWAgbWF5IHN1Y2NlZWQuIEVtaXRzIHRoZSBzY2hlZHVsZWQgbGVkZ2VyIGFzIGFuIGV2ZW50CnNvIG9mZi1jaGFpbiBtb25pdG9yaW5nIGNhbiBvYnNlcnZlIHRoZSBpbnRlbnQuIEFkbWluIG9ubHkuAAAAAAAAEHNjaGVkdWxlX3VucGF1c2UAAAAAAAAAAA==",
        "AAAAAAAAADRGb3J3YXJkIGNhbmNlbF93aXRoZHJhd2FsIHRvIHRoZSB3aXRoZHJhd2FsX2hhbmRsZXIuAAAAEWNhbmNlbF93aXRoZHJhd2FsAAAAAAAAAgAAAAAAAAAGY2FsbGVyAAAAAAATAAAAAAAAAANrZXkAAAAD7gAAACAAAAAA",
        "AAAAAAAAADRGb3J3YXJkIGNyZWF0ZV93aXRoZHJhd2FsIHRvIHRoZSB3aXRoZHJhd2FsX2hhbmRsZXIuAAAAEWNyZWF0ZV93aXRoZHJhd2FsAAAAAAAAAgAAAAAAAAAGY2FsbGVyAAAAAAATAAAAAAAAAAZwYXJhbXMAAAAAB9AAAAAWQ3JlYXRlV2l0aGRyYXdhbFBhcmFtcwAAAAAAAQAAA+4AAAAg",
        "AAAAAAAAAFBTZXQgdGhlIFVJIGZlZSBmYWN0b3IgZm9yIGEgcmVjZWl2ZXIuIERlbGVnYXRlcyBhdXRoIGVuZm9yY2VtZW50IHRvIGZlZV9oYW5kbGVyLgAAABFzZXRfdWlfZmVlX2ZhY3RvcgAAAAAAAAIAAAAAAAAAC3VpX3JlY2VpdmVyAAAAABMAAAAAAAAABmZhY3RvcgAAAAAACgAAAAA=",
        "AAAAAAAAAD5DbGFpbSBlYXJuZWQgZnVuZGluZyBmZWVzIGFjcm9zcyBtdWx0aXBsZSBtYXJrZXRzIGluIG9uZSBjYWxsLgAAAAAAEmNsYWltX2Z1bmRpbmdfZmVlcwAAAAAAAwAAAAAAAAAGY2FsbGVyAAAAAAATAAAAAAAAAAdtYXJrZXRzAAAAA+oAAAATAAAAAAAAAAZ0b2tlbnMAAAAAA+oAAAATAAAAAA==",
        "AAAAAAAAAERVcGRhdGUgdGhlIGZlZV9oYW5kbGVyIGFkZHJlc3MuIE9ubHkgdGhlIHN0b3JlZCBhZG1pbiBtYXkgY2FsbCB0aGlzLgAAABJ1cGRhdGVfZmVlX2hhbmRsZXIAAAAAAAIAAAAAAAAABmNhbGxlcgAAAAAAEwAAAAAAAAALbmV3X2hhbmRsZXIAAAAAEwAAAAA=",
        "AAAAAAAAAEdRdWVyeSB0aGUgY3VycmVudCBwb3NpdGlvbiBtYW5hZ2VyIGZvciBhbiBhY2NvdW50IG9uIGEgc3BlY2lmaWMgbWFya2V0LgAAAAAUZ2V0X3Bvc2l0aW9uX21hbmFnZXIAAAACAAAAAAAAAAVvd25lcgAAAAAAABMAAAAAAAAABm1hcmtldAAAAAAAEwAAAAEAAAPoAAAAEw==",
        "AAAAAAAAAVVTZXQgYSBwb3NpdGlvbiBtYW5hZ2VyIGZvciB0aGUgY2FsbGVyIG9uIGEgc3BlY2lmaWMgbWFya2V0LgoKQSBwb3NpdGlvbiBtYW5hZ2VyIGlzIGF1dGhvcml6ZWQgdG8gY3JlYXRlLCBpbmNyZWFzZSwgZGVjcmVhc2UsIG9yIGNsb3NlCnBvc2l0aW9ucyBvbiBiZWhhbGYgb2YgdGhlIG93bmVyLCBidXQgY2Fubm90IHJlZGlyZWN0IGNvbGxhdGVyYWwgcmVjZWlwdHMuClRoZSBtYW5hZ2VyIGNhbm5vdCBvdmVycmlkZSB0aGUgcmVjZWl2ZXIg4oCUIGZ1bmRzIGFsd2F5cyBnbyB0byB0aGUgb3duZXIuCgpSZXZva2UgYW4gZXhpc3RpbmcgbWFuYWdlciB3aXRoIHJlbW92ZV9wb3NpdGlvbl9tYW5hZ2VyLgAAAAAAABRzZXRfcG9zaXRpb25fbWFuYWdlcgAAAAMAAAAAAAAABmNhbGxlcgAAAAAAEwAAAAAAAAAGbWFya2V0AAAAAAATAAAAAAAAAAdtYW5hZ2VyAAAAABMAAAAA",
        "AAAAAAAAAEZVcGRhdGUgdGhlIG9yZGVyX2hhbmRsZXIgYWRkcmVzcy4gT25seSB0aGUgc3RvcmVkIGFkbWluIG1heSBjYWxsIHRoaXMuAAAAAAAUdXBkYXRlX29yZGVyX2hhbmRsZXIAAAACAAAAAAAAAAZjYWxsZXIAAAAAABMAAAAAAAAAC25ld19oYW5kbGVyAAAAABMAAAAA",
        "AAAAAAAAAAAAAAAVcmVzZXRfY2lyY3VpdF9icmVha2VyAAAAAAAAAQAAAAAAAAAGbWFya2V0AAAAAAATAAAAAA==",
        "AAAAAAAAANFVcGRhdGUgdGhlIGRlcG9zaXRfaGFuZGxlciBhZGRyZXNzLiBPbmx5IHRoZSBzdG9yZWQgYWRtaW4gbWF5IGNhbGwgdGhpcy4KUmVqZWN0cyB0aGUgcm91dGVyJ3Mgb3duIGFkZHJlc3MgYW5kIGl0cyBvdGhlciByZWdpc3RlcmVkIGhhbmRsZXIgYWRkcmVzc2VzCmFzIGEgYmFzaWMgc2FuaXR5IGNoZWNrIGFnYWluc3QgY29weS1wYXN0ZSBtaXNjb25maWd1cmF0aW9uLgAAAAAAABZ1cGRhdGVfZGVwb3NpdF9oYW5kbGVyAAAAAAACAAAAAAAAAAZjYWxsZXIAAAAAABMAAAAAAAAAC25ld19oYW5kbGVyAAAAABMAAAAA",
        "AAAAAAAAAIZSZXZva2UgdGhlIGNhbGxlcidzIHBvc2l0aW9uIG1hbmFnZXIgb24gYSBzcGVjaWZpYyBtYXJrZXQuIE9ubHkgdGhlCm93bmVyIGNhbiBjYWxsIHRoaXM7IGdldF9wb3NpdGlvbl9tYW5hZ2VyIHJldHVybnMgTm9uZSBhZnRlcndhcmRzLgAAAAAAF3JlbW92ZV9wb3NpdGlvbl9tYW5hZ2VyAAAAAAIAAAAAAAAABmNhbGxlcgAAAAAAEwAAAAAAAAAGbWFya2V0AAAAAAATAAAAAQAAAAE=",
        "AAAAAAAAAOFVcGRhdGUgdGhlIHdpdGhkcmF3YWxfaGFuZGxlciBhZGRyZXNzLiBPbmx5IHRoZSBzdG9yZWQgYWRtaW4gbWF5IGNhbGwgdGhpcy4KUmVqZWN0cyB0aGUgcm91dGVyJ3Mgb3duIGFkZHJlc3MgYW5kIGl0cyBvdGhlciByZWdpc3RlcmVkIGhhbmRsZXIgYWRkcmVzc2VzCmFzIGEgYmFzaWMgc2FuaXR5IGNoZWNrIGFnYWluc3QgY29weS1wYXN0ZSBtaXNjb25maWd1cmF0aW9uIChpc3N1ZSAjNjM1KS4AAAAAAAAZdXBkYXRlX3dpdGhkcmF3YWxfaGFuZGxlcgAAAAAAAAIAAAAAAAAABmNhbGxlcgAAAAAAEwAAAAAAAAALbmV3X2hhbmRsZXIAAAAAEwAAAAA=",
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
        multicall: this.txFromJSON<Array<Buffer>>,
        initialize: this.txFromJSON<null>,
        set_paused: this.txFromJSON<null>,
        cancel_order: this.txFromJSON<null>,
        update_order: this.txFromJSON<null>,
        cancel_deposit: this.txFromJSON<null>,
        create_deposit: this.txFromJSON<Buffer>,
        execute_unpause: this.txFromJSON<null>,
        schedule_unpause: this.txFromJSON<null>,
        cancel_withdrawal: this.txFromJSON<null>,
        create_withdrawal: this.txFromJSON<Buffer>,
        set_ui_fee_factor: this.txFromJSON<null>,
        claim_funding_fees: this.txFromJSON<null>,
        update_fee_handler: this.txFromJSON<null>,
        get_position_manager: this.txFromJSON<Option<string>>,
        set_position_manager: this.txFromJSON<null>,
        update_order_handler: this.txFromJSON<null>,
        reset_circuit_breaker: this.txFromJSON<null>,
        update_deposit_handler: this.txFromJSON<null>,
        remove_position_manager: this.txFromJSON<boolean>,
        update_withdrawal_handler: this.txFromJSON<null>
  }
}