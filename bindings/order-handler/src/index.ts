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
  4: {message:"OrderNotFound"},
  5: {message:"InvalidOrderType"},
  6: {message:"UnsatisfiedTrigger"},
  8: {message:"PriceTooLow"},
  9: {message:"OrderFrozen"},
  /**
   * Increase/swap orders require collateral to have been transferred to
   * order_vault (via exchange_router SendTokens) before calling create_order.
   * record_transfer_in returned zero, meaning no collateral arrived.
   */
  10: {message:"ZeroCollateral"},
  /**
   * A position order named `on_behalf_of`, but that owner has not
   * registered the caller as their position manager for the market
   * (issue #385/#535). Raised by both `create_order` and `create_orders`.
   */
  11: {message:"UnauthorizedPositionManager"},
  /**
   * `flag_stale_keeper` was called but the role's last activity is still
   * within the configured heartbeat timeout (issue #249).
   */
  12: {message:"KeeperNotStale"},
  /**
   * Position size/collateral ratio exceeds configured maximum leverage.
   */
  13: {message:"MaxLeverageExceeded"},
  /**
   * `create_orders` received more than 5 orders in a single batch.
   */
  14: {message:"BatchSizeLimitExceeded"},
  /**
   * The target market is paused due to circuit breaker (issue #203).
   */
  15: {message:"MarketPaused"},
  /**
   * swap_path contains a repeated market address — would corrupt pool accounting (issue #232).
   */
  16: {message:"CyclicSwapPath"},
  /**
   * swap_path exceeds the maximum allowed number of hops (issue #300).
   */
  17: {message:"SwapPathTooLong"},
  /**
   * execution_fee is below the configured global minimum (issue #294).
   */
  18: {message:"InsufficientExecutionFee"},
  /**
   * create_order called with size_delta_usd = 0 on a position order (issue #269).
   */
  19: {message:"ZeroSizeDelta"},
  /**
   * execute_order called after the order's user-set expiry_ledger (issue #272).
   * The order is auto-cancelled and any collateral refunded to the user.
   */
  20: {message:"OrderExpired"},
  /**
   * create_orders batch contains more than one increase/swap leg funded in the
   * same collateral token (issue #454) — record_transfer_in's shared per-token
   * balance delta can only unambiguously attribute one leg per token per batch.
   */
  21: {message:"DuplicateCollateralTokenInBatch"},
  /**
   * execute_adl called while the market/side's PnL-to-pool-value ratio does
   * not exceed the configured ADL threshold (issue #417). Re-validated here,
   * not just in the adl_handler wrapper, so the real mutating entry point
   * can't be forced by a caller who bypasses adl_handler entirely.
   */
  22: {message:"AdlRequirementNotMet"},
  /**
   * execute_adl called against a position that is not currently profitable
   * (issue #417) — ADL may only partially close profitable positions.
   */
  23: {message:"AdlPositionNotProfitable"},
  /**
   * Position increase would push the market/side's open interest above the
   * configured `max_open_interest` cap (issue #450).
   */
  24: {message:"MaxOpenInterestExceeded"},
  /**
   * set_keeper_heartbeat_timeout called with timeout_ledgers = 0 (issue #634)
   * — would make every keeper on the role appear stale after a single ledger.
   */
  25: {message:"InvalidHeartbeatTimeout"},
  /**
   * liquidate_position or execute_adl referenced a position_key with no
   * stored PositionProps (issue #642) — distinct from OrderNotFound, which
   * means no such *order* id, not no such position.
   */
  26: {message:"PositionNotFound"},
  /**
   * Issue #643: update_oracle called with the current oracle address
   * (no-op) or with this contract's own address / another of its stored
   * instance addresses (order_vault, data_store, role_store, admin,
   * referral_storage) — almost certainly a copy-paste mistake, not an
   * intentional rotation.
   */
  27: {message:"InvalidOracle"},
  /**
   * The supplied market address is not registered in data_store —
   * one or more of its token addresses (index/long/short) returned None.
   * Mirrors the InvalidMarket pattern in deposit_handler/withdrawal_handler
   * (issue #371) so callers can match this condition as a typed error
   * instead of a generic execution failure.
   */
  28: {message:"InvalidMarket"},
  /**
   * partial_liquidate_position referenced a position that is not currently
   * liquidatable (issue #533) — re-validated here, not just in the
   * liquidation_handler wrapper, so the real mutating entry point can't be
   * forced by a caller who bypasses liquidation_handler entirely (mirrors
   * AdlRequirementNotMet's rationale for execute_adl).
   */
  29: {message:"NotLiquidatable"},
  /**
   * flag_stale_keeper's keeper parameter does not actually hold role
   * (issue #794) — without this check, the emitted KeeperHeartbeatMissed
   * audit-trail event could name an arbitrary or unrelated address.
   */
  30: {message:"KeeperDoesNotHoldRole"},
  /**
   * The executing keeper's accumulated slash penalty has reached the
   * configured keeper_slash_threshold (issue #587).
   */
  31: {message:"KeeperSuspended"}
}

export type OrderStorageKey = {tag: "Order", values: readonly [Buffer]} | {tag: "OrderFrozen", values: readonly [Buffer]} | {tag: "OrderExpiry", values: readonly [Buffer]};

/**
 * Positions are stored in this contract's persistent storage under this key.
 * The #[contracttype] XDR encoding must match the one in increase/decrease_position_utils.
 */
export type PositionStorageKey = {tag: "Position", values: readonly [Buffer]};


export interface FundingRateSnapshot {
  funding_factor_per_second: i128;
  long_open_interest: u128;
  market: string;
  short_open_interest: u128;
  timestamp: u64;
}


export interface PositionClosedEvent {
  account: string;
  execution_price: i128;
  key: Buffer;
  market: string;
  pnl_usd: i128;
}


export interface PositionOpenedEvent {
  account: string;
  avg_entry_price: i128;
  collateral_amount: i128;
  is_long: boolean;
  key: Buffer;
  market: string;
  size_in_usd: i128;
}



export interface PositionDecreasedEvent {
  account: string;
  delta_size_usd: i128;
  execution_price: i128;
  key: Buffer;
  market: string;
  pnl_usd: i128;
}


export interface PositionIncreasedEvent {
  account: string;
  avg_entry_price: i128;
  delta_collateral: i128;
  delta_size_usd: i128;
  key: Buffer;
  market: string;
  new_size_usd: i128;
}


/**
 * Result of `liquidate_position` returned to `liquidation_handler` (issue #437).
 * A named struct (ScMap-encoded, keyed by field name) rather than a positional
 * tuple, so a future reorder of the fields on either side fails to decode
 * instead of silently swapping values across the two independently-deployed
 * contracts.
 */
export interface LiquidatePositionResult {
  execution_price: i128;
  keeper_execution_fee: i128;
  pnl_usd: i128;
}


export interface PositionLiquidatedEvent {
  account: string;
  execution_price: i128;
  /**
 * Collateral paid to the liquidation keeper as its execution fee (issue #437).
 */
keeper_execution_fee: i128;
  key: Buffer;
  market: string;
  /**
 * Realised PnL of the closed position (issue #437).
 */
pnl_usd: i128;
  remaining_collateral: i128;
}


/**
 * Result of `partial_liquidate_position` returned to `liquidation_handler`
 * (issue #533). `liquidated_size_usd` / `remaining_size_usd` reflect the
 * position's actual post-close state, computed from the same clamp
 * `decrease_position` itself applies, rather than a value liquidation_handler
 * derives independently and never confirms was actually applied.
 */
export interface PartialLiquidatePositionResult {
  execution_price: i128;
  keeper_execution_fee: i128;
  liquidated_size_usd: i128;
  pnl_usd: i128;
  remaining_size_usd: i128;
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
   * 
   * Storage layout (InstanceKey and PositionStorageKey / OrderStorageKey) must not
   * change between versions — existing persistent entries remain readable after upgrade.
   */
  upgrade: ({new_wasm_hash}: {new_wasm_hash: Buffer}, options?: MethodOptions) => Promise<AssembledTransaction<null>>

  /**
   * Construct and simulate a get_order transaction. Returns an `AssembledTransaction` object which will have a `result` field containing the result of the simulation. If this transaction changes contract state, you will need to call `signAndSend()` on the returned object.
   * Return a stored order by key, or None if not found.
   */
  get_order: ({key}: {key: Buffer}, options?: MethodOptions) => Promise<AssembledTransaction<Option<OrderProps>>>

  /**
   * Construct and simulate a initialize transaction. Returns an `AssembledTransaction` object which will have a `result` field containing the result of the simulation. If this transaction changes contract state, you will need to call `signAndSend()` on the returned object.
   * One-time setup.
   */
  initialize: ({admin, role_store, data_store, oracle, order_vault}: {admin: string, role_store: string, data_store: string, oracle: string, order_vault: string}, options?: MethodOptions) => Promise<AssembledTransaction<null>>

  /**
   * Construct and simulate a execute_adl transaction. Returns an `AssembledTransaction` object which will have a `result` field containing the result of the simulation. If this transaction changes contract state, you will need to call `signAndSend()` on the returned object.
   * Partially close a profitable position for ADL. Called by adl_handler after checks.
   * 
   * Issue #417: adl_handler performs these same checks before calling here, but
   * this contract grants the ADL_KEEPER role check directly and is independently
   * callable — so the real mutating entry point must re-validate its own
   * preconditions rather than trust the wrapper, exactly as `liquidate_position`
   * re-checks `is_liquidatable` instead of trusting `liquidation_handler`.
   */
  execute_adl: ({keeper, account, market, collateral_token, is_long, size_delta_usd}: {keeper: string, account: string, market: string, collateral_token: string, is_long: boolean, size_delta_usd: i128}, options?: MethodOptions) => Promise<AssembledTransaction<null>>

  /**
   * Construct and simulate a cancel_order transaction. Returns an `AssembledTransaction` object which will have a `result` field containing the result of the simulation. If this transaction changes contract state, you will need to call `signAndSend()` on the returned object.
   * Cancel a pending order and refund collateral to the account.
   */
  cancel_order: ({caller, key}: {caller: string, key: Buffer}, options?: MethodOptions) => Promise<AssembledTransaction<null>>

  /**
   * Construct and simulate a create_order transaction. Returns an `AssembledTransaction` object which will have a `result` field containing the result of the simulation. If this transaction changes contract state, you will need to call `signAndSend()` on the returned object.
   * Create a new order and record collateral in the order vault.
   * 
   * # Collateral model (canonical — issue #47)
   * 
   * **Chosen path:** the exchange_router is responsible for transferring
   * collateral from the caller to order_vault BEFORE invoking create_order.
   * order_handler then calls `record_transfer_in` to snapshot the delta.
   * 
   * **Why this model:**
   * - The router owns the auth context for the caller's token approval.
   * - Keeping the pull inside the router makes the vault a passive custodian
   * with no token-approval dependencies of its own.
   * - Handlers never hold approvals, so they cannot silently double-pull.
   * 
   * **Invariant enforced here:**
   * - For increase/swap orders: `record_transfer_in` delta MUST be > 0.
   * A zero delta means tokens were not pre-sent; the transaction reverts
   * with `ZeroCollateral` before any state is written.
   * - For decrease/stop-loss/liquidation orders: no collateral is deposited;
   * `collateral_delta_amount` comes from params (typically the existing position size).
   * 
   * **Multicall sequence the router enforces for incre
   */
  create_order: ({caller, params}: {caller: string, params: CreateOrderParams}, options?: MethodOptions) => Promise<AssembledTransaction<Buffer>>

  /**
   * Construct and simulate a freeze_order transaction. Returns an `AssembledTransaction` object which will have a `result` field containing the result of the simulation. If this transaction changes contract state, you will need to call `signAndSend()` on the returned object.
   * Freeze an order that cannot currently be executed.
   */
  freeze_order: ({keeper, key}: {keeper: string, key: Buffer}, options?: MethodOptions) => Promise<AssembledTransaction<null>>

  /**
   * Construct and simulate a get_position transaction. Returns an `AssembledTransaction` object which will have a `result` field containing the result of the simulation. If this transaction changes contract state, you will need to call `signAndSend()` on the returned object.
   * Return a stored position by its position_key (sha256 hash), or None.
   * Used by liquidation_handler and adl_handler to check position health.
   */
  get_position: ({key}: {key: Buffer}, options?: MethodOptions) => Promise<AssembledTransaction<Option<PositionProps>>>

  /**
   * Construct and simulate a update_order transaction. Returns an `AssembledTransaction` object which will have a `result` field containing the result of the simulation. If this transaction changes contract state, you will need to call `signAndSend()` on the returned object.
   * Update a pending order's trigger/acceptable price or size delta.
   */
  update_order: ({caller, key, size_delta_usd, acceptable_price, trigger_price, min_output_amount}: {caller: string, key: Buffer, size_delta_usd: i128, acceptable_price: i128, trigger_price: i128, min_output_amount: i128}, options?: MethodOptions) => Promise<AssembledTransaction<null>>

  /**
   * Construct and simulate a create_orders transaction. Returns an `AssembledTransaction` object which will have a `result` field containing the result of the simulation. If this transaction changes contract state, you will need to call `signAndSend()` on the returned object.
   * Create up to 5 orders atomically in a single call (issue #219).
   * 
   * All orders are created or none are (Soroban atomicity). For increase/swap orders,
   * the caller must pre-fund the order_vault via SendTokens before calling this.
   * `record_transfer_in` is called per increase/swap order to snapshot the delta.
   * 
   * At most one increase/swap leg per distinct `initial_collateral_token` is
   * supported per batch (issue #454): `record_transfer_in`'s balance delta is a
   * single running counter per token, so two legs sharing a token cannot be
   * unambiguously attributed. A batch violating this reverts up front with
   * `DuplicateCollateralTokenInBatch` instead of a confusing `ZeroCollateral`
   * panic partway through processing.
   * 
   * Returns the list of created order keys in the same order as `requests`.
   */
  create_orders: ({caller, requests}: {caller: string, requests: Array<CreateOrderParams>}, options?: MethodOptions) => Promise<AssembledTransaction<Array<Buffer>>>

  /**
   * Construct and simulate a execute_order transaction. Returns an `AssembledTransaction` object which will have a `result` field containing the result of the simulation. If this transaction changes contract state, you will need to call `signAndSend()` on the returned object.
   * Execute a pending order (called by keeper).
   */
  execute_order: ({keeper, key}: {keeper: string, key: Buffer}, options?: MethodOptions) => Promise<AssembledTransaction<null>>

  /**
   * Construct and simulate a update_oracle transaction. Returns an `AssembledTransaction` object which will have a `result` field containing the result of the simulation. If this transaction changes contract state, you will need to call `signAndSend()` on the returned object.
   */
  update_oracle: ({caller, new_oracle}: {caller: string, new_oracle: string}, options?: MethodOptions) => Promise<AssembledTransaction<null>>

  /**
   * Construct and simulate a bump_position_ttl transaction. Returns an `AssembledTransaction` object which will have a `result` field containing the result of the simulation. If this transaction changes contract state, you will need to call `signAndSend()` on the returned object.
   * Bump the TTL of a stored position by rewriting it to persistent storage.
   * Allows reader/view contracts to extend the lifetime of positions they access.
   */
  bump_position_ttl: ({caller, key}: {caller: string, key: Buffer}, options?: MethodOptions) => Promise<AssembledTransaction<boolean>>

  /**
   * Construct and simulate a flag_stale_keeper transaction. Returns an `AssembledTransaction` object which will have a `result` field containing the result of the simulation. If this transaction changes contract state, you will need to call `signAndSend()` on the returned object.
   * Flag a keeper as stale (issue #249). Admin-gated.
   * 
   * Verifies the `role`'s heartbeat has lapsed and emits `KeeperHeartbeatMissed`
   * so the staleness is recorded on-chain. The admin can then revoke the
   * keeper's role via `role_store::revoke_role` — which has no timelock, so the
   * replacement can be wired immediately. Panics with `KeeperNotStale` if the
   * role is still within its heartbeat window, preventing premature flagging.
   */
  flag_stale_keeper: ({caller, keeper, role}: {caller: string, keeper: string, role: Buffer}, options?: MethodOptions) => Promise<AssembledTransaction<null>>

  /**
   * Construct and simulate a liquidate_position transaction. Returns an `AssembledTransaction` object which will have a `result` field containing the result of the simulation. If this transaction changes contract state, you will need to call `signAndSend()` on the returned object.
   * Force-liquidate a position. Called by the liquidation_handler after role/health checks.
   * 
   * Positions live in order_handler storage, so liquidation must run here.
   * 
   * Returns a `LiquidatePositionResult` so liquidation_handler can include the
   * price, fee, and PnL in its own confirmation event (issue #437) without
   * re-deriving them or correlating a separate transfer event.
   */
  liquidate_position: ({keeper, account, market, collateral_token, is_long}: {keeper: string, account: string, market: string, collateral_token: string, is_long: boolean}, options?: MethodOptions) => Promise<AssembledTransaction<LiquidatePositionResult>>

  /**
   * Construct and simulate a set_referral_storage transaction. Returns an `AssembledTransaction` object which will have a `result` field containing the result of the simulation. If this transaction changes contract state, you will need to call `signAndSend()` on the returned object.
   * Register the referral_storage contract address (admin only — issue #217).
   * Once set, execute_order calls increment_referrer_volume after each trade.
   */
  set_referral_storage: ({caller, referral_storage}: {caller: string, referral_storage: string}, options?: MethodOptions) => Promise<AssembledTransaction<null>>

  /**
   * Construct and simulate a cleanup_expired_order transaction. Returns an `AssembledTransaction` object which will have a `result` field containing the result of the simulation. If this transaction changes contract state, you will need to call `signAndSend()` on the returned object.
   * Issue #272: cancel an order whose `expiry_ledger` has passed. Callable by
   * anyone; caller receives a small incentive fee from the order's execution_fee.
   */
  cleanup_expired_order: ({caller, key}: {caller: string, key: Buffer}, options?: MethodOptions) => Promise<AssembledTransaction<null>>

  /**
   * Construct and simulate a check_keeper_heartbeat transaction. Returns an `AssembledTransaction` object which will have a `result` field containing the result of the simulation. If this transaction changes contract state, you will need to call `signAndSend()` on the returned object.
   * Read a keeper role's liveness status from data_store (issue #249).
   * 
   * View-only. Returns the last-active ledger, the gap since then, and whether
   * that gap has exceeded the configured heartbeat timeout. A role that has
   * never recorded activity reports `last_active_ledger = 0` and is treated as
   * stale (its full lifetime exceeds any timeout).
   */
  check_keeper_heartbeat: ({data_store, role}: {data_store: string, role: Buffer}, options?: MethodOptions) => Promise<AssembledTransaction<KeeperHeartbeatStatus>>

  /**
   * Construct and simulate a partial_liquidate_position transaction. Returns an `AssembledTransaction` object which will have a `result` field containing the result of the simulation. If this transaction changes contract state, you will need to call `signAndSend()` on the returned object.
   * Partially close a liquidatable position (issue #533). Called by
   * liquidation_handler after role/health checks, mirroring `liquidate_position`
   * and `execute_adl`'s delegation pattern: positions live in order_handler
   * storage, so the actual size reduction — and the keeper execution fee, paid
   * the same way `liquidate_position` pays it — must happen here, not in the
   * wrapper. Unlike a full liquidation, the position is left open at its
   * reduced size rather than removed.
   */
  partial_liquidate_position: ({keeper, account, market, collateral_token, is_long, liquidation_factor_bps}: {keeper: string, account: string, market: string, collateral_token: string, is_long: boolean, liquidation_factor_bps: u128}, options?: MethodOptions) => Promise<AssembledTransaction<PartialLiquidatePositionResult>>

  /**
   * Construct and simulate a set_keeper_heartbeat_timeout transaction. Returns an `AssembledTransaction` object which will have a `result` field containing the result of the simulation. If this transaction changes contract state, you will need to call `signAndSend()` on the returned object.
   * Admin-configurable heartbeat timeout for a keeper `role`, in ledgers
   * (issue #249). When the gap since the role's last activity exceeds this,
   * the keeper is considered stale. Unset roles use
   * `DEFAULT_KEEPER_HEARTBEAT_TIMEOUT` (2880 ledgers, ~4h).
   */
  set_keeper_heartbeat_timeout: ({caller, role, timeout_ledgers}: {caller: string, role: Buffer, timeout_ledgers: u64}, options?: MethodOptions) => Promise<AssembledTransaction<null>>

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
      new ContractSpec([ "AAAABAAAAAAAAAAAAAAABUVycm9yAAAAAAAAHgAAAAAAAAASQWxyZWFkeUluaXRpYWxpemVkAAAAAAABAAAAAAAAAA5Ob3RJbml0aWFsaXplZAAAAAAAAgAAAAAAAAAMVW5hdXRob3JpemVkAAAAAwAAAAAAAAANT3JkZXJOb3RGb3VuZAAAAAAAAAQAAAAAAAAAEEludmFsaWRPcmRlclR5cGUAAAAFAAAAAAAAABJVbnNhdGlzZmllZFRyaWdnZXIAAAAAAAYAAAAAAAAAC1ByaWNlVG9vTG93AAAAAAgAAAAAAAAAC09yZGVyRnJvemVuAAAAAAkAAADOSW5jcmVhc2Uvc3dhcCBvcmRlcnMgcmVxdWlyZSBjb2xsYXRlcmFsIHRvIGhhdmUgYmVlbiB0cmFuc2ZlcnJlZCB0bwpvcmRlcl92YXVsdCAodmlhIGV4Y2hhbmdlX3JvdXRlciBTZW5kVG9rZW5zKSBiZWZvcmUgY2FsbGluZyBjcmVhdGVfb3JkZXIuCnJlY29yZF90cmFuc2Zlcl9pbiByZXR1cm5lZCB6ZXJvLCBtZWFuaW5nIG5vIGNvbGxhdGVyYWwgYXJyaXZlZC4AAAAAAA5aZXJvQ29sbGF0ZXJhbAAAAAAACgAAAMJBIHBvc2l0aW9uIG9yZGVyIG5hbWVkIGBvbl9iZWhhbGZfb2ZgLCBidXQgdGhhdCBvd25lciBoYXMgbm90CnJlZ2lzdGVyZWQgdGhlIGNhbGxlciBhcyB0aGVpciBwb3NpdGlvbiBtYW5hZ2VyIGZvciB0aGUgbWFya2V0Cihpc3N1ZSAjMzg1LyM1MzUpLiBSYWlzZWQgYnkgYm90aCBgY3JlYXRlX29yZGVyYCBhbmQgYGNyZWF0ZV9vcmRlcnNgLgAAAAAAG1VuYXV0aG9yaXplZFBvc2l0aW9uTWFuYWdlcgAAAAALAAAAemBmbGFnX3N0YWxlX2tlZXBlcmAgd2FzIGNhbGxlZCBidXQgdGhlIHJvbGUncyBsYXN0IGFjdGl2aXR5IGlzIHN0aWxsCndpdGhpbiB0aGUgY29uZmlndXJlZCBoZWFydGJlYXQgdGltZW91dCAoaXNzdWUgIzI0OSkuAAAAAAAOS2VlcGVyTm90U3RhbGUAAAAAAAwAAABDUG9zaXRpb24gc2l6ZS9jb2xsYXRlcmFsIHJhdGlvIGV4Y2VlZHMgY29uZmlndXJlZCBtYXhpbXVtIGxldmVyYWdlLgAAAAATTWF4TGV2ZXJhZ2VFeGNlZWRlZAAAAAANAAAAPmBjcmVhdGVfb3JkZXJzYCByZWNlaXZlZCBtb3JlIHRoYW4gNSBvcmRlcnMgaW4gYSBzaW5nbGUgYmF0Y2guAAAAAAAWQmF0Y2hTaXplTGltaXRFeGNlZWRlZAAAAAAADgAAAEBUaGUgdGFyZ2V0IG1hcmtldCBpcyBwYXVzZWQgZHVlIHRvIGNpcmN1aXQgYnJlYWtlciAoaXNzdWUgIzIwMykuAAAADE1hcmtldFBhdXNlZAAAAA8AAABcc3dhcF9wYXRoIGNvbnRhaW5zIGEgcmVwZWF0ZWQgbWFya2V0IGFkZHJlc3Mg4oCUIHdvdWxkIGNvcnJ1cHQgcG9vbCBhY2NvdW50aW5nIChpc3N1ZSAjMjMyKS4AAAAOQ3ljbGljU3dhcFBhdGgAAAAAABAAAABCc3dhcF9wYXRoIGV4Y2VlZHMgdGhlIG1heGltdW0gYWxsb3dlZCBudW1iZXIgb2YgaG9wcyAoaXNzdWUgIzMwMCkuAAAAAAAPU3dhcFBhdGhUb29Mb25nAAAAABEAAABCZXhlY3V0aW9uX2ZlZSBpcyBiZWxvdyB0aGUgY29uZmlndXJlZCBnbG9iYWwgbWluaW11bSAoaXNzdWUgIzI5NCkuAAAAAAAYSW5zdWZmaWNpZW50RXhlY3V0aW9uRmVlAAAAEgAAAE1jcmVhdGVfb3JkZXIgY2FsbGVkIHdpdGggc2l6ZV9kZWx0YV91c2QgPSAwIG9uIGEgcG9zaXRpb24gb3JkZXIgKGlzc3VlICMyNjkpLgAAAAAAAA1aZXJvU2l6ZURlbHRhAAAAAAAAEwAAAJBleGVjdXRlX29yZGVyIGNhbGxlZCBhZnRlciB0aGUgb3JkZXIncyB1c2VyLXNldCBleHBpcnlfbGVkZ2VyIChpc3N1ZSAjMjcyKS4KVGhlIG9yZGVyIGlzIGF1dG8tY2FuY2VsbGVkIGFuZCBhbnkgY29sbGF0ZXJhbCByZWZ1bmRlZCB0byB0aGUgdXNlci4AAAAMT3JkZXJFeHBpcmVkAAAAFAAAAONjcmVhdGVfb3JkZXJzIGJhdGNoIGNvbnRhaW5zIG1vcmUgdGhhbiBvbmUgaW5jcmVhc2Uvc3dhcCBsZWcgZnVuZGVkIGluIHRoZQpzYW1lIGNvbGxhdGVyYWwgdG9rZW4gKGlzc3VlICM0NTQpIOKAlCByZWNvcmRfdHJhbnNmZXJfaW4ncyBzaGFyZWQgcGVyLXRva2VuCmJhbGFuY2UgZGVsdGEgY2FuIG9ubHkgdW5hbWJpZ3VvdXNseSBhdHRyaWJ1dGUgb25lIGxlZyBwZXIgdG9rZW4gcGVyIGJhdGNoLgAAAAAfRHVwbGljYXRlQ29sbGF0ZXJhbFRva2VuSW5CYXRjaAAAAAAVAAABFWV4ZWN1dGVfYWRsIGNhbGxlZCB3aGlsZSB0aGUgbWFya2V0L3NpZGUncyBQbkwtdG8tcG9vbC12YWx1ZSByYXRpbyBkb2VzCm5vdCBleGNlZWQgdGhlIGNvbmZpZ3VyZWQgQURMIHRocmVzaG9sZCAoaXNzdWUgIzQxNykuIFJlLXZhbGlkYXRlZCBoZXJlLApub3QganVzdCBpbiB0aGUgYWRsX2hhbmRsZXIgd3JhcHBlciwgc28gdGhlIHJlYWwgbXV0YXRpbmcgZW50cnkgcG9pbnQKY2FuJ3QgYmUgZm9yY2VkIGJ5IGEgY2FsbGVyIHdobyBieXBhc3NlcyBhZGxfaGFuZGxlciBlbnRpcmVseS4AAAAAAAAUQWRsUmVxdWlyZW1lbnROb3RNZXQAAAAWAAAAimV4ZWN1dGVfYWRsIGNhbGxlZCBhZ2FpbnN0IGEgcG9zaXRpb24gdGhhdCBpcyBub3QgY3VycmVudGx5IHByb2ZpdGFibGUKKGlzc3VlICM0MTcpIOKAlCBBREwgbWF5IG9ubHkgcGFydGlhbGx5IGNsb3NlIHByb2ZpdGFibGUgcG9zaXRpb25zLgAAAAAAGEFkbFBvc2l0aW9uTm90UHJvZml0YWJsZQAAABcAAAB3UG9zaXRpb24gaW5jcmVhc2Ugd291bGQgcHVzaCB0aGUgbWFya2V0L3NpZGUncyBvcGVuIGludGVyZXN0IGFib3ZlIHRoZQpjb25maWd1cmVkIGBtYXhfb3Blbl9pbnRlcmVzdGAgY2FwIChpc3N1ZSAjNDUwKS4AAAAAF01heE9wZW5JbnRlcmVzdEV4Y2VlZGVkAAAAABgAAACVc2V0X2tlZXBlcl9oZWFydGJlYXRfdGltZW91dCBjYWxsZWQgd2l0aCB0aW1lb3V0X2xlZGdlcnMgPSAwIChpc3N1ZSAjNjM0KQrigJQgd291bGQgbWFrZSBldmVyeSBrZWVwZXIgb24gdGhlIHJvbGUgYXBwZWFyIHN0YWxlIGFmdGVyIGEgc2luZ2xlIGxlZGdlci4AAAAAAAAXSW52YWxpZEhlYXJ0YmVhdFRpbWVvdXQAAAAAGQAAALxsaXF1aWRhdGVfcG9zaXRpb24gb3IgZXhlY3V0ZV9hZGwgcmVmZXJlbmNlZCBhIHBvc2l0aW9uX2tleSB3aXRoIG5vCnN0b3JlZCBQb3NpdGlvblByb3BzIChpc3N1ZSAjNjQyKSDigJQgZGlzdGluY3QgZnJvbSBPcmRlck5vdEZvdW5kLCB3aGljaAptZWFucyBubyBzdWNoICpvcmRlciogaWQsIG5vdCBubyBzdWNoIHBvc2l0aW9uLgAAABBQb3NpdGlvbk5vdEZvdW5kAAAAGgAAAR5Jc3N1ZSAjNjQzOiB1cGRhdGVfb3JhY2xlIGNhbGxlZCB3aXRoIHRoZSBjdXJyZW50IG9yYWNsZSBhZGRyZXNzCihuby1vcCkgb3Igd2l0aCB0aGlzIGNvbnRyYWN0J3Mgb3duIGFkZHJlc3MgLyBhbm90aGVyIG9mIGl0cyBzdG9yZWQKaW5zdGFuY2UgYWRkcmVzc2VzIChvcmRlcl92YXVsdCwgZGF0YV9zdG9yZSwgcm9sZV9zdG9yZSwgYWRtaW4sCnJlZmVycmFsX3N0b3JhZ2UpIOKAlCBhbG1vc3QgY2VydGFpbmx5IGEgY29weS1wYXN0ZSBtaXN0YWtlLCBub3QgYW4KaW50ZW50aW9uYWwgcm90YXRpb24uAAAAAAANSW52YWxpZE9yYWNsZQAAAAAAABsAAAE2VGhlIHN1cHBsaWVkIG1hcmtldCBhZGRyZXNzIGlzIG5vdCByZWdpc3RlcmVkIGluIGRhdGFfc3RvcmUg4oCUCm9uZSBvciBtb3JlIG9mIGl0cyB0b2tlbiBhZGRyZXNzZXMgKGluZGV4L2xvbmcvc2hvcnQpIHJldHVybmVkIE5vbmUuCk1pcnJvcnMgdGhlIEludmFsaWRNYXJrZXQgcGF0dGVybiBpbiBkZXBvc2l0X2hhbmRsZXIvd2l0aGRyYXdhbF9oYW5kbGVyCihpc3N1ZSAjMzcxKSBzbyBjYWxsZXJzIGNhbiBtYXRjaCB0aGlzIGNvbmRpdGlvbiBhcyBhIHR5cGVkIGVycm9yCmluc3RlYWQgb2YgYSBnZW5lcmljIGV4ZWN1dGlvbiBmYWlsdXJlLgAAAAAADUludmFsaWRNYXJrZXQAAAAAAAAcAAABR3BhcnRpYWxfbGlxdWlkYXRlX3Bvc2l0aW9uIHJlZmVyZW5jZWQgYSBwb3NpdGlvbiB0aGF0IGlzIG5vdCBjdXJyZW50bHkKbGlxdWlkYXRhYmxlIChpc3N1ZSAjNTMzKSDigJQgcmUtdmFsaWRhdGVkIGhlcmUsIG5vdCBqdXN0IGluIHRoZQpsaXF1aWRhdGlvbl9oYW5kbGVyIHdyYXBwZXIsIHNvIHRoZSByZWFsIG11dGF0aW5nIGVudHJ5IHBvaW50IGNhbid0IGJlCmZvcmNlZCBieSBhIGNhbGxlciB3aG8gYnlwYXNzZXMgbGlxdWlkYXRpb25faGFuZGxlciBlbnRpcmVseSAobWlycm9ycwpBZGxSZXF1aXJlbWVudE5vdE1ldCdzIHJhdGlvbmFsZSBmb3IgZXhlY3V0ZV9hZGwpLgAAAAAPTm90TGlxdWlkYXRhYmxlAAAAAB0AAADHZmxhZ19zdGFsZV9rZWVwZXIncyBrZWVwZXIgcGFyYW1ldGVyIGRvZXMgbm90IGFjdHVhbGx5IGhvbGQgcm9sZQooaXNzdWUgIzc5NCkg4oCUIHdpdGhvdXQgdGhpcyBjaGVjaywgdGhlIGVtaXR0ZWQgS2VlcGVySGVhcnRiZWF0TWlzc2VkCmF1ZGl0LXRyYWlsIGV2ZW50IGNvdWxkIG5hbWUgYW4gYXJiaXRyYXJ5IG9yIHVucmVsYXRlZCBhZGRyZXNzLgAAAAAVS2VlcGVyRG9lc05vdEhvbGRSb2xlAAAAAAAAHgAAAHBUaGUgZXhlY3V0aW5nIGtlZXBlcidzIGFjY3VtdWxhdGVkIHNsYXNoIHBlbmFsdHkgaGFzIHJlYWNoZWQgdGhlCmNvbmZpZ3VyZWQga2VlcGVyX3NsYXNoX3RocmVzaG9sZCAoaXNzdWUgIzU4NykuAAAAD0tlZXBlclN1c3BlbmRlZAAAAAAf",
        "AAAAAgAAAAAAAAAAAAAAD09yZGVyU3RvcmFnZUtleQAAAAADAAAAAQAAAAAAAAAFT3JkZXIAAAAAAAABAAAD7gAAACAAAAABAAAAAAAAAAtPcmRlckZyb3plbgAAAAABAAAD7gAAACAAAAABAAAAT1Blci1vcmRlciBsZWRnZXItc2VxdWVuY2UgZXhwaXJ5IHNldCBieSB0aGUgdXNlciBhdCBjcmVhdGlvbiB0aW1lIChpc3N1ZSAjMjcyKS4AAAAAC09yZGVyRXhwaXJ5AAAAAAEAAAPuAAAAIA==",
        "AAAAAgAAAKNQb3NpdGlvbnMgYXJlIHN0b3JlZCBpbiB0aGlzIGNvbnRyYWN0J3MgcGVyc2lzdGVudCBzdG9yYWdlIHVuZGVyIHRoaXMga2V5LgpUaGUgI1tjb250cmFjdHR5cGVdIFhEUiBlbmNvZGluZyBtdXN0IG1hdGNoIHRoZSBvbmUgaW4gaW5jcmVhc2UvZGVjcmVhc2VfcG9zaXRpb25fdXRpbHMuAAAAAAAAAAASUG9zaXRpb25TdG9yYWdlS2V5AAAAAAABAAAAAQAAAAAAAAAIUG9zaXRpb24AAAABAAAD7gAAACA=",
        "AAAAAQAAAAAAAAAAAAAAE0Z1bmRpbmdSYXRlU25hcHNob3QAAAAABQAAAAAAAAAZZnVuZGluZ19mYWN0b3JfcGVyX3NlY29uZAAAAAAAAAsAAAAAAAAAEmxvbmdfb3Blbl9pbnRlcmVzdAAAAAAACgAAAAAAAAAGbWFya2V0AAAAAAATAAAAAAAAABNzaG9ydF9vcGVuX2ludGVyZXN0AAAAAAoAAAAAAAAACXRpbWVzdGFtcAAAAAAAAAY=",
        "AAAAAQAAAAAAAAAAAAAAE1Bvc2l0aW9uQ2xvc2VkRXZlbnQAAAAABQAAAAAAAAAHYWNjb3VudAAAAAATAAAAAAAAAA9leGVjdXRpb25fcHJpY2UAAAAACwAAAAAAAAADa2V5AAAAA+4AAAAgAAAAAAAAAAZtYXJrZXQAAAAAABMAAAAAAAAAB3BubF91c2QAAAAACw==",
        "AAAAAQAAAAAAAAAAAAAAE1Bvc2l0aW9uT3BlbmVkRXZlbnQAAAAABwAAAAAAAAAHYWNjb3VudAAAAAATAAAAAAAAAA9hdmdfZW50cnlfcHJpY2UAAAAACwAAAAAAAAARY29sbGF0ZXJhbF9hbW91bnQAAAAAAAALAAAAAAAAAAdpc19sb25nAAAAAAEAAAAAAAAAA2tleQAAAAPuAAAAIAAAAAAAAAAGbWFya2V0AAAAAAATAAAAAAAAAAtzaXplX2luX3VzZAAAAAAL",
        "AAAABQAAAP5FbWl0dGVkIHdoZW4gYSBrZWVwZXIgcm9sZSBpcyBmb3VuZCBzdGFsZSAoaXNzdWUgIzI0OSk6IHRoZSBnYXAgYmV0d2VlbiB0aGUKY3VycmVudCBsZWRnZXIgYW5kIHRoZSByb2xlJ3MgbGFzdCByZWNvcmRlZCBhY3Rpdml0eSBoYXMgZXhjZWVkZWQgdGhlCmNvbmZpZ3VyZWQgaGVhcnRiZWF0IHRpbWVvdXQuIFNpZ25hbHMgdGhlIGFkbWluIHRoYXQgdGhlIGtlZXBlciBoYXMgZ29uZQpzaWxlbnQgYW5kIGl0cyByb2xlIGNhbiBiZSByZXZva2VkLgAAAAAAAAAAABVLZWVwZXJIZWFydGJlYXRNaXNzZWQAAAAAAAABAAAACWtwcl9zdGFsZQAAAAAAAAQAAAAAAAAABHJvbGUAAAPuAAAAIAAAAAAAAAAAAAAABmtlZXBlcgAAAAAAEwAAAAAAAAAAAAAAC2xhc3RfbGVkZ2VyAAAAAAYAAAAAAAAAAAAAAA5jdXJyZW50X2xlZGdlcgAAAAAABgAAAAAAAAAC",
        "AAAAAQAAAAAAAAAAAAAAFlBvc2l0aW9uRGVjcmVhc2VkRXZlbnQAAAAAAAYAAAAAAAAAB2FjY291bnQAAAAAEwAAAAAAAAAOZGVsdGFfc2l6ZV91c2QAAAAAAAsAAAAAAAAAD2V4ZWN1dGlvbl9wcmljZQAAAAALAAAAAAAAAANrZXkAAAAD7gAAACAAAAAAAAAABm1hcmtldAAAAAAAEwAAAAAAAAAHcG5sX3VzZAAAAAAL",
        "AAAAAQAAAAAAAAAAAAAAFlBvc2l0aW9uSW5jcmVhc2VkRXZlbnQAAAAAAAcAAAAAAAAAB2FjY291bnQAAAAAEwAAAAAAAAAPYXZnX2VudHJ5X3ByaWNlAAAAAAsAAAAAAAAAEGRlbHRhX2NvbGxhdGVyYWwAAAALAAAAAAAAAA5kZWx0YV9zaXplX3VzZAAAAAAACwAAAAAAAAADa2V5AAAAA+4AAAAgAAAAAAAAAAZtYXJrZXQAAAAAABMAAAAAAAAADG5ld19zaXplX3VzZAAAAAs=",
        "AAAAAQAAAThSZXN1bHQgb2YgYGxpcXVpZGF0ZV9wb3NpdGlvbmAgcmV0dXJuZWQgdG8gYGxpcXVpZGF0aW9uX2hhbmRsZXJgIChpc3N1ZSAjNDM3KS4KQSBuYW1lZCBzdHJ1Y3QgKFNjTWFwLWVuY29kZWQsIGtleWVkIGJ5IGZpZWxkIG5hbWUpIHJhdGhlciB0aGFuIGEgcG9zaXRpb25hbAp0dXBsZSwgc28gYSBmdXR1cmUgcmVvcmRlciBvZiB0aGUgZmllbGRzIG9uIGVpdGhlciBzaWRlIGZhaWxzIHRvIGRlY29kZQppbnN0ZWFkIG9mIHNpbGVudGx5IHN3YXBwaW5nIHZhbHVlcyBhY3Jvc3MgdGhlIHR3byBpbmRlcGVuZGVudGx5LWRlcGxveWVkCmNvbnRyYWN0cy4AAAAAAAAAF0xpcXVpZGF0ZVBvc2l0aW9uUmVzdWx0AAAAAAMAAAAAAAAAD2V4ZWN1dGlvbl9wcmljZQAAAAALAAAAAAAAABRrZWVwZXJfZXhlY3V0aW9uX2ZlZQAAAAsAAAAAAAAAB3BubF91c2QAAAAACw==",
        "AAAAAQAAAAAAAAAAAAAAF1Bvc2l0aW9uTGlxdWlkYXRlZEV2ZW50AAAAAAcAAAAAAAAAB2FjY291bnQAAAAAEwAAAAAAAAAPZXhlY3V0aW9uX3ByaWNlAAAAAAsAAABMQ29sbGF0ZXJhbCBwYWlkIHRvIHRoZSBsaXF1aWRhdGlvbiBrZWVwZXIgYXMgaXRzIGV4ZWN1dGlvbiBmZWUgKGlzc3VlICM0MzcpLgAAABRrZWVwZXJfZXhlY3V0aW9uX2ZlZQAAAAsAAAAAAAAAA2tleQAAAAPuAAAAIAAAAAAAAAAGbWFya2V0AAAAAAATAAAAMVJlYWxpc2VkIFBuTCBvZiB0aGUgY2xvc2VkIHBvc2l0aW9uIChpc3N1ZSAjNDM3KS4AAAAAAAAHcG5sX3VzZAAAAAALAAAAAAAAABRyZW1haW5pbmdfY29sbGF0ZXJhbAAAAAs=",
        "AAAAAQAAAVtSZXN1bHQgb2YgYHBhcnRpYWxfbGlxdWlkYXRlX3Bvc2l0aW9uYCByZXR1cm5lZCB0byBgbGlxdWlkYXRpb25faGFuZGxlcmAKKGlzc3VlICM1MzMpLiBgbGlxdWlkYXRlZF9zaXplX3VzZGAgLyBgcmVtYWluaW5nX3NpemVfdXNkYCByZWZsZWN0IHRoZQpwb3NpdGlvbidzIGFjdHVhbCBwb3N0LWNsb3NlIHN0YXRlLCBjb21wdXRlZCBmcm9tIHRoZSBzYW1lIGNsYW1wCmBkZWNyZWFzZV9wb3NpdGlvbmAgaXRzZWxmIGFwcGxpZXMsIHJhdGhlciB0aGFuIGEgdmFsdWUgbGlxdWlkYXRpb25faGFuZGxlcgpkZXJpdmVzIGluZGVwZW5kZW50bHkgYW5kIG5ldmVyIGNvbmZpcm1zIHdhcyBhY3R1YWxseSBhcHBsaWVkLgAAAAAAAAAAHlBhcnRpYWxMaXF1aWRhdGVQb3NpdGlvblJlc3VsdAAAAAAABQAAAAAAAAAPZXhlY3V0aW9uX3ByaWNlAAAAAAsAAAAAAAAAFGtlZXBlcl9leGVjdXRpb25fZmVlAAAACwAAAAAAAAATbGlxdWlkYXRlZF9zaXplX3VzZAAAAAALAAAAAAAAAAdwbmxfdXNkAAAAAAsAAAAAAAAAEnJlbWFpbmluZ19zaXplX3VzZAAAAAAACw==",
        "AAAAAAAAAOZVcGdyYWRlIHRoZSBjb250cmFjdCB3YXNtLiBPbmx5IHRoZSBzdG9yZWQgYWRtaW4gbWF5IGNhbGwgdGhpcy4KClN0b3JhZ2UgbGF5b3V0IChJbnN0YW5jZUtleSBhbmQgUG9zaXRpb25TdG9yYWdlS2V5IC8gT3JkZXJTdG9yYWdlS2V5KSBtdXN0IG5vdApjaGFuZ2UgYmV0d2VlbiB2ZXJzaW9ucyDigJQgZXhpc3RpbmcgcGVyc2lzdGVudCBlbnRyaWVzIHJlbWFpbiByZWFkYWJsZSBhZnRlciB1cGdyYWRlLgAAAAAAB3VwZ3JhZGUAAAAAAQAAAAAAAAANbmV3X3dhc21faGFzaAAAAAAAA+4AAAAgAAAAAA==",
        "AAAAAAAAADNSZXR1cm4gYSBzdG9yZWQgb3JkZXIgYnkga2V5LCBvciBOb25lIGlmIG5vdCBmb3VuZC4AAAAACWdldF9vcmRlcgAAAAAAAAEAAAAAAAAAA2tleQAAAAPuAAAAIAAAAAEAAAPoAAAH0AAAAApPcmRlclByb3BzAAA=",
        "AAAAAAAAAA9PbmUtdGltZSBzZXR1cC4AAAAACmluaXRpYWxpemUAAAAAAAUAAAAAAAAABWFkbWluAAAAAAAAEwAAAAAAAAAKcm9sZV9zdG9yZQAAAAAAEwAAAAAAAAAKZGF0YV9zdG9yZQAAAAAAEwAAAAAAAAAGb3JhY2xlAAAAAAATAAAAAAAAAAtvcmRlcl92YXVsdAAAAAATAAAAAA==",
        "AAAAAAAAAcdQYXJ0aWFsbHkgY2xvc2UgYSBwcm9maXRhYmxlIHBvc2l0aW9uIGZvciBBREwuIENhbGxlZCBieSBhZGxfaGFuZGxlciBhZnRlciBjaGVja3MuCgpJc3N1ZSAjNDE3OiBhZGxfaGFuZGxlciBwZXJmb3JtcyB0aGVzZSBzYW1lIGNoZWNrcyBiZWZvcmUgY2FsbGluZyBoZXJlLCBidXQKdGhpcyBjb250cmFjdCBncmFudHMgdGhlIEFETF9LRUVQRVIgcm9sZSBjaGVjayBkaXJlY3RseSBhbmQgaXMgaW5kZXBlbmRlbnRseQpjYWxsYWJsZSDigJQgc28gdGhlIHJlYWwgbXV0YXRpbmcgZW50cnkgcG9pbnQgbXVzdCByZS12YWxpZGF0ZSBpdHMgb3duCnByZWNvbmRpdGlvbnMgcmF0aGVyIHRoYW4gdHJ1c3QgdGhlIHdyYXBwZXIsIGV4YWN0bHkgYXMgYGxpcXVpZGF0ZV9wb3NpdGlvbmAKcmUtY2hlY2tzIGBpc19saXF1aWRhdGFibGVgIGluc3RlYWQgb2YgdHJ1c3RpbmcgYGxpcXVpZGF0aW9uX2hhbmRsZXJgLgAAAAALZXhlY3V0ZV9hZGwAAAAABgAAAAAAAAAGa2VlcGVyAAAAAAATAAAAAAAAAAdhY2NvdW50AAAAABMAAAAAAAAABm1hcmtldAAAAAAAEwAAAAAAAAAQY29sbGF0ZXJhbF90b2tlbgAAABMAAAAAAAAAB2lzX2xvbmcAAAAAAQAAAAAAAAAOc2l6ZV9kZWx0YV91c2QAAAAAAAsAAAAA",
        "AAAAAAAAADxDYW5jZWwgYSBwZW5kaW5nIG9yZGVyIGFuZCByZWZ1bmQgY29sbGF0ZXJhbCB0byB0aGUgYWNjb3VudC4AAAAMY2FuY2VsX29yZGVyAAAAAgAAAAAAAAAGY2FsbGVyAAAAAAATAAAAAAAAAANrZXkAAAAD7gAAACAAAAAA",
        "AAAAAAAABABDcmVhdGUgYSBuZXcgb3JkZXIgYW5kIHJlY29yZCBjb2xsYXRlcmFsIGluIHRoZSBvcmRlciB2YXVsdC4KCiMgQ29sbGF0ZXJhbCBtb2RlbCAoY2Fub25pY2FsIOKAlCBpc3N1ZSAjNDcpCgoqKkNob3NlbiBwYXRoOioqIHRoZSBleGNoYW5nZV9yb3V0ZXIgaXMgcmVzcG9uc2libGUgZm9yIHRyYW5zZmVycmluZwpjb2xsYXRlcmFsIGZyb20gdGhlIGNhbGxlciB0byBvcmRlcl92YXVsdCBCRUZPUkUgaW52b2tpbmcgY3JlYXRlX29yZGVyLgpvcmRlcl9oYW5kbGVyIHRoZW4gY2FsbHMgYHJlY29yZF90cmFuc2Zlcl9pbmAgdG8gc25hcHNob3QgdGhlIGRlbHRhLgoKKipXaHkgdGhpcyBtb2RlbDoqKgotIFRoZSByb3V0ZXIgb3ducyB0aGUgYXV0aCBjb250ZXh0IGZvciB0aGUgY2FsbGVyJ3MgdG9rZW4gYXBwcm92YWwuCi0gS2VlcGluZyB0aGUgcHVsbCBpbnNpZGUgdGhlIHJvdXRlciBtYWtlcyB0aGUgdmF1bHQgYSBwYXNzaXZlIGN1c3RvZGlhbgp3aXRoIG5vIHRva2VuLWFwcHJvdmFsIGRlcGVuZGVuY2llcyBvZiBpdHMgb3duLgotIEhhbmRsZXJzIG5ldmVyIGhvbGQgYXBwcm92YWxzLCBzbyB0aGV5IGNhbm5vdCBzaWxlbnRseSBkb3VibGUtcHVsbC4KCioqSW52YXJpYW50IGVuZm9yY2VkIGhlcmU6KioKLSBGb3IgaW5jcmVhc2Uvc3dhcCBvcmRlcnM6IGByZWNvcmRfdHJhbnNmZXJfaW5gIGRlbHRhIE1VU1QgYmUgPiAwLgpBIHplcm8gZGVsdGEgbWVhbnMgdG9rZW5zIHdlcmUgbm90IHByZS1zZW50OyB0aGUgdHJhbnNhY3Rpb24gcmV2ZXJ0cwp3aXRoIGBaZXJvQ29sbGF0ZXJhbGAgYmVmb3JlIGFueSBzdGF0ZSBpcyB3cml0dGVuLgotIEZvciBkZWNyZWFzZS9zdG9wLWxvc3MvbGlxdWlkYXRpb24gb3JkZXJzOiBubyBjb2xsYXRlcmFsIGlzIGRlcG9zaXRlZDsKYGNvbGxhdGVyYWxfZGVsdGFfYW1vdW50YCBjb21lcyBmcm9tIHBhcmFtcyAodHlwaWNhbGx5IHRoZSBleGlzdGluZyBwb3NpdGlvbiBzaXplKS4KCioqTXVsdGljYWxsIHNlcXVlbmNlIHRoZSByb3V0ZXIgZW5mb3JjZXMgZm9yIGluY3JlAAAADGNyZWF0ZV9vcmRlcgAAAAIAAAAAAAAABmNhbGxlcgAAAAAAEwAAAAAAAAAGcGFyYW1zAAAAAAfQAAAAEUNyZWF0ZU9yZGVyUGFyYW1zAAAAAAAAAQAAA+4AAAAg",
        "AAAAAAAAADJGcmVlemUgYW4gb3JkZXIgdGhhdCBjYW5ub3QgY3VycmVudGx5IGJlIGV4ZWN1dGVkLgAAAAAADGZyZWV6ZV9vcmRlcgAAAAIAAAAAAAAABmtlZXBlcgAAAAAAEwAAAAAAAAADa2V5AAAAA+4AAAAgAAAAAA==",
        "AAAAAAAAAIpSZXR1cm4gYSBzdG9yZWQgcG9zaXRpb24gYnkgaXRzIHBvc2l0aW9uX2tleSAoc2hhMjU2IGhhc2gpLCBvciBOb25lLgpVc2VkIGJ5IGxpcXVpZGF0aW9uX2hhbmRsZXIgYW5kIGFkbF9oYW5kbGVyIHRvIGNoZWNrIHBvc2l0aW9uIGhlYWx0aC4AAAAAAAxnZXRfcG9zaXRpb24AAAABAAAAAAAAAANrZXkAAAAD7gAAACAAAAABAAAD6AAAB9AAAAANUG9zaXRpb25Qcm9wcwAAAA==",
        "AAAAAAAAAEBVcGRhdGUgYSBwZW5kaW5nIG9yZGVyJ3MgdHJpZ2dlci9hY2NlcHRhYmxlIHByaWNlIG9yIHNpemUgZGVsdGEuAAAADHVwZGF0ZV9vcmRlcgAAAAYAAAAAAAAABmNhbGxlcgAAAAAAEwAAAAAAAAADa2V5AAAAA+4AAAAgAAAAAAAAAA5zaXplX2RlbHRhX3VzZAAAAAAACwAAAAAAAAAQYWNjZXB0YWJsZV9wcmljZQAAAAsAAAAAAAAADXRyaWdnZXJfcHJpY2UAAAAAAAALAAAAAAAAABFtaW5fb3V0cHV0X2Ftb3VudAAAAAAAAAsAAAAA",
        "AAAAAAAAAwdDcmVhdGUgdXAgdG8gNSBvcmRlcnMgYXRvbWljYWxseSBpbiBhIHNpbmdsZSBjYWxsIChpc3N1ZSAjMjE5KS4KCkFsbCBvcmRlcnMgYXJlIGNyZWF0ZWQgb3Igbm9uZSBhcmUgKFNvcm9iYW4gYXRvbWljaXR5KS4gRm9yIGluY3JlYXNlL3N3YXAgb3JkZXJzLAp0aGUgY2FsbGVyIG11c3QgcHJlLWZ1bmQgdGhlIG9yZGVyX3ZhdWx0IHZpYSBTZW5kVG9rZW5zIGJlZm9yZSBjYWxsaW5nIHRoaXMuCmByZWNvcmRfdHJhbnNmZXJfaW5gIGlzIGNhbGxlZCBwZXIgaW5jcmVhc2Uvc3dhcCBvcmRlciB0byBzbmFwc2hvdCB0aGUgZGVsdGEuCgpBdCBtb3N0IG9uZSBpbmNyZWFzZS9zd2FwIGxlZyBwZXIgZGlzdGluY3QgYGluaXRpYWxfY29sbGF0ZXJhbF90b2tlbmAgaXMKc3VwcG9ydGVkIHBlciBiYXRjaCAoaXNzdWUgIzQ1NCk6IGByZWNvcmRfdHJhbnNmZXJfaW5gJ3MgYmFsYW5jZSBkZWx0YSBpcyBhCnNpbmdsZSBydW5uaW5nIGNvdW50ZXIgcGVyIHRva2VuLCBzbyB0d28gbGVncyBzaGFyaW5nIGEgdG9rZW4gY2Fubm90IGJlCnVuYW1iaWd1b3VzbHkgYXR0cmlidXRlZC4gQSBiYXRjaCB2aW9sYXRpbmcgdGhpcyByZXZlcnRzIHVwIGZyb250IHdpdGgKYER1cGxpY2F0ZUNvbGxhdGVyYWxUb2tlbkluQmF0Y2hgIGluc3RlYWQgb2YgYSBjb25mdXNpbmcgYFplcm9Db2xsYXRlcmFsYApwYW5pYyBwYXJ0d2F5IHRocm91Z2ggcHJvY2Vzc2luZy4KClJldHVybnMgdGhlIGxpc3Qgb2YgY3JlYXRlZCBvcmRlciBrZXlzIGluIHRoZSBzYW1lIG9yZGVyIGFzIGByZXF1ZXN0c2AuAAAAAA1jcmVhdGVfb3JkZXJzAAAAAAAAAgAAAAAAAAAGY2FsbGVyAAAAAAATAAAAAAAAAAhyZXF1ZXN0cwAAA+oAAAfQAAAAEUNyZWF0ZU9yZGVyUGFyYW1zAAAAAAAAAQAAA+oAAAPuAAAAIA==",
        "AAAAAAAAACtFeGVjdXRlIGEgcGVuZGluZyBvcmRlciAoY2FsbGVkIGJ5IGtlZXBlcikuAAAAAA1leGVjdXRlX29yZGVyAAAAAAAAAgAAAAAAAAAGa2VlcGVyAAAAAAATAAAAAAAAAANrZXkAAAAD7gAAACAAAAAA",
        "AAAAAAAAAAAAAAANdXBkYXRlX29yYWNsZQAAAAAAAAIAAAAAAAAABmNhbGxlcgAAAAAAEwAAAAAAAAAKbmV3X29yYWNsZQAAAAAAEwAAAAA=",
        "AAAAAAAAAJZCdW1wIHRoZSBUVEwgb2YgYSBzdG9yZWQgcG9zaXRpb24gYnkgcmV3cml0aW5nIGl0IHRvIHBlcnNpc3RlbnQgc3RvcmFnZS4KQWxsb3dzIHJlYWRlci92aWV3IGNvbnRyYWN0cyB0byBleHRlbmQgdGhlIGxpZmV0aW1lIG9mIHBvc2l0aW9ucyB0aGV5IGFjY2Vzcy4AAAAAABFidW1wX3Bvc2l0aW9uX3R0bAAAAAAAAAIAAAAAAAAABmNhbGxlcgAAAAAAEwAAAAAAAAADa2V5AAAAA+4AAAAgAAAAAQAAAAE=",
        "AAAAAAAAAaZGbGFnIGEga2VlcGVyIGFzIHN0YWxlIChpc3N1ZSAjMjQ5KS4gQWRtaW4tZ2F0ZWQuCgpWZXJpZmllcyB0aGUgYHJvbGVgJ3MgaGVhcnRiZWF0IGhhcyBsYXBzZWQgYW5kIGVtaXRzIGBLZWVwZXJIZWFydGJlYXRNaXNzZWRgCnNvIHRoZSBzdGFsZW5lc3MgaXMgcmVjb3JkZWQgb24tY2hhaW4uIFRoZSBhZG1pbiBjYW4gdGhlbiByZXZva2UgdGhlCmtlZXBlcidzIHJvbGUgdmlhIGByb2xlX3N0b3JlOjpyZXZva2Vfcm9sZWAg4oCUIHdoaWNoIGhhcyBubyB0aW1lbG9jaywgc28gdGhlCnJlcGxhY2VtZW50IGNhbiBiZSB3aXJlZCBpbW1lZGlhdGVseS4gUGFuaWNzIHdpdGggYEtlZXBlck5vdFN0YWxlYCBpZiB0aGUKcm9sZSBpcyBzdGlsbCB3aXRoaW4gaXRzIGhlYXJ0YmVhdCB3aW5kb3csIHByZXZlbnRpbmcgcHJlbWF0dXJlIGZsYWdnaW5nLgAAAAAAEWZsYWdfc3RhbGVfa2VlcGVyAAAAAAAAAwAAAAAAAAAGY2FsbGVyAAAAAAATAAAAAAAAAAZrZWVwZXIAAAAAABMAAAAAAAAABHJvbGUAAAPuAAAAIAAAAAA=",
        "AAAAAAAAAW1Gb3JjZS1saXF1aWRhdGUgYSBwb3NpdGlvbi4gQ2FsbGVkIGJ5IHRoZSBsaXF1aWRhdGlvbl9oYW5kbGVyIGFmdGVyIHJvbGUvaGVhbHRoIGNoZWNrcy4KClBvc2l0aW9ucyBsaXZlIGluIG9yZGVyX2hhbmRsZXIgc3RvcmFnZSwgc28gbGlxdWlkYXRpb24gbXVzdCBydW4gaGVyZS4KClJldHVybnMgYSBgTGlxdWlkYXRlUG9zaXRpb25SZXN1bHRgIHNvIGxpcXVpZGF0aW9uX2hhbmRsZXIgY2FuIGluY2x1ZGUgdGhlCnByaWNlLCBmZWUsIGFuZCBQbkwgaW4gaXRzIG93biBjb25maXJtYXRpb24gZXZlbnQgKGlzc3VlICM0MzcpIHdpdGhvdXQKcmUtZGVyaXZpbmcgdGhlbSBvciBjb3JyZWxhdGluZyBhIHNlcGFyYXRlIHRyYW5zZmVyIGV2ZW50LgAAAAAAABJsaXF1aWRhdGVfcG9zaXRpb24AAAAAAAUAAAAAAAAABmtlZXBlcgAAAAAAEwAAAAAAAAAHYWNjb3VudAAAAAATAAAAAAAAAAZtYXJrZXQAAAAAABMAAAAAAAAAEGNvbGxhdGVyYWxfdG9rZW4AAAATAAAAAAAAAAdpc19sb25nAAAAAAEAAAABAAAH0AAAABdMaXF1aWRhdGVQb3NpdGlvblJlc3VsdAA=",
        "AAAAAAAAAJVSZWdpc3RlciB0aGUgcmVmZXJyYWxfc3RvcmFnZSBjb250cmFjdCBhZGRyZXNzIChhZG1pbiBvbmx5IOKAlCBpc3N1ZSAjMjE3KS4KT25jZSBzZXQsIGV4ZWN1dGVfb3JkZXIgY2FsbHMgaW5jcmVtZW50X3JlZmVycmVyX3ZvbHVtZSBhZnRlciBlYWNoIHRyYWRlLgAAAAAAABRzZXRfcmVmZXJyYWxfc3RvcmFnZQAAAAIAAAAAAAAABmNhbGxlcgAAAAAAEwAAAAAAAAAQcmVmZXJyYWxfc3RvcmFnZQAAABMAAAAA",
        "AAAAAAAAAJdJc3N1ZSAjMjcyOiBjYW5jZWwgYW4gb3JkZXIgd2hvc2UgYGV4cGlyeV9sZWRnZXJgIGhhcyBwYXNzZWQuIENhbGxhYmxlIGJ5CmFueW9uZTsgY2FsbGVyIHJlY2VpdmVzIGEgc21hbGwgaW5jZW50aXZlIGZlZSBmcm9tIHRoZSBvcmRlcidzIGV4ZWN1dGlvbl9mZWUuAAAAABVjbGVhbnVwX2V4cGlyZWRfb3JkZXIAAAAAAAACAAAAAAAAAAZjYWxsZXIAAAAAABMAAAAAAAAAA2tleQAAAAPuAAAAIAAAAAA=",
        "AAAAAAAAAVBSZWFkIGEga2VlcGVyIHJvbGUncyBsaXZlbmVzcyBzdGF0dXMgZnJvbSBkYXRhX3N0b3JlIChpc3N1ZSAjMjQ5KS4KClZpZXctb25seS4gUmV0dXJucyB0aGUgbGFzdC1hY3RpdmUgbGVkZ2VyLCB0aGUgZ2FwIHNpbmNlIHRoZW4sIGFuZCB3aGV0aGVyCnRoYXQgZ2FwIGhhcyBleGNlZWRlZCB0aGUgY29uZmlndXJlZCBoZWFydGJlYXQgdGltZW91dC4gQSByb2xlIHRoYXQgaGFzCm5ldmVyIHJlY29yZGVkIGFjdGl2aXR5IHJlcG9ydHMgYGxhc3RfYWN0aXZlX2xlZGdlciA9IDBgIGFuZCBpcyB0cmVhdGVkIGFzCnN0YWxlIChpdHMgZnVsbCBsaWZldGltZSBleGNlZWRzIGFueSB0aW1lb3V0KS4AAAAWY2hlY2tfa2VlcGVyX2hlYXJ0YmVhdAAAAAAAAgAAAAAAAAAKZGF0YV9zdG9yZQAAAAAAEwAAAAAAAAAEcm9sZQAAA+4AAAAgAAAAAQAAB9AAAAAVS2VlcGVySGVhcnRiZWF0U3RhdHVzAAAA",
        "AAAAAAAAAdNQYXJ0aWFsbHkgY2xvc2UgYSBsaXF1aWRhdGFibGUgcG9zaXRpb24gKGlzc3VlICM1MzMpLiBDYWxsZWQgYnkKbGlxdWlkYXRpb25faGFuZGxlciBhZnRlciByb2xlL2hlYWx0aCBjaGVja3MsIG1pcnJvcmluZyBgbGlxdWlkYXRlX3Bvc2l0aW9uYAphbmQgYGV4ZWN1dGVfYWRsYCdzIGRlbGVnYXRpb24gcGF0dGVybjogcG9zaXRpb25zIGxpdmUgaW4gb3JkZXJfaGFuZGxlcgpzdG9yYWdlLCBzbyB0aGUgYWN0dWFsIHNpemUgcmVkdWN0aW9uIOKAlCBhbmQgdGhlIGtlZXBlciBleGVjdXRpb24gZmVlLCBwYWlkCnRoZSBzYW1lIHdheSBgbGlxdWlkYXRlX3Bvc2l0aW9uYCBwYXlzIGl0IOKAlCBtdXN0IGhhcHBlbiBoZXJlLCBub3QgaW4gdGhlCndyYXBwZXIuIFVubGlrZSBhIGZ1bGwgbGlxdWlkYXRpb24sIHRoZSBwb3NpdGlvbiBpcyBsZWZ0IG9wZW4gYXQgaXRzCnJlZHVjZWQgc2l6ZSByYXRoZXIgdGhhbiByZW1vdmVkLgAAAAAacGFydGlhbF9saXF1aWRhdGVfcG9zaXRpb24AAAAAAAYAAAAAAAAABmtlZXBlcgAAAAAAEwAAAAAAAAAHYWNjb3VudAAAAAATAAAAAAAAAAZtYXJrZXQAAAAAABMAAAAAAAAAEGNvbGxhdGVyYWxfdG9rZW4AAAATAAAAAAAAAAdpc19sb25nAAAAAAEAAAAAAAAAFmxpcXVpZGF0aW9uX2ZhY3Rvcl9icHMAAAAAAAoAAAABAAAH0AAAAB5QYXJ0aWFsTGlxdWlkYXRlUG9zaXRpb25SZXN1bHQAAA==",
        "AAAAAAAAAPRBZG1pbi1jb25maWd1cmFibGUgaGVhcnRiZWF0IHRpbWVvdXQgZm9yIGEga2VlcGVyIGByb2xlYCwgaW4gbGVkZ2VycwooaXNzdWUgIzI0OSkuIFdoZW4gdGhlIGdhcCBzaW5jZSB0aGUgcm9sZSdzIGxhc3QgYWN0aXZpdHkgZXhjZWVkcyB0aGlzLAp0aGUga2VlcGVyIGlzIGNvbnNpZGVyZWQgc3RhbGUuIFVuc2V0IHJvbGVzIHVzZQpgREVGQVVMVF9LRUVQRVJfSEVBUlRCRUFUX1RJTUVPVVRgICgyODgwIGxlZGdlcnMsIH40aCkuAAAAHHNldF9rZWVwZXJfaGVhcnRiZWF0X3RpbWVvdXQAAAADAAAAAAAAAAZjYWxsZXIAAAAAABMAAAAAAAAABHJvbGUAAAPuAAAAIAAAAAAAAAAPdGltZW91dF9sZWRnZXJzAAAAAAYAAAAA",
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
        initialize: this.txFromJSON<null>,
        execute_adl: this.txFromJSON<null>,
        cancel_order: this.txFromJSON<null>,
        create_order: this.txFromJSON<Buffer>,
        freeze_order: this.txFromJSON<null>,
        get_position: this.txFromJSON<Option<PositionProps>>,
        update_order: this.txFromJSON<null>,
        create_orders: this.txFromJSON<Array<Buffer>>,
        execute_order: this.txFromJSON<null>,
        update_oracle: this.txFromJSON<null>,
        bump_position_ttl: this.txFromJSON<boolean>,
        flag_stale_keeper: this.txFromJSON<null>,
        liquidate_position: this.txFromJSON<LiquidatePositionResult>,
        set_referral_storage: this.txFromJSON<null>,
        cleanup_expired_order: this.txFromJSON<null>,
        check_keeper_heartbeat: this.txFromJSON<KeeperHeartbeatStatus>,
        partial_liquidate_position: this.txFromJSON<PartialLiquidatePositionResult>,
        set_keeper_heartbeat_timeout: this.txFromJSON<null>
  }
}