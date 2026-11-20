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
  3: {message:"Unauthorized"},
  4: {message:"Underflow"},
  /**
   * set_liquidation_execution_fee called with a fee above MAX_LIQUIDATION_EXECUTION_FEE
   * (issue #633) — an unbounded fee can seize a liquidated position's entire collateral.
   */
  5: {message:"LiquidationExecutionFeeTooHigh"}
}







export interface Client {
  /**
   * Construct and simulate a get_bool transaction. Returns an `AssembledTransaction` object which will have a `result` field containing the result of the simulation. If this transaction changes contract state, you will need to call `signAndSend()` on the returned object.
   */
  get_bool: ({key}: {key: Buffer}, options?: MethodOptions) => Promise<AssembledTransaction<boolean>>

  /**
   * Construct and simulate a get_i128 transaction. Returns an `AssembledTransaction` object which will have a `result` field containing the result of the simulation. If this transaction changes contract state, you will need to call `signAndSend()` on the returned object.
   */
  get_i128: ({key}: {key: Buffer}, options?: MethodOptions) => Promise<AssembledTransaction<i128>>

  /**
   * Construct and simulate a get_u128 transaction. Returns an `AssembledTransaction` object which will have a `result` field containing the result of the simulation. If this transaction changes contract state, you will need to call `signAndSend()` on the returned object.
   */
  get_u128: ({key}: {key: Buffer}, options?: MethodOptions) => Promise<AssembledTransaction<u128>>

  /**
   * Construct and simulate a set_bool transaction. Returns an `AssembledTransaction` object which will have a `result` field containing the result of the simulation. If this transaction changes contract state, you will need to call `signAndSend()` on the returned object.
   */
  set_bool: ({caller, key, value}: {caller: string, key: Buffer, value: boolean}, options?: MethodOptions) => Promise<AssembledTransaction<boolean>>

  /**
   * Construct and simulate a set_i128 transaction. Returns an `AssembledTransaction` object which will have a `result` field containing the result of the simulation. If this transaction changes contract state, you will need to call `signAndSend()` on the returned object.
   */
  set_i128: ({caller, key, value}: {caller: string, key: Buffer, value: i128}, options?: MethodOptions) => Promise<AssembledTransaction<i128>>

  /**
   * Construct and simulate a set_u128 transaction. Returns an `AssembledTransaction` object which will have a `result` field containing the result of the simulation. If this transaction changes contract state, you will need to call `signAndSend()` on the returned object.
   * Write a u128 value to persistent storage.
   * 
   * Issue #357: Requires CONTROLLER role. Any CONTROLLER holder may write
   * to any key — there is no per-namespace ownership enforcement.
   */
  set_u128: ({caller, key, value}: {caller: string, key: Buffer, value: u128}, options?: MethodOptions) => Promise<AssembledTransaction<u128>>

  /**
   * Construct and simulate a get_nonce transaction. Returns an `AssembledTransaction` object which will have a `result` field containing the result of the simulation. If this transaction changes contract state, you will need to call `signAndSend()` on the returned object.
   */
  get_nonce: (options?: MethodOptions) => Promise<AssembledTransaction<u64>>

  /**
   * Construct and simulate a initialize transaction. Returns an `AssembledTransaction` object which will have a `result` field containing the result of the simulation. If this transaction changes contract state, you will need to call `signAndSend()` on the returned object.
   * One-time init: link to role_store for CONTROLLER checks.
   */
  initialize: ({admin, role_store}: {admin: string, role_store: string}, options?: MethodOptions) => Promise<AssembledTransaction<null>>

  /**
   * Construct and simulate a get_address transaction. Returns an `AssembledTransaction` object which will have a `result` field containing the result of the simulation. If this transaction changes contract state, you will need to call `signAndSend()` on the returned object.
   */
  get_address: ({key}: {key: Buffer}, options?: MethodOptions) => Promise<AssembledTransaction<Option<string>>>

  /**
   * Construct and simulate a get_bytes32 transaction. Returns an `AssembledTransaction` object which will have a `result` field containing the result of the simulation. If this transaction changes contract state, you will need to call `signAndSend()` on the returned object.
   */
  get_bytes32: ({key}: {key: Buffer}, options?: MethodOptions) => Promise<AssembledTransaction<Buffer>>

  /**
   * Construct and simulate a remove_bool transaction. Returns an `AssembledTransaction` object which will have a `result` field containing the result of the simulation. If this transaction changes contract state, you will need to call `signAndSend()` on the returned object.
   */
  remove_bool: ({caller, key}: {caller: string, key: Buffer}, options?: MethodOptions) => Promise<AssembledTransaction<null>>

  /**
   * Construct and simulate a remove_i128 transaction. Returns an `AssembledTransaction` object which will have a `result` field containing the result of the simulation. If this transaction changes contract state, you will need to call `signAndSend()` on the returned object.
   */
  remove_i128: ({caller, key}: {caller: string, key: Buffer}, options?: MethodOptions) => Promise<AssembledTransaction<null>>

  /**
   * Construct and simulate a remove_u128 transaction. Returns an `AssembledTransaction` object which will have a `result` field containing the result of the simulation. If this transaction changes contract state, you will need to call `signAndSend()` on the returned object.
   */
  remove_u128: ({caller, key}: {caller: string, key: Buffer}, options?: MethodOptions) => Promise<AssembledTransaction<null>>

  /**
   * Construct and simulate a set_address transaction. Returns an `AssembledTransaction` object which will have a `result` field containing the result of the simulation. If this transaction changes contract state, you will need to call `signAndSend()` on the returned object.
   */
  set_address: ({caller, key, value}: {caller: string, key: Buffer, value: string}, options?: MethodOptions) => Promise<AssembledTransaction<string>>

  /**
   * Construct and simulate a set_bytes32 transaction. Returns an `AssembledTransaction` object which will have a `result` field containing the result of the simulation. If this transaction changes contract state, you will need to call `signAndSend()` on the returned object.
   */
  set_bytes32: ({caller, key, value}: {caller: string, key: Buffer, value: Buffer}, options?: MethodOptions) => Promise<AssembledTransaction<Buffer>>

  /**
   * Construct and simulate a decrement_u128 transaction. Returns an `AssembledTransaction` object which will have a `result` field containing the result of the simulation. If this transaction changes contract state, you will need to call `signAndSend()` on the returned object.
   */
  decrement_u128: ({caller, key, amount}: {caller: string, key: Buffer, amount: u128}, options?: MethodOptions) => Promise<AssembledTransaction<u128>>

  /**
   * Construct and simulate a get_u128_batch transaction. Returns an `AssembledTransaction` object which will have a `result` field containing the result of the simulation. If this transaction changes contract state, you will need to call `signAndSend()` on the returned object.
   * Read multiple u128 values in one call to reduce cross-contract call overhead.
   */
  get_u128_batch: ({keys}: {keys: Array<Buffer>}, options?: MethodOptions) => Promise<AssembledTransaction<Array<u128>>>

  /**
   * Construct and simulate a increment_u128 transaction. Returns an `AssembledTransaction` object which will have a `result` field containing the result of the simulation. If this transaction changes contract state, you will need to call `signAndSend()` on the returned object.
   */
  increment_u128: ({caller, key, amount}: {caller: string, key: Buffer, amount: u128}, options?: MethodOptions) => Promise<AssembledTransaction<u128>>

  /**
   * Construct and simulate a remove_address transaction. Returns an `AssembledTransaction` object which will have a `result` field containing the result of the simulation. If this transaction changes contract state, you will need to call `signAndSend()` on the returned object.
   */
  remove_address: ({caller, key}: {caller: string, key: Buffer}, options?: MethodOptions) => Promise<AssembledTransaction<null>>

  /**
   * Construct and simulate a remove_bytes32 transaction. Returns an `AssembledTransaction` object which will have a `result` field containing the result of the simulation. If this transaction changes contract state, you will need to call `signAndSend()` on the returned object.
   */
  remove_bytes32: ({caller, key}: {caller: string, key: Buffer}, options?: MethodOptions) => Promise<AssembledTransaction<null>>

  /**
   * Construct and simulate a get_u128_cached transaction. Returns an `AssembledTransaction` object which will have a `result` field containing the result of the simulation. If this transaction changes contract state, you will need to call `signAndSend()` on the returned object.
   * Cache-first read for market config u128 values (#299).
   * 
   * Checks the instance cache first.  On a miss, reads from persistent storage
   * and populates the cache so subsequent reads are served without a persistent
   * round-trip.  Use for the same keys managed by `set_u128_config`.
   * 
   * Issue #353: On a cache hit, the value is also re-checked against persistent
   * storage.  This makes the cache self-healing: if any plain mutator
   * (`set_u128`, `apply_delta_to_u128`, `increment_u128`, `decrement_u128`,
   * `remove_u128`) updated the same key without going through
   * `set_u128_config`, the next `get_u128_cached` call will detect the
   * divergence and return the fresh persistent value, updating the cache.
   */
  get_u128_cached: ({key}: {key: Buffer}, options?: MethodOptions) => Promise<AssembledTransaction<u128>>

  /**
   * Construct and simulate a increment_nonce transaction. Returns an `AssembledTransaction` object which will have a `result` field containing the result of the simulation. If this transaction changes contract state, you will need to call `signAndSend()` on the returned object.
   */
  increment_nonce: ({caller}: {caller: string}, options?: MethodOptions) => Promise<AssembledTransaction<u64>>

  /**
   * Construct and simulate a set_u128_config transaction. Returns an `AssembledTransaction` object which will have a `result` field containing the result of the simulation. If this transaction changes contract state, you will need to call `signAndSend()` on the returned object.
   * Write-through cache variant for rarely-changing market config (#299).
   * 
   * Writes to both persistent storage (durable) and the instance-level cache
   * (cheap reads). Use for fee factors, OI caps, leverage limits, and other
   * admin-set parameters that change infrequently but are read on every order
   * execution.  Subsequent `get_u128_cached` calls are served from the
   * cheaper instance entry without a persistent read.
   */
  set_u128_config: ({caller, key, value}: {caller: string, key: Buffer, value: u128}, options?: MethodOptions) => Promise<AssembledTransaction<u128>>

  /**
   * Construct and simulate a contains_address transaction. Returns an `AssembledTransaction` object which will have a `result` field containing the result of the simulation. If this transaction changes contract state, you will need to call `signAndSend()` on the returned object.
   */
  contains_address: ({set_key, value}: {set_key: Buffer, value: string}, options?: MethodOptions) => Promise<AssembledTransaction<boolean>>

  /**
   * Construct and simulate a contains_bytes32 transaction. Returns an `AssembledTransaction` object which will have a `result` field containing the result of the simulation. If this transaction changes contract state, you will need to call `signAndSend()` on the returned object.
   */
  contains_bytes32: ({set_key, value}: {set_key: Buffer, value: Buffer}, options?: MethodOptions) => Promise<AssembledTransaction<boolean>>

  /**
   * Construct and simulate a get_keeper_stats transaction. Returns an `AssembledTransaction` object which will have a `result` field containing the result of the simulation. If this transaction changes contract state, you will need to call `signAndSend()` on the returned object.
   * Retrieve performance metrics for a keeper: (execution_count, total_variance_bps, slash_penalty_amount).
   */
  get_keeper_stats: ({keeper}: {keeper: string}, options?: MethodOptions) => Promise<AssembledTransaction<readonly [u128, u128, u128]>>

  /**
   * Construct and simulate a get_i128_instance transaction. Returns an `AssembledTransaction` object which will have a `result` field containing the result of the simulation. If this transaction changes contract state, you will need to call `signAndSend()` on the returned object.
   */
  get_i128_instance: ({key}: {key: Buffer}, options?: MethodOptions) => Promise<AssembledTransaction<i128>>

  /**
   * Construct and simulate a get_u128_instance transaction. Returns an `AssembledTransaction` object which will have a `result` field containing the result of the simulation. If this transaction changes contract state, you will need to call `signAndSend()` on the returned object.
   */
  get_u128_instance: ({key}: {key: Buffer}, options?: MethodOptions) => Promise<AssembledTransaction<u128>>

  /**
   * Construct and simulate a set_i128_instance transaction. Returns an `AssembledTransaction` object which will have a `result` field containing the result of the simulation. If this transaction changes contract state, you will need to call `signAndSend()` on the returned object.
   */
  set_i128_instance: ({caller, key, value}: {caller: string, key: Buffer, value: i128}, options?: MethodOptions) => Promise<AssembledTransaction<i128>>

  /**
   * Construct and simulate a set_u128_instance transaction. Returns an `AssembledTransaction` object which will have a `result` field containing the result of the simulation. If this transaction changes contract state, you will need to call `signAndSend()` on the returned object.
   */
  set_u128_instance: ({caller, key, value}: {caller: string, key: Buffer, value: u128}, options?: MethodOptions) => Promise<AssembledTransaction<u128>>

  /**
   * Construct and simulate a add_address_to_set transaction. Returns an `AssembledTransaction` object which will have a `result` field containing the result of the simulation. If this transaction changes contract state, you will need to call `signAndSend()` on the returned object.
   */
  add_address_to_set: ({caller, set_key, value}: {caller: string, set_key: Buffer, value: string}, options?: MethodOptions) => Promise<AssembledTransaction<null>>

  /**
   * Construct and simulate a add_bytes32_to_set transaction. Returns an `AssembledTransaction` object which will have a `result` field containing the result of the simulation. If this transaction changes contract state, you will need to call `signAndSend()` on the returned object.
   */
  add_bytes32_to_set: ({caller, set_key, value}: {caller: string, set_key: Buffer, value: Buffer}, options?: MethodOptions) => Promise<AssembledTransaction<null>>

  /**
   * Construct and simulate a get_address_set_at transaction. Returns an `AssembledTransaction` object which will have a `result` field containing the result of the simulation. If this transaction changes contract state, you will need to call `signAndSend()` on the returned object.
   */
  get_address_set_at: ({set_key, start, end}: {set_key: Buffer, start: u32, end: u32}, options?: MethodOptions) => Promise<AssembledTransaction<Array<string>>>

  /**
   * Construct and simulate a get_bytes32_set_at transaction. Returns an `AssembledTransaction` object which will have a `result` field containing the result of the simulation. If this transaction changes contract state, you will need to call `signAndSend()` on the returned object.
   */
  get_bytes32_set_at: ({set_key, start, end}: {set_key: Buffer, start: u32, end: u32}, options?: MethodOptions) => Promise<AssembledTransaction<Array<Buffer>>>

  /**
   * Construct and simulate a apply_delta_to_i128 transaction. Returns an `AssembledTransaction` object which will have a `result` field containing the result of the simulation. If this transaction changes contract state, you will need to call `signAndSend()` on the returned object.
   */
  apply_delta_to_i128: ({caller, key, delta}: {caller: string, key: Buffer, delta: i128}, options?: MethodOptions) => Promise<AssembledTransaction<i128>>

  /**
   * Construct and simulate a apply_delta_to_u128 transaction. Returns an `AssembledTransaction` object which will have a `result` field containing the result of the simulation. If this transaction changes contract state, you will need to call `signAndSend()` on the returned object.
   * Add `delta` (signed) to existing u128 value. Panics on underflow.
   * 
   * # Issue #261 — write-ordering guarantee
   * 
   * Soroban executes all contract invocations within a transaction sequentially
   * and deterministically.  Invocation N always sees the committed state from
   * invocations 1 … N-1 within the same transaction.  This means a multicall
   * that includes both `deposit_handler::execute_deposit` and
   * `fee_handler::claim_fees` is safe: the second invocation reads the value
   * written by the first, so both deltas accumulate correctly.
   * 
   * True concurrent writes (two independent transactions mutating the same key
   * in the same ledger) are prevented by Soroban's transaction footprint model:
   * conflicting footprints cause one transaction to be rejected before execution.
   * 
   * Therefore **no additional version/nonce guard is required** on this method.
   * Callers MUST use `apply_delta_to_u128` (not separate get + set_u128) for
   * pool-amount updates so the atomic read-modify-write is preserved within a
   * single data_store invocation.
   */
  apply_delta_to_u128: ({caller, key, delta}: {caller: string, key: Buffer, delta: i128}, options?: MethodOptions) => Promise<AssembledTransaction<u128>>

  /**
   * Construct and simulate a is_keeper_suspended transaction. Returns an `AssembledTransaction` object which will have a `result` field containing the result of the simulation. If this transaction changes contract state, you will need to call `signAndSend()` on the returned object.
   * True when `keeper`'s accumulated slash penalty has reached the global
   * `keeper_slash_threshold_key` (issue #587). Handlers call this before
   * letting a keeper execute, which is what gives slashing a real
   * consequence. A threshold of 0 (the default) disables suspension.
   */
  is_keeper_suspended: ({keeper}: {keeper: string}, options?: MethodOptions) => Promise<AssembledTransaction<boolean>>

  /**
   * Construct and simulate a get_position_manager transaction. Returns an `AssembledTransaction` object which will have a `result` field containing the result of the simulation. If this transaction changes contract state, you will need to call `signAndSend()` on the returned object.
   * Get the authorized position manager for a given owner and market.
   * Returns None if no manager is set or if it has been revoked via
   * remove_position_manager.
   */
  get_position_manager: ({owner, market}: {owner: string, market: string}, options?: MethodOptions) => Promise<AssembledTransaction<Option<string>>>

  /**
   * Construct and simulate a set_position_manager transaction. Returns an `AssembledTransaction` object which will have a `result` field containing the result of the simulation. If this transaction changes contract state, you will need to call `signAndSend()` on the returned object.
   * Set a position manager for a given owner and market.
   * Only the owner can call this. Returns the manager that was set.
   * Revoke with remove_position_manager.
   */
  set_position_manager: ({owner, market, manager}: {owner: string, market: string, manager: string}, options?: MethodOptions) => Promise<AssembledTransaction<string>>

  /**
   * Construct and simulate a get_address_set_count transaction. Returns an `AssembledTransaction` object which will have a `result` field containing the result of the simulation. If this transaction changes contract state, you will need to call `signAndSend()` on the returned object.
   */
  get_address_set_count: ({set_key}: {set_key: Buffer}, options?: MethodOptions) => Promise<AssembledTransaction<u32>>

  /**
   * Construct and simulate a get_bytes32_set_count transaction. Returns an `AssembledTransaction` object which will have a `result` field containing the result of the simulation. If this transaction changes contract state, you will need to call `signAndSend()` on the returned object.
   */
  get_bytes32_set_count: ({set_key}: {set_key: Buffer}, options?: MethodOptions) => Promise<AssembledTransaction<u32>>

  /**
   * Construct and simulate a get_min_execution_fee transaction. Returns an `AssembledTransaction` object which will have a `result` field containing the result of the simulation. If this transaction changes contract state, you will need to call `signAndSend()` on the returned object.
   * Get the global minimum execution fee required for all order types.
   * Returns 0 if not configured (no minimum enforced).
   */
  get_min_execution_fee: (options?: MethodOptions) => Promise<AssembledTransaction<u128>>

  /**
   * Construct and simulate a set_min_execution_fee transaction. Returns an `AssembledTransaction` object which will have a `result` field containing the result of the simulation. If this transaction changes contract state, you will need to call `signAndSend()` on the returned object.
   * Set the global minimum execution fee (controller-only).
   */
  set_min_execution_fee: ({caller, fee}: {caller: string, fee: u128}, options?: MethodOptions) => Promise<AssembledTransaction<u128>>

  /**
   * Construct and simulate a record_keeper_execution transaction. Returns an `AssembledTransaction` object which will have a `result` field containing the result of the simulation. If this transaction changes contract state, you will need to call `signAndSend()` on the returned object.
   * Record execution quality metrics for a keeper and apply slashing penalty if variance exceeds limit (>5%).
   */
  record_keeper_execution: ({caller, keeper, executed_price, expected_price}: {caller: string, keeper: string, executed_price: u128, expected_price: u128}, options?: MethodOptions) => Promise<AssembledTransaction<null>>

  /**
   * Construct and simulate a remove_address_from_set transaction. Returns an `AssembledTransaction` object which will have a `result` field containing the result of the simulation. If this transaction changes contract state, you will need to call `signAndSend()` on the returned object.
   */
  remove_address_from_set: ({caller, set_key, value}: {caller: string, set_key: Buffer, value: string}, options?: MethodOptions) => Promise<AssembledTransaction<null>>

  /**
   * Construct and simulate a remove_bytes32_from_set transaction. Returns an `AssembledTransaction` object which will have a `result` field containing the result of the simulation. If this transaction changes contract state, you will need to call `signAndSend()` on the returned object.
   */
  remove_bytes32_from_set: ({caller, set_key, value}: {caller: string, set_key: Buffer, value: Buffer}, options?: MethodOptions) => Promise<AssembledTransaction<null>>

  /**
   * Construct and simulate a remove_position_manager transaction. Returns an `AssembledTransaction` object which will have a `result` field containing the result of the simulation. If this transaction changes contract state, you will need to call `signAndSend()` on the returned object.
   * Revoke the position manager for a given owner and market.
   * Only the owner can call this. Returns true if a manager was previously
   * set (and is now removed), false if none was set.
   */
  remove_position_manager: ({owner, market}: {owner: string, market: string}, options?: MethodOptions) => Promise<AssembledTransaction<boolean>>

  /**
   * Construct and simulate a get_liquidation_execution_fee transaction. Returns an `AssembledTransaction` object which will have a `result` field containing the result of the simulation. If this transaction changes contract state, you will need to call `signAndSend()` on the returned object.
   * Get the liquidation execution fee for a given market.
   * This fee is paid to the keeper from position collateral on successful liquidation.
   */
  get_liquidation_execution_fee: ({market}: {market: string}, options?: MethodOptions) => Promise<AssembledTransaction<u128>>

  /**
   * Construct and simulate a set_liquidation_execution_fee transaction. Returns an `AssembledTransaction` object which will have a `result` field containing the result of the simulation. If this transaction changes contract state, you will need to call `signAndSend()` on the returned object.
   * Set the liquidation execution fee for a given market (admin-only). Capped at
   * MAX_LIQUIDATION_EXECUTION_FEE so a fat-fingered/misused value can't seize a
   * liquidated position's entire collateral as keeper fee (issue #633).
   */
  set_liquidation_execution_fee: ({caller, market, fee}: {caller: string, market: string, fee: u128}, options?: MethodOptions) => Promise<AssembledTransaction<u128>>

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
      new ContractSpec([ "AAAABAAAAAAAAAAAAAAABUVycm9yAAAAAAAABQAAAAAAAAAOTm90SW5pdGlhbGl6ZWQAAAAAAAEAAAAAAAAAEkFscmVhZHlJbml0aWFsaXplZAAAAAAAAgAAAAAAAAAMVW5hdXRob3JpemVkAAAAAwAAAAAAAAAJVW5kZXJmbG93AAAAAAAABAAAAKpzZXRfbGlxdWlkYXRpb25fZXhlY3V0aW9uX2ZlZSBjYWxsZWQgd2l0aCBhIGZlZSBhYm92ZSBNQVhfTElRVUlEQVRJT05fRVhFQ1VUSU9OX0ZFRQooaXNzdWUgIzYzMykg4oCUIGFuIHVuYm91bmRlZCBmZWUgY2FuIHNlaXplIGEgbGlxdWlkYXRlZCBwb3NpdGlvbidzIGVudGlyZSBjb2xsYXRlcmFsLgAAAAAAHkxpcXVpZGF0aW9uRXhlY3V0aW9uRmVlVG9vSGlnaAAAAAAABQ==",
        "AAAABQAAAAAAAAAAAAAADUtlZXBlclNsYXNoZWQAAAAAAAABAAAACWtwcl9zbGFzaAAAAAAAAAUAAAAAAAAABmtlZXBlcgAAAAAAEwAAAAAAAAAAAAAADmV4ZWN1dGVkX3ByaWNlAAAAAAAKAAAAAAAAAAAAAAAOZXhwZWN0ZWRfcHJpY2UAAAAAAAoAAAAAAAAAAAAAAAx2YXJpYW5jZV9icHMAAAAKAAAAAAAAAAAAAAAOcGVuYWx0eV9hbW91bnQAAAAAAAoAAAAAAAAAAg==",
        "AAAABQAAAAAAAAAAAAAAEk1pbkV4ZWN1dGlvbkZlZVNldAAAAAAAAQAAAAttaW5fZmVlX3NldAAAAAABAAAAAAAAAANmZWUAAAAACgAAAAAAAAAC",
        "AAAABQAAARtJc3N1ZSAjNjA3OiBzZXRfcG9zaXRpb25fbWFuYWdlciwgc2V0X2xpcXVpZGF0aW9uX2V4ZWN1dGlvbl9mZWUsIGFuZApzZXRfbWluX2V4ZWN1dGlvbl9mZWUgcHJldmlvdXNseSBoYWQgbm8gZXZlbnQgYW55d2hlcmUgaW4gdGhlIGNhbGwgY2hhaW4KKGluY2x1ZGluZyBleGNoYW5nZV9yb3V0ZXI6OnNldF9wb3NpdGlvbl9tYW5hZ2VyJ3MgcGFzcy10aHJvdWdoLCB3aGljaApjYWxscyB0aGlzIHNhbWUgZnVuY3Rpb24g4oCUIG9uZSBldmVudCBoZXJlIGNvdmVycyBib3RoIGVudHJ5IHBvaW50cykuAAAAAAAAAAASUG9zaXRpb25NYW5hZ2VyU2V0AAAAAAABAAAAC3Bvc19tZ3Jfc2V0AAAAAAMAAAAAAAAABW93bmVyAAAAAAAAEwAAAAAAAAAAAAAABm1hcmtldAAAAAAAEwAAAAAAAAAAAAAAB21hbmFnZXIAAAAAEwAAAAAAAAAC",
        "AAAABQAAAAAAAAAAAAAAFERhdGFTdG9yZUluaXRpYWxpemVkAAAAAQAAAARpbml0AAAAAQAAAAAAAAAKcm9sZV9zdG9yZQAAAAAAEwAAAAAAAAAC",
        "AAAABQAAAAAAAAAAAAAAFlBvc2l0aW9uTWFuYWdlclJlbW92ZWQAAAAAAAEAAAAKcG9zX21ncl9ybQAAAAAAAgAAAAAAAAAFb3duZXIAAAAAAAATAAAAAAAAAAAAAAAGbWFya2V0AAAAAAATAAAAAAAAAAI=",
        "AAAABQAAAAAAAAAAAAAAGkxpcXVpZGF0aW9uRXhlY3V0aW9uRmVlU2V0AAAAAAABAAAAC2xpcV9mZWVfc2V0AAAAAAIAAAAAAAAABm1hcmtldAAAAAAAEwAAAAAAAAAAAAAAA2ZlZQAAAAAKAAAAAAAAAAI=",
        "AAAAAAAAAAAAAAAIZ2V0X2Jvb2wAAAABAAAAAAAAAANrZXkAAAAD7gAAACAAAAABAAAAAQ==",
        "AAAAAAAAAAAAAAAIZ2V0X2kxMjgAAAABAAAAAAAAAANrZXkAAAAD7gAAACAAAAABAAAACw==",
        "AAAAAAAAAAAAAAAIZ2V0X3UxMjgAAAABAAAAAAAAAANrZXkAAAAD7gAAACAAAAABAAAACg==",
        "AAAAAAAAAAAAAAAIc2V0X2Jvb2wAAAADAAAAAAAAAAZjYWxsZXIAAAAAABMAAAAAAAAAA2tleQAAAAPuAAAAIAAAAAAAAAAFdmFsdWUAAAAAAAABAAAAAQAAAAE=",
        "AAAAAAAAAAAAAAAIc2V0X2kxMjgAAAADAAAAAAAAAAZjYWxsZXIAAAAAABMAAAAAAAAAA2tleQAAAAPuAAAAIAAAAAAAAAAFdmFsdWUAAAAAAAALAAAAAQAAAAs=",
        "AAAAAAAAALBXcml0ZSBhIHUxMjggdmFsdWUgdG8gcGVyc2lzdGVudCBzdG9yYWdlLgoKSXNzdWUgIzM1NzogUmVxdWlyZXMgQ09OVFJPTExFUiByb2xlLiBBbnkgQ09OVFJPTExFUiBob2xkZXIgbWF5IHdyaXRlCnRvIGFueSBrZXkg4oCUIHRoZXJlIGlzIG5vIHBlci1uYW1lc3BhY2Ugb3duZXJzaGlwIGVuZm9yY2VtZW50LgAAAAhzZXRfdTEyOAAAAAMAAAAAAAAABmNhbGxlcgAAAAAAEwAAAAAAAAADa2V5AAAAA+4AAAAgAAAAAAAAAAV2YWx1ZQAAAAAAAAoAAAABAAAACg==",
        "AAAAAAAAAAAAAAAJZ2V0X25vbmNlAAAAAAAAAAAAAAEAAAAG",
        "AAAAAAAAADhPbmUtdGltZSBpbml0OiBsaW5rIHRvIHJvbGVfc3RvcmUgZm9yIENPTlRST0xMRVIgY2hlY2tzLgAAAAppbml0aWFsaXplAAAAAAACAAAAAAAAAAVhZG1pbgAAAAAAABMAAAAAAAAACnJvbGVfc3RvcmUAAAAAABMAAAAA",
        "AAAAAAAAAAAAAAALZ2V0X2FkZHJlc3MAAAAAAQAAAAAAAAADa2V5AAAAA+4AAAAgAAAAAQAAA+gAAAAT",
        "AAAAAAAAAAAAAAALZ2V0X2J5dGVzMzIAAAAAAQAAAAAAAAADa2V5AAAAA+4AAAAgAAAAAQAAA+4AAAAg",
        "AAAAAAAAAAAAAAALcmVtb3ZlX2Jvb2wAAAAAAgAAAAAAAAAGY2FsbGVyAAAAAAATAAAAAAAAAANrZXkAAAAD7gAAACAAAAAA",
        "AAAAAAAAAAAAAAALcmVtb3ZlX2kxMjgAAAAAAgAAAAAAAAAGY2FsbGVyAAAAAAATAAAAAAAAAANrZXkAAAAD7gAAACAAAAAA",
        "AAAAAAAAAAAAAAALcmVtb3ZlX3UxMjgAAAAAAgAAAAAAAAAGY2FsbGVyAAAAAAATAAAAAAAAAANrZXkAAAAD7gAAACAAAAAA",
        "AAAAAAAAAAAAAAALc2V0X2FkZHJlc3MAAAAAAwAAAAAAAAAGY2FsbGVyAAAAAAATAAAAAAAAAANrZXkAAAAD7gAAACAAAAAAAAAABXZhbHVlAAAAAAAAEwAAAAEAAAAT",
        "AAAAAAAAAAAAAAALc2V0X2J5dGVzMzIAAAAAAwAAAAAAAAAGY2FsbGVyAAAAAAATAAAAAAAAAANrZXkAAAAD7gAAACAAAAAAAAAABXZhbHVlAAAAAAAD7gAAACAAAAABAAAD7gAAACA=",
        "AAAAAAAAAAAAAAAOZGVjcmVtZW50X3UxMjgAAAAAAAMAAAAAAAAABmNhbGxlcgAAAAAAEwAAAAAAAAADa2V5AAAAA+4AAAAgAAAAAAAAAAZhbW91bnQAAAAAAAoAAAABAAAACg==",
        "AAAAAAAAAE1SZWFkIG11bHRpcGxlIHUxMjggdmFsdWVzIGluIG9uZSBjYWxsIHRvIHJlZHVjZSBjcm9zcy1jb250cmFjdCBjYWxsIG92ZXJoZWFkLgAAAAAAAA5nZXRfdTEyOF9iYXRjaAAAAAAAAQAAAAAAAAAEa2V5cwAAA+oAAAPuAAAAIAAAAAEAAAPqAAAACg==",
        "AAAAAAAAAAAAAAAOaW5jcmVtZW50X3UxMjgAAAAAAAMAAAAAAAAABmNhbGxlcgAAAAAAEwAAAAAAAAADa2V5AAAAA+4AAAAgAAAAAAAAAAZhbW91bnQAAAAAAAoAAAABAAAACg==",
        "AAAAAAAAAAAAAAAOcmVtb3ZlX2FkZHJlc3MAAAAAAAIAAAAAAAAABmNhbGxlcgAAAAAAEwAAAAAAAAADa2V5AAAAA+4AAAAgAAAAAA==",
        "AAAAAAAAAAAAAAAOcmVtb3ZlX2J5dGVzMzIAAAAAAAIAAAAAAAAABmNhbGxlcgAAAAAAEwAAAAAAAAADa2V5AAAAA+4AAAAgAAAAAA==",
        "AAAAAAAAAqlDYWNoZS1maXJzdCByZWFkIGZvciBtYXJrZXQgY29uZmlnIHUxMjggdmFsdWVzICgjMjk5KS4KCkNoZWNrcyB0aGUgaW5zdGFuY2UgY2FjaGUgZmlyc3QuICBPbiBhIG1pc3MsIHJlYWRzIGZyb20gcGVyc2lzdGVudCBzdG9yYWdlCmFuZCBwb3B1bGF0ZXMgdGhlIGNhY2hlIHNvIHN1YnNlcXVlbnQgcmVhZHMgYXJlIHNlcnZlZCB3aXRob3V0IGEgcGVyc2lzdGVudApyb3VuZC10cmlwLiAgVXNlIGZvciB0aGUgc2FtZSBrZXlzIG1hbmFnZWQgYnkgYHNldF91MTI4X2NvbmZpZ2AuCgpJc3N1ZSAjMzUzOiBPbiBhIGNhY2hlIGhpdCwgdGhlIHZhbHVlIGlzIGFsc28gcmUtY2hlY2tlZCBhZ2FpbnN0IHBlcnNpc3RlbnQKc3RvcmFnZS4gIFRoaXMgbWFrZXMgdGhlIGNhY2hlIHNlbGYtaGVhbGluZzogaWYgYW55IHBsYWluIG11dGF0b3IKKGBzZXRfdTEyOGAsIGBhcHBseV9kZWx0YV90b191MTI4YCwgYGluY3JlbWVudF91MTI4YCwgYGRlY3JlbWVudF91MTI4YCwKYHJlbW92ZV91MTI4YCkgdXBkYXRlZCB0aGUgc2FtZSBrZXkgd2l0aG91dCBnb2luZyB0aHJvdWdoCmBzZXRfdTEyOF9jb25maWdgLCB0aGUgbmV4dCBgZ2V0X3UxMjhfY2FjaGVkYCBjYWxsIHdpbGwgZGV0ZWN0IHRoZQpkaXZlcmdlbmNlIGFuZCByZXR1cm4gdGhlIGZyZXNoIHBlcnNpc3RlbnQgdmFsdWUsIHVwZGF0aW5nIHRoZSBjYWNoZS4AAAAAAAAPZ2V0X3UxMjhfY2FjaGVkAAAAAAEAAAAAAAAAA2tleQAAAAPuAAAAIAAAAAEAAAAK",
        "AAAAAAAAAAAAAAAPaW5jcmVtZW50X25vbmNlAAAAAAEAAAAAAAAABmNhbGxlcgAAAAAAEwAAAAEAAAAG",
        "AAAAAAAAAZZXcml0ZS10aHJvdWdoIGNhY2hlIHZhcmlhbnQgZm9yIHJhcmVseS1jaGFuZ2luZyBtYXJrZXQgY29uZmlnICgjMjk5KS4KCldyaXRlcyB0byBib3RoIHBlcnNpc3RlbnQgc3RvcmFnZSAoZHVyYWJsZSkgYW5kIHRoZSBpbnN0YW5jZS1sZXZlbCBjYWNoZQooY2hlYXAgcmVhZHMpLiBVc2UgZm9yIGZlZSBmYWN0b3JzLCBPSSBjYXBzLCBsZXZlcmFnZSBsaW1pdHMsIGFuZCBvdGhlcgphZG1pbi1zZXQgcGFyYW1ldGVycyB0aGF0IGNoYW5nZSBpbmZyZXF1ZW50bHkgYnV0IGFyZSByZWFkIG9uIGV2ZXJ5IG9yZGVyCmV4ZWN1dGlvbi4gIFN1YnNlcXVlbnQgYGdldF91MTI4X2NhY2hlZGAgY2FsbHMgYXJlIHNlcnZlZCBmcm9tIHRoZQpjaGVhcGVyIGluc3RhbmNlIGVudHJ5IHdpdGhvdXQgYSBwZXJzaXN0ZW50IHJlYWQuAAAAAAAPc2V0X3UxMjhfY29uZmlnAAAAAAMAAAAAAAAABmNhbGxlcgAAAAAAEwAAAAAAAAADa2V5AAAAA+4AAAAgAAAAAAAAAAV2YWx1ZQAAAAAAAAoAAAABAAAACg==",
        "AAAAAAAAAAAAAAAQY29udGFpbnNfYWRkcmVzcwAAAAIAAAAAAAAAB3NldF9rZXkAAAAD7gAAACAAAAAAAAAABXZhbHVlAAAAAAAAEwAAAAEAAAAB",
        "AAAAAAAAAAAAAAAQY29udGFpbnNfYnl0ZXMzMgAAAAIAAAAAAAAAB3NldF9rZXkAAAAD7gAAACAAAAAAAAAABXZhbHVlAAAAAAAD7gAAACAAAAABAAAAAQ==",
        "AAAAAAAAAGdSZXRyaWV2ZSBwZXJmb3JtYW5jZSBtZXRyaWNzIGZvciBhIGtlZXBlcjogKGV4ZWN1dGlvbl9jb3VudCwgdG90YWxfdmFyaWFuY2VfYnBzLCBzbGFzaF9wZW5hbHR5X2Ftb3VudCkuAAAAABBnZXRfa2VlcGVyX3N0YXRzAAAAAQAAAAAAAAAGa2VlcGVyAAAAAAATAAAAAQAAA+0AAAADAAAACgAAAAoAAAAK",
        "AAAAAAAAAAAAAAARZ2V0X2kxMjhfaW5zdGFuY2UAAAAAAAABAAAAAAAAAANrZXkAAAAD7gAAACAAAAABAAAACw==",
        "AAAAAAAAAAAAAAARZ2V0X3UxMjhfaW5zdGFuY2UAAAAAAAABAAAAAAAAAANrZXkAAAAD7gAAACAAAAABAAAACg==",
        "AAAAAAAAAAAAAAARc2V0X2kxMjhfaW5zdGFuY2UAAAAAAAADAAAAAAAAAAZjYWxsZXIAAAAAABMAAAAAAAAAA2tleQAAAAPuAAAAIAAAAAAAAAAFdmFsdWUAAAAAAAALAAAAAQAAAAs=",
        "AAAAAAAAAAAAAAARc2V0X3UxMjhfaW5zdGFuY2UAAAAAAAADAAAAAAAAAAZjYWxsZXIAAAAAABMAAAAAAAAAA2tleQAAAAPuAAAAIAAAAAAAAAAFdmFsdWUAAAAAAAAKAAAAAQAAAAo=",
        "AAAAAAAAAAAAAAASYWRkX2FkZHJlc3NfdG9fc2V0AAAAAAADAAAAAAAAAAZjYWxsZXIAAAAAABMAAAAAAAAAB3NldF9rZXkAAAAD7gAAACAAAAAAAAAABXZhbHVlAAAAAAAAEwAAAAA=",
        "AAAAAAAAAAAAAAASYWRkX2J5dGVzMzJfdG9fc2V0AAAAAAADAAAAAAAAAAZjYWxsZXIAAAAAABMAAAAAAAAAB3NldF9rZXkAAAAD7gAAACAAAAAAAAAABXZhbHVlAAAAAAAD7gAAACAAAAAA",
        "AAAAAAAAAAAAAAASZ2V0X2FkZHJlc3Nfc2V0X2F0AAAAAAADAAAAAAAAAAdzZXRfa2V5AAAAA+4AAAAgAAAAAAAAAAVzdGFydAAAAAAAAAQAAAAAAAAAA2VuZAAAAAAEAAAAAQAAA+oAAAAT",
        "AAAAAAAAAAAAAAASZ2V0X2J5dGVzMzJfc2V0X2F0AAAAAAADAAAAAAAAAAdzZXRfa2V5AAAAA+4AAAAgAAAAAAAAAAVzdGFydAAAAAAAAAQAAAAAAAAAA2VuZAAAAAAEAAAAAQAAA+oAAAPuAAAAIA==",
        "AAAAAAAAAAAAAAATYXBwbHlfZGVsdGFfdG9faTEyOAAAAAADAAAAAAAAAAZjYWxsZXIAAAAAABMAAAAAAAAAA2tleQAAAAPuAAAAIAAAAAAAAAAFZGVsdGEAAAAAAAALAAAAAQAAAAs=",
        "AAAAAAAAA/BBZGQgYGRlbHRhYCAoc2lnbmVkKSB0byBleGlzdGluZyB1MTI4IHZhbHVlLiBQYW5pY3Mgb24gdW5kZXJmbG93LgoKIyBJc3N1ZSAjMjYxIOKAlCB3cml0ZS1vcmRlcmluZyBndWFyYW50ZWUKClNvcm9iYW4gZXhlY3V0ZXMgYWxsIGNvbnRyYWN0IGludm9jYXRpb25zIHdpdGhpbiBhIHRyYW5zYWN0aW9uIHNlcXVlbnRpYWxseQphbmQgZGV0ZXJtaW5pc3RpY2FsbHkuICBJbnZvY2F0aW9uIE4gYWx3YXlzIHNlZXMgdGhlIGNvbW1pdHRlZCBzdGF0ZSBmcm9tCmludm9jYXRpb25zIDEg4oCmIE4tMSB3aXRoaW4gdGhlIHNhbWUgdHJhbnNhY3Rpb24uICBUaGlzIG1lYW5zIGEgbXVsdGljYWxsCnRoYXQgaW5jbHVkZXMgYm90aCBgZGVwb3NpdF9oYW5kbGVyOjpleGVjdXRlX2RlcG9zaXRgIGFuZApgZmVlX2hhbmRsZXI6OmNsYWltX2ZlZXNgIGlzIHNhZmU6IHRoZSBzZWNvbmQgaW52b2NhdGlvbiByZWFkcyB0aGUgdmFsdWUKd3JpdHRlbiBieSB0aGUgZmlyc3QsIHNvIGJvdGggZGVsdGFzIGFjY3VtdWxhdGUgY29ycmVjdGx5LgoKVHJ1ZSBjb25jdXJyZW50IHdyaXRlcyAodHdvIGluZGVwZW5kZW50IHRyYW5zYWN0aW9ucyBtdXRhdGluZyB0aGUgc2FtZSBrZXkKaW4gdGhlIHNhbWUgbGVkZ2VyKSBhcmUgcHJldmVudGVkIGJ5IFNvcm9iYW4ncyB0cmFuc2FjdGlvbiBmb290cHJpbnQgbW9kZWw6CmNvbmZsaWN0aW5nIGZvb3RwcmludHMgY2F1c2Ugb25lIHRyYW5zYWN0aW9uIHRvIGJlIHJlamVjdGVkIGJlZm9yZSBleGVjdXRpb24uCgpUaGVyZWZvcmUgKipubyBhZGRpdGlvbmFsIHZlcnNpb24vbm9uY2UgZ3VhcmQgaXMgcmVxdWlyZWQqKiBvbiB0aGlzIG1ldGhvZC4KQ2FsbGVycyBNVVNUIHVzZSBgYXBwbHlfZGVsdGFfdG9fdTEyOGAgKG5vdCBzZXBhcmF0ZSBnZXQgKyBzZXRfdTEyOCkgZm9yCnBvb2wtYW1vdW50IHVwZGF0ZXMgc28gdGhlIGF0b21pYyByZWFkLW1vZGlmeS13cml0ZSBpcyBwcmVzZXJ2ZWQgd2l0aGluIGEKc2luZ2xlIGRhdGFfc3RvcmUgaW52b2NhdGlvbi4AAAATYXBwbHlfZGVsdGFfdG9fdTEyOAAAAAADAAAAAAAAAAZjYWxsZXIAAAAAABMAAAAAAAAAA2tleQAAAAPuAAAAIAAAAAAAAAAFZGVsdGEAAAAAAAALAAAAAQAAAAo=",
        "AAAAAAAAAQlUcnVlIHdoZW4gYGtlZXBlcmAncyBhY2N1bXVsYXRlZCBzbGFzaCBwZW5hbHR5IGhhcyByZWFjaGVkIHRoZSBnbG9iYWwKYGtlZXBlcl9zbGFzaF90aHJlc2hvbGRfa2V5YCAoaXNzdWUgIzU4NykuIEhhbmRsZXJzIGNhbGwgdGhpcyBiZWZvcmUKbGV0dGluZyBhIGtlZXBlciBleGVjdXRlLCB3aGljaCBpcyB3aGF0IGdpdmVzIHNsYXNoaW5nIGEgcmVhbApjb25zZXF1ZW5jZS4gQSB0aHJlc2hvbGQgb2YgMCAodGhlIGRlZmF1bHQpIGRpc2FibGVzIHN1c3BlbnNpb24uAAAAAAAAE2lzX2tlZXBlcl9zdXNwZW5kZWQAAAAAAQAAAAAAAAAGa2VlcGVyAAAAAAATAAAAAQAAAAE=",
        "AAAAAAAAAJpHZXQgdGhlIGF1dGhvcml6ZWQgcG9zaXRpb24gbWFuYWdlciBmb3IgYSBnaXZlbiBvd25lciBhbmQgbWFya2V0LgpSZXR1cm5zIE5vbmUgaWYgbm8gbWFuYWdlciBpcyBzZXQgb3IgaWYgaXQgaGFzIGJlZW4gcmV2b2tlZCB2aWEKcmVtb3ZlX3Bvc2l0aW9uX21hbmFnZXIuAAAAAAAUZ2V0X3Bvc2l0aW9uX21hbmFnZXIAAAACAAAAAAAAAAVvd25lcgAAAAAAABMAAAAAAAAABm1hcmtldAAAAAAAEwAAAAEAAAPoAAAAEw==",
        "AAAAAAAAAJlTZXQgYSBwb3NpdGlvbiBtYW5hZ2VyIGZvciBhIGdpdmVuIG93bmVyIGFuZCBtYXJrZXQuCk9ubHkgdGhlIG93bmVyIGNhbiBjYWxsIHRoaXMuIFJldHVybnMgdGhlIG1hbmFnZXIgdGhhdCB3YXMgc2V0LgpSZXZva2Ugd2l0aCByZW1vdmVfcG9zaXRpb25fbWFuYWdlci4AAAAAAAAUc2V0X3Bvc2l0aW9uX21hbmFnZXIAAAADAAAAAAAAAAVvd25lcgAAAAAAABMAAAAAAAAABm1hcmtldAAAAAAAEwAAAAAAAAAHbWFuYWdlcgAAAAATAAAAAQAAABM=",
        "AAAAAAAAAAAAAAAVZ2V0X2FkZHJlc3Nfc2V0X2NvdW50AAAAAAAAAQAAAAAAAAAHc2V0X2tleQAAAAPuAAAAIAAAAAEAAAAE",
        "AAAAAAAAAAAAAAAVZ2V0X2J5dGVzMzJfc2V0X2NvdW50AAAAAAAAAQAAAAAAAAAHc2V0X2tleQAAAAPuAAAAIAAAAAEAAAAE",
        "AAAAAAAAAHVHZXQgdGhlIGdsb2JhbCBtaW5pbXVtIGV4ZWN1dGlvbiBmZWUgcmVxdWlyZWQgZm9yIGFsbCBvcmRlciB0eXBlcy4KUmV0dXJucyAwIGlmIG5vdCBjb25maWd1cmVkIChubyBtaW5pbXVtIGVuZm9yY2VkKS4AAAAAAAAVZ2V0X21pbl9leGVjdXRpb25fZmVlAAAAAAAAAAAAAAEAAAAK",
        "AAAAAAAAADdTZXQgdGhlIGdsb2JhbCBtaW5pbXVtIGV4ZWN1dGlvbiBmZWUgKGNvbnRyb2xsZXItb25seSkuAAAAABVzZXRfbWluX2V4ZWN1dGlvbl9mZWUAAAAAAAACAAAAAAAAAAZjYWxsZXIAAAAAABMAAAAAAAAAA2ZlZQAAAAAKAAAAAQAAAAo=",
        "AAAAAAAAAGlSZWNvcmQgZXhlY3V0aW9uIHF1YWxpdHkgbWV0cmljcyBmb3IgYSBrZWVwZXIgYW5kIGFwcGx5IHNsYXNoaW5nIHBlbmFsdHkgaWYgdmFyaWFuY2UgZXhjZWVkcyBsaW1pdCAoPjUlKS4AAAAAAAAXcmVjb3JkX2tlZXBlcl9leGVjdXRpb24AAAAABAAAAAAAAAAGY2FsbGVyAAAAAAATAAAAAAAAAAZrZWVwZXIAAAAAABMAAAAAAAAADmV4ZWN1dGVkX3ByaWNlAAAAAAAKAAAAAAAAAA5leHBlY3RlZF9wcmljZQAAAAAACgAAAAA=",
        "AAAAAAAAAAAAAAAXcmVtb3ZlX2FkZHJlc3NfZnJvbV9zZXQAAAAAAwAAAAAAAAAGY2FsbGVyAAAAAAATAAAAAAAAAAdzZXRfa2V5AAAAA+4AAAAgAAAAAAAAAAV2YWx1ZQAAAAAAABMAAAAA",
        "AAAAAAAAAAAAAAAXcmVtb3ZlX2J5dGVzMzJfZnJvbV9zZXQAAAAAAwAAAAAAAAAGY2FsbGVyAAAAAAATAAAAAAAAAAdzZXRfa2V5AAAAA+4AAAAgAAAAAAAAAAV2YWx1ZQAAAAAAA+4AAAAgAAAAAA==",
        "AAAAAAAAALFSZXZva2UgdGhlIHBvc2l0aW9uIG1hbmFnZXIgZm9yIGEgZ2l2ZW4gb3duZXIgYW5kIG1hcmtldC4KT25seSB0aGUgb3duZXIgY2FuIGNhbGwgdGhpcy4gUmV0dXJucyB0cnVlIGlmIGEgbWFuYWdlciB3YXMgcHJldmlvdXNseQpzZXQgKGFuZCBpcyBub3cgcmVtb3ZlZCksIGZhbHNlIGlmIG5vbmUgd2FzIHNldC4AAAAAAAAXcmVtb3ZlX3Bvc2l0aW9uX21hbmFnZXIAAAAAAgAAAAAAAAAFb3duZXIAAAAAAAATAAAAAAAAAAZtYXJrZXQAAAAAABMAAAABAAAAAQ==",
        "AAAAAAAAAIhHZXQgdGhlIGxpcXVpZGF0aW9uIGV4ZWN1dGlvbiBmZWUgZm9yIGEgZ2l2ZW4gbWFya2V0LgpUaGlzIGZlZSBpcyBwYWlkIHRvIHRoZSBrZWVwZXIgZnJvbSBwb3NpdGlvbiBjb2xsYXRlcmFsIG9uIHN1Y2Nlc3NmdWwgbGlxdWlkYXRpb24uAAAAHWdldF9saXF1aWRhdGlvbl9leGVjdXRpb25fZmVlAAAAAAAAAQAAAAAAAAAGbWFya2V0AAAAAAATAAAAAQAAAAo=",
        "AAAAAAAAANxTZXQgdGhlIGxpcXVpZGF0aW9uIGV4ZWN1dGlvbiBmZWUgZm9yIGEgZ2l2ZW4gbWFya2V0IChhZG1pbi1vbmx5KS4gQ2FwcGVkIGF0Ck1BWF9MSVFVSURBVElPTl9FWEVDVVRJT05fRkVFIHNvIGEgZmF0LWZpbmdlcmVkL21pc3VzZWQgdmFsdWUgY2FuJ3Qgc2VpemUgYQpsaXF1aWRhdGVkIHBvc2l0aW9uJ3MgZW50aXJlIGNvbGxhdGVyYWwgYXMga2VlcGVyIGZlZSAoaXNzdWUgIzYzMykuAAAAHXNldF9saXF1aWRhdGlvbl9leGVjdXRpb25fZmVlAAAAAAAAAwAAAAAAAAAGY2FsbGVyAAAAAAATAAAAAAAAAAZtYXJrZXQAAAAAABMAAAAAAAAAA2ZlZQAAAAAKAAAAAQAAAAo=" ]),
      options
    )
  }
  public readonly fromJSON = {
    get_bool: this.txFromJSON<boolean>,
        get_i128: this.txFromJSON<i128>,
        get_u128: this.txFromJSON<u128>,
        set_bool: this.txFromJSON<boolean>,
        set_i128: this.txFromJSON<i128>,
        set_u128: this.txFromJSON<u128>,
        get_nonce: this.txFromJSON<u64>,
        initialize: this.txFromJSON<null>,
        get_address: this.txFromJSON<Option<string>>,
        get_bytes32: this.txFromJSON<Buffer>,
        remove_bool: this.txFromJSON<null>,
        remove_i128: this.txFromJSON<null>,
        remove_u128: this.txFromJSON<null>,
        set_address: this.txFromJSON<string>,
        set_bytes32: this.txFromJSON<Buffer>,
        decrement_u128: this.txFromJSON<u128>,
        get_u128_batch: this.txFromJSON<Array<u128>>,
        increment_u128: this.txFromJSON<u128>,
        remove_address: this.txFromJSON<null>,
        remove_bytes32: this.txFromJSON<null>,
        get_u128_cached: this.txFromJSON<u128>,
        increment_nonce: this.txFromJSON<u64>,
        set_u128_config: this.txFromJSON<u128>,
        contains_address: this.txFromJSON<boolean>,
        contains_bytes32: this.txFromJSON<boolean>,
        get_keeper_stats: this.txFromJSON<readonly [u128, u128, u128]>,
        get_i128_instance: this.txFromJSON<i128>,
        get_u128_instance: this.txFromJSON<u128>,
        set_i128_instance: this.txFromJSON<i128>,
        set_u128_instance: this.txFromJSON<u128>,
        add_address_to_set: this.txFromJSON<null>,
        add_bytes32_to_set: this.txFromJSON<null>,
        get_address_set_at: this.txFromJSON<Array<string>>,
        get_bytes32_set_at: this.txFromJSON<Array<Buffer>>,
        apply_delta_to_i128: this.txFromJSON<i128>,
        apply_delta_to_u128: this.txFromJSON<u128>,
        is_keeper_suspended: this.txFromJSON<boolean>,
        get_position_manager: this.txFromJSON<Option<string>>,
        set_position_manager: this.txFromJSON<string>,
        get_address_set_count: this.txFromJSON<u32>,
        get_bytes32_set_count: this.txFromJSON<u32>,
        get_min_execution_fee: this.txFromJSON<u128>,
        set_min_execution_fee: this.txFromJSON<u128>,
        record_keeper_execution: this.txFromJSON<null>,
        remove_address_from_set: this.txFromJSON<null>,
        remove_bytes32_from_set: this.txFromJSON<null>,
        remove_position_manager: this.txFromJSON<boolean>,
        get_liquidation_execution_fee: this.txFromJSON<u128>,
        set_liquidation_execution_fee: this.txFromJSON<u128>
  }
}