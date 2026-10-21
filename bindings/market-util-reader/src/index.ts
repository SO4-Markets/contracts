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
  /**
   * The supplied market address is not registered in data_store —
   * one or more of its token addresses (index/long/short) returned None.
   * Mirrors the InvalidMarket pattern in deposit_handler/withdrawal_handler
   * (issue #371) so callers can match this condition as a typed error
   * instead of a generic execution failure.
   */
  1: {message:"InvalidMarket"}
}


export interface MarketUtilisation {
  combined_utilisation_bps: u32;
  is_at_long_oi_cap: boolean;
  is_at_short_oi_cap: boolean;
  long_open_interest_usd: u128;
  long_utilisation_bps: u32;
  pool_value_usd: u128;
  short_open_interest_usd: u128;
  short_utilisation_bps: u32;
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
   * Construct and simulate a get_market_utilisation transaction. Returns an `AssembledTransaction` object which will have a `result` field containing the result of the simulation. If this transaction changes contract state, you will need to call `signAndSend()` on the returned object.
   */
  get_market_utilisation: ({data_store, oracle, market}: {data_store: string, oracle: string, market: string}, options?: MethodOptions) => Promise<AssembledTransaction<MarketUtilisation>>

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
      new ContractSpec([ "AAAABAAAAAAAAAAAAAAABUVycm9yAAAAAAAAAQAAATZUaGUgc3VwcGxpZWQgbWFya2V0IGFkZHJlc3MgaXMgbm90IHJlZ2lzdGVyZWQgaW4gZGF0YV9zdG9yZSDigJQKb25lIG9yIG1vcmUgb2YgaXRzIHRva2VuIGFkZHJlc3NlcyAoaW5kZXgvbG9uZy9zaG9ydCkgcmV0dXJuZWQgTm9uZS4KTWlycm9ycyB0aGUgSW52YWxpZE1hcmtldCBwYXR0ZXJuIGluIGRlcG9zaXRfaGFuZGxlci93aXRoZHJhd2FsX2hhbmRsZXIKKGlzc3VlICMzNzEpIHNvIGNhbGxlcnMgY2FuIG1hdGNoIHRoaXMgY29uZGl0aW9uIGFzIGEgdHlwZWQgZXJyb3IKaW5zdGVhZCBvZiBhIGdlbmVyaWMgZXhlY3V0aW9uIGZhaWx1cmUuAAAAAAANSW52YWxpZE1hcmtldAAAAAAAAAE=",
        "AAAAAQAAAAAAAAAAAAAAEU1hcmtldFV0aWxpc2F0aW9uAAAAAAAACAAAAAAAAAAYY29tYmluZWRfdXRpbGlzYXRpb25fYnBzAAAABAAAAAAAAAARaXNfYXRfbG9uZ19vaV9jYXAAAAAAAAABAAAAAAAAABJpc19hdF9zaG9ydF9vaV9jYXAAAAAAAAEAAAAAAAAAFmxvbmdfb3Blbl9pbnRlcmVzdF91c2QAAAAAAAoAAAAAAAAAFGxvbmdfdXRpbGlzYXRpb25fYnBzAAAABAAAAAAAAAAOcG9vbF92YWx1ZV91c2QAAAAAAAoAAAAAAAAAF3Nob3J0X29wZW5faW50ZXJlc3RfdXNkAAAAAAoAAAAAAAAAFXNob3J0X3V0aWxpc2F0aW9uX2JwcwAAAAAAAAQ=",
        "AAAAAAAAAAAAAAAWZ2V0X21hcmtldF91dGlsaXNhdGlvbgAAAAAAAwAAAAAAAAAKZGF0YV9zdG9yZQAAAAAAEwAAAAAAAAAGb3JhY2xlAAAAAAATAAAAAAAAAAZtYXJrZXQAAAAAABMAAAABAAAH0AAAABFNYXJrZXRVdGlsaXNhdGlvbgAAAA==",
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
    get_market_utilisation: this.txFromJSON<MarketUtilisation>
  }
}