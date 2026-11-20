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
  1: {message:"TooManyEntries"}
}


export interface BatchClaimResult {
  claims_attempted: u32;
  /**
 * (market, token) pairs whose `claim_fees` call failed — isolated so one
 * bad pair does not revert the fees already claimed for every other pair.
 */
failed: Array<readonly [string, string]>;
  markets: u32;
  tokens: u32;
  total_claimed: u128;
}


export const Errors = {
  1: {message:"AlreadyInitialized"},
  2: {message:"NotInitialized"},
  3: {message:"Unauthorized"},
  4: {message:"NothingToClaim"},
  5: {message:"InvalidAmount"},
  /**
   * Issue #254: the pool's tracked `pool_amount` is less than the stored
   * claimable amount — a rounding-accumulation discrepancy — so the claim
   * is rejected rather than allowed to overdraft the pool's accounting.
   */
  6: {message:"InsufficientPoolBalance"},
  /**
   * No protocol or market fee recipient is configured, so protocol fees have
   * no trusted destination and stay claimable.
   */
  7: {message:"FeeReceiverNotSet"},
  /**
   * The recipient would be the market pool itself or this handler — paying
   * fees there would decrement `pool_amount` without tokens leaving the pool.
   */
  8: {message:"InvalidFeeReceiver"}
}








export interface Client {
  /**
   * Construct and simulate a claim_all_fees transaction. Returns an `AssembledTransaction` object which will have a `result` field containing the result of the simulation. If this transaction changes contract state, you will need to call `signAndSend()` on the returned object.
   * Claim protocol fees across all market/token combinations in one call.
   * 
   * `fee_handler` remains responsible for the actual transfer, the
   * FEE_KEEPER authorization check, and choosing the configured recipient.
   * Zero balances are skipped because `fee_handler::claim_fees` returns `0`
   * for them; a market with no configured recipient is reported in `failed`.
   */
  claim_all_fees: ({fee_handler, keeper, markets, tokens}: {fee_handler: string, keeper: string, markets: Array<string>, tokens: Array<string>}, options?: MethodOptions) => Promise<AssembledTransaction<BatchClaimResult>>

  /**
   * Construct and simulate a upgrade transaction. Returns an `AssembledTransaction` object which will have a `result` field containing the result of the simulation. If this transaction changes contract state, you will need to call `signAndSend()` on the returned object.
   * Upgrade the contract wasm. Only the stored admin may call this.
   */
  upgrade: ({new_wasm_hash}: {new_wasm_hash: Buffer}, options?: MethodOptions) => Promise<AssembledTransaction<null>>

  /**
   * Construct and simulate a claim_fees transaction. Returns an `AssembledTransaction` object which will have a `result` field containing the result of the simulation. If this transaction changes contract state, you will need to call `signAndSend()` on the returned object.
   * Sweep accumulated protocol fees for a market/token to the market's
   * configured fee recipient. FEE_KEEPER only.
   * 
   * The destination is never caller-supplied: it is the per-market override
   * set by `set_market_fee_receiver`, or else the protocol recipient set by
   * `set_fee_receiver`. With neither configured the call reverts with
   * `FeeReceiverNotSet` and the fees stay claimable.
   * 
   * Before withdrawing, the claimable amount is validated against the pool's
   * tracked `pool_amount` and rejected with `InsufficientPoolBalance` if it
   * would overdraft it (issue #254). The actual SEP-41 balance is then read as
   * a second guard and the transfer is capped at `min(claimable, pool_balance)`.
   * In practice all three values agree because fees are accrued with ceiling
   * rounding, so the pool always holds at least as many tokens as are recorded
   * as claimable. `pool_amount` is decremented by exactly what is transferred.
   */
  claim_fees: ({keeper, market, token}: {keeper: string, market: string, token: string}, options?: MethodOptions) => Promise<AssembledTransaction<u128>>

  /**
   * Construct and simulate a initialize transaction. Returns an `AssembledTransaction` object which will have a `result` field containing the result of the simulation. If this transaction changes contract state, you will need to call `signAndSend()` on the returned object.
   */
  initialize: ({admin, role_store, data_store}: {admin: string, role_store: string, data_store: string}, options?: MethodOptions) => Promise<AssembledTransaction<null>>

  /**
   * Construct and simulate a accrue_ui_fee transaction. Returns an `AssembledTransaction` object which will have a `result` field containing the result of the simulation. If this transaction changes contract state, you will need to call `signAndSend()` on the returned object.
   * Accrue a UI fee on behalf of a receiver.
   * 
   * NOTE (issue #602): despite the name, no real trade path calls this yet —
   * `exchange_router`, `order_handler`, and `swap_utils` never invoke it, and
   * nothing computes `amount` from `get_ui_fee_factor`. The
   * `set_ui_fee_factor`/`accrue_ui_fee` UI-fee-attribution feature is
   * configuration-and-accrual scaffolding only; it has no effect on any real
   * swap or trade until a caller with CONTROLLER wires this into execution.
   * 
   * Only a caller that holds the CONTROLLER role may accrue fees; this prevents
   * arbitrary inflation of a receiver's balance.
   */
  accrue_ui_fee: ({controller, token, ui_receiver, amount}: {controller: string, token: string, ui_receiver: string, amount: u128}, options?: MethodOptions) => Promise<AssembledTransaction<null>>

  /**
   * Construct and simulate a claim_ui_fees transaction. Returns an `AssembledTransaction` object which will have a `result` field containing the result of the simulation. If this transaction changes contract state, you will need to call `signAndSend()` on the returned object.
   * Claim all accrued UI fees for the calling receiver.
   * 
   * A receiver may only claim their own balance — passing a different address as
   * `ui_receiver` will fail the `require_auth()` check.
   * 
   * The withdrawal is capped at the pool's actual token balance (issue #254).
   */
  claim_ui_fees: ({ui_receiver, market, token}: {ui_receiver: string, market: string, token: string}, options?: MethodOptions) => Promise<AssembledTransaction<u128>>

  /**
   * Construct and simulate a claimable_fees transaction. Returns an `AssembledTransaction` object which will have a `result` field containing the result of the simulation. If this transaction changes contract state, you will need to call `signAndSend()` on the returned object.
   * Return the accumulated protocol fee amount for a given market + token.
   */
  claimable_fees: ({market, token}: {market: string, token: string}, options?: MethodOptions) => Promise<AssembledTransaction<u128>>

  /**
   * Construct and simulate a get_fee_receiver transaction. Returns an `AssembledTransaction` object which will have a `result` field containing the result of the simulation. If this transaction changes contract state, you will need to call `signAndSend()` on the returned object.
   * Return the recipient `claim_fees` will pay for `market`: its override if
   * set, else the protocol recipient, else `None`.
   */
  get_fee_receiver: ({market}: {market: string}, options?: MethodOptions) => Promise<AssembledTransaction<Option<string>>>

  /**
   * Construct and simulate a is_auto_compound transaction. Returns an `AssembledTransaction` object which will have a `result` field containing the result of the simulation. If this transaction changes contract state, you will need to call `signAndSend()` on the returned object.
   * Return whether auto-compound mode is enabled for a market (issue #285).
   */
  is_auto_compound: ({market}: {market: string}, options?: MethodOptions) => Promise<AssembledTransaction<boolean>>

  /**
   * Construct and simulate a set_fee_receiver transaction. Returns an `AssembledTransaction` object which will have a `result` field containing the result of the simulation. If this transaction changes contract state, you will need to call `signAndSend()` on the returned object.
   * Set the protocol-wide fee recipient (treasury). Admin only; emits
   * `FeeReceiverSet` with `market: None`. The recipient cannot be cleared,
   * only replaced, so a configured protocol never loses its fee destination.
   */
  set_fee_receiver: ({receiver}: {receiver: string}, options?: MethodOptions) => Promise<AssembledTransaction<null>>

  /**
   * Construct and simulate a claimable_ui_fees transaction. Returns an `AssembledTransaction` object which will have a `result` field containing the result of the simulation. If this transaction changes contract state, you will need to call `signAndSend()` on the returned object.
   * Return the accumulated UI fee for a (token, ui_receiver) pair.
   */
  claimable_ui_fees: ({token, ui_receiver}: {token: string, ui_receiver: string}, options?: MethodOptions) => Promise<AssembledTransaction<u128>>

  /**
   * Construct and simulate a get_ui_fee_factor transaction. Returns an `AssembledTransaction` object which will have a `result` field containing the result of the simulation. If this transaction changes contract state, you will need to call `signAndSend()` on the returned object.
   * Return the stored UI fee factor for a given receiver (0 if unset).
   */
  get_ui_fee_factor: ({ui_receiver}: {ui_receiver: string}, options?: MethodOptions) => Promise<AssembledTransaction<u128>>

  /**
   * Construct and simulate a set_auto_compound transaction. Returns an `AssembledTransaction` object which will have a `result` field containing the result of the simulation. If this transaction changes contract state, you will need to call `signAndSend()` on the returned object.
   * Enable or disable auto-compound mode for a market (admin only, issue #285).
   * 
   * When enabled, position fees are retained in `pool_amount` (they are already
   * added to the pool on every order execution) and `claim_fees` returns 0 for
   * this market. Toggling the flag does not disturb existing positions.
   */
  set_auto_compound: ({market, enabled}: {market: string, enabled: boolean}, options?: MethodOptions) => Promise<AssembledTransaction<null>>

  /**
   * Construct and simulate a set_ui_fee_factor transaction. Returns an `AssembledTransaction` object which will have a `result` field containing the result of the simulation. If this transaction changes contract state, you will need to call `signAndSend()` on the returned object.
   * Set the UI fee factor for a given receiver. Only the stored admin may call.
   * 
   * `factor` must be ≤ FLOAT_PRECISION (10^30, i.e. 100%). A factor above this
   * is nonsensical (> 100% fee) and is rejected with `InvalidAmount`.
   */
  set_ui_fee_factor: ({ui_receiver, factor}: {ui_receiver: string, factor: u128}, options?: MethodOptions) => Promise<AssembledTransaction<null>>

  /**
   * Construct and simulate a claim_funding_fees transaction. Returns an `AssembledTransaction` object which will have a `result` field containing the result of the simulation. If this transaction changes contract state, you will need to call `signAndSend()` on the returned object.
   * Claim funding fees earned by a position account. Anyone can call for their own account.
   * 
   * The withdrawal is capped at the pool's actual token balance (issue #254).
   */
  claim_funding_fees: ({account, market, token}: {account: string, market: string, token: string}, options?: MethodOptions) => Promise<AssembledTransaction<u128>>

  /**
   * Construct and simulate a record_fee_accrual transaction. Returns an `AssembledTransaction` object which will have a `result` field containing the result of the simulation. If this transaction changes contract state, you will need to call `signAndSend()` on the returned object.
   * Record fee accrual and emit FeeAccrued event with fee_type breakdown (Issue #515).
   * fee_type: 1 = Position, 2 = Funding, 3 = Borrowing, 4 = Swap, 5 = Liquidation
   */
  record_fee_accrual: ({caller, market, token, fee_type, amount}: {caller: string, market: string, token: string, fee_type: u32, amount: u128}, options?: MethodOptions) => Promise<AssembledTransaction<null>>

  /**
   * Construct and simulate a get_market_fee_receiver transaction. Returns an `AssembledTransaction` object which will have a `result` field containing the result of the simulation. If this transaction changes contract state, you will need to call `signAndSend()` on the returned object.
   * Return `market`'s fee-recipient override, if configured.
   */
  get_market_fee_receiver: ({market}: {market: string}, options?: MethodOptions) => Promise<AssembledTransaction<Option<string>>>

  /**
   * Construct and simulate a set_market_fee_receiver transaction. Returns an `AssembledTransaction` object which will have a `result` field containing the result of the simulation. If this transaction changes contract state, you will need to call `signAndSend()` on the returned object.
   * Set (`Some`) or clear (`None`) `market`'s fee-recipient override for
   * issuer- or partner-specific destinations. Admin only; emits
   * `FeeReceiverSet` with `market: Some(market)`. The override may not be the
   * market pool itself or this handler.
   */
  set_market_fee_receiver: ({market, receiver}: {market: string, receiver: Option<string>}, options?: MethodOptions) => Promise<AssembledTransaction<null>>

  /**
   * Construct and simulate a get_protocol_fee_receiver transaction. Returns an `AssembledTransaction` object which will have a `result` field containing the result of the simulation. If this transaction changes contract state, you will need to call `signAndSend()` on the returned object.
   * Return the protocol-wide fee recipient, if configured.
   */
  get_protocol_fee_receiver: (options?: MethodOptions) => Promise<AssembledTransaction<Option<string>>>

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
      new ContractSpec([ "AAAABAAAAAAAAAAAAAAABUVycm9yAAAAAAAAAQAAAAAAAAAOVG9vTWFueUVudHJpZXMAAAAAAAE=",
        "AAAAAQAAAAAAAAAAAAAAEEJhdGNoQ2xhaW1SZXN1bHQAAAAFAAAAAAAAABBjbGFpbXNfYXR0ZW1wdGVkAAAABAAAAJAobWFya2V0LCB0b2tlbikgcGFpcnMgd2hvc2UgYGNsYWltX2ZlZXNgIGNhbGwgZmFpbGVkIOKAlCBpc29sYXRlZCBzbyBvbmUKYmFkIHBhaXIgZG9lcyBub3QgcmV2ZXJ0IHRoZSBmZWVzIGFscmVhZHkgY2xhaW1lZCBmb3IgZXZlcnkgb3RoZXIgcGFpci4AAAAGZmFpbGVkAAAAAAPqAAAD7QAAAAIAAAATAAAAEwAAAAAAAAAHbWFya2V0cwAAAAAEAAAAAAAAAAZ0b2tlbnMAAAAAAAQAAAAAAAAADXRvdGFsX2NsYWltZWQAAAAAAAAK",
        "AAAABQAAAAAAAAAAAAAAEEJhdGNoRmVlc0NsYWltZWQAAAABAAAACWZlZV9iYXRjaAAAAAAAAAQAAAAAAAAABmtlZXBlcgAAAAAAEwAAAAAAAAAAAAAADG1hcmtldF9jb3VudAAAAAQAAAAAAAAAAAAAAAt0b2tlbl9jb3VudAAAAAAEAAAAAAAAAAAAAAANdG90YWxfY2xhaW1lZAAAAAAAAAoAAAAAAAAAAg==",
        "AAAAAAAAAV1DbGFpbSBwcm90b2NvbCBmZWVzIGFjcm9zcyBhbGwgbWFya2V0L3Rva2VuIGNvbWJpbmF0aW9ucyBpbiBvbmUgY2FsbC4KCmBmZWVfaGFuZGxlcmAgcmVtYWlucyByZXNwb25zaWJsZSBmb3IgdGhlIGFjdHVhbCB0cmFuc2ZlciwgdGhlCkZFRV9LRUVQRVIgYXV0aG9yaXphdGlvbiBjaGVjaywgYW5kIGNob29zaW5nIHRoZSBjb25maWd1cmVkIHJlY2lwaWVudC4KWmVybyBiYWxhbmNlcyBhcmUgc2tpcHBlZCBiZWNhdXNlIGBmZWVfaGFuZGxlcjo6Y2xhaW1fZmVlc2AgcmV0dXJucyBgMGAKZm9yIHRoZW07IGEgbWFya2V0IHdpdGggbm8gY29uZmlndXJlZCByZWNpcGllbnQgaXMgcmVwb3J0ZWQgaW4gYGZhaWxlZGAuAAAAAAAADmNsYWltX2FsbF9mZWVzAAAAAAAEAAAAAAAAAAtmZWVfaGFuZGxlcgAAAAATAAAAAAAAAAZrZWVwZXIAAAAAABMAAAAAAAAAB21hcmtldHMAAAAD6gAAABMAAAAAAAAABnRva2VucwAAAAAD6gAAABMAAAABAAAH0AAAABBCYXRjaENsYWltUmVzdWx0",
        "AAAABAAAAAAAAAAAAAAABUVycm9yAAAAAAAACAAAAAAAAAASQWxyZWFkeUluaXRpYWxpemVkAAAAAAABAAAAAAAAAA5Ob3RJbml0aWFsaXplZAAAAAAAAgAAAAAAAAAMVW5hdXRob3JpemVkAAAAAwAAAAAAAAAOTm90aGluZ1RvQ2xhaW0AAAAAAAQAAAAAAAAADUludmFsaWRBbW91bnQAAAAAAAAFAAAA0klzc3VlICMyNTQ6IHRoZSBwb29sJ3MgdHJhY2tlZCBgcG9vbF9hbW91bnRgIGlzIGxlc3MgdGhhbiB0aGUgc3RvcmVkCmNsYWltYWJsZSBhbW91bnQg4oCUIGEgcm91bmRpbmctYWNjdW11bGF0aW9uIGRpc2NyZXBhbmN5IOKAlCBzbyB0aGUgY2xhaW0KaXMgcmVqZWN0ZWQgcmF0aGVyIHRoYW4gYWxsb3dlZCB0byBvdmVyZHJhZnQgdGhlIHBvb2wncyBhY2NvdW50aW5nLgAAAAAAF0luc3VmZmljaWVudFBvb2xCYWxhbmNlAAAAAAYAAABzTm8gcHJvdG9jb2wgb3IgbWFya2V0IGZlZSByZWNpcGllbnQgaXMgY29uZmlndXJlZCwgc28gcHJvdG9jb2wgZmVlcyBoYXZlCm5vIHRydXN0ZWQgZGVzdGluYXRpb24gYW5kIHN0YXkgY2xhaW1hYmxlLgAAAAARRmVlUmVjZWl2ZXJOb3RTZXQAAAAAAAAHAAAAklRoZSByZWNpcGllbnQgd291bGQgYmUgdGhlIG1hcmtldCBwb29sIGl0c2VsZiBvciB0aGlzIGhhbmRsZXIg4oCUIHBheWluZwpmZWVzIHRoZXJlIHdvdWxkIGRlY3JlbWVudCBgcG9vbF9hbW91bnRgIHdpdGhvdXQgdG9rZW5zIGxlYXZpbmcgdGhlIHBvb2wuAAAAAAASSW52YWxpZEZlZVJlY2VpdmVyAAAAAAAI",
        "AAAABQAAAAAAAAAAAAAACkZlZUFjY3J1ZWQAAAAAAAEAAAAHZmVlX2FjYwAAAAAEAAAAAAAAAAZtYXJrZXQAAAAAABMAAAAAAAAAAAAAAAV0b2tlbgAAAAAAABMAAAAAAAAAAAAAAAhmZWVfdHlwZQAAAAQAAAAAAAAAAAAAAAZhbW91bnQAAAAAAAoAAAAAAAAAAg==",
        "AAAABQAAAAAAAAAAAAAACkZlZUNsYWltZWQAAAAAAAEAAAAHZmVlX2NsbQAAAAAEAAAAAAAAAAZtYXJrZXQAAAAAABMAAAAAAAAAAAAAAAV0b2tlbgAAAAAAABMAAAAAAAAAAAAAAAZhbW91bnQAAAAAAAoAAAAAAAAAAAAAAAhyZWNlaXZlcgAAABMAAAAAAAAAAg==",
        "AAAABQAAAAAAAAAAAAAADFVpRmVlQWNjcnVlZAAAAAEAAAAKdWlfZmVlX2FjYwAAAAAAAwAAAAAAAAALdWlfcmVjZWl2ZXIAAAAAEwAAAAAAAAAAAAAABXRva2VuAAAAAAAAEwAAAAAAAAAAAAAABmFtb3VudAAAAAAACgAAAAAAAAAC",
        "AAAABQAAAAAAAAAAAAAADFVpRmVlQ2xhaW1lZAAAAAEAAAAKdWlfZmVlX2NsbQAAAAAAAwAAAAAAAAALdWlfcmVjZWl2ZXIAAAAAEwAAAAAAAAAAAAAABXRva2VuAAAAAAAAEwAAAAAAAAAAAAAABmFtb3VudAAAAAAACgAAAAAAAAAC",
        "AAAABQAAAO5FbWl0dGVkIG9uIGV2ZXJ5IGNoYW5nZSB0byBhIGZlZSByZWNpcGllbnQuIGBtYXJrZXRgIGlzIGBOb25lYCBmb3IgdGhlCnByb3RvY29sLXdpZGUgcmVjaXBpZW50IGFuZCBgU29tZWAgZm9yIGEgcGVyLW1hcmtldCBvdmVycmlkZTsgYHJlY2VpdmVyYCBpcwpgTm9uZWAgd2hlbiBhIG1hcmtldCBvdmVycmlkZSBpcyBjbGVhcmVkICh0aGUgbWFya2V0IGZhbGxzIGJhY2sgdG8gdGhlCnByb3RvY29sIHJlY2lwaWVudCkuAAAAAAAAAAAADkZlZVJlY2VpdmVyU2V0AAAAAAABAAAAB2ZlZV9yY3YAAAAAAwAAAAAAAAAGbWFya2V0AAAAAAPoAAAAEwAAAAAAAAAAAAAACHByZXZpb3VzAAAD6AAAABMAAAAAAAAAAAAAAAhyZWNlaXZlcgAAA+gAAAATAAAAAAAAAAI=",
        "AAAABQAAAAAAAAAAAAAADlVpRmVlRmFjdG9yU2V0AAAAAAABAAAACnVpX2ZlZV9zZXQAAAAAAAIAAAAAAAAAC3VpX3JlY2VpdmVyAAAAABMAAAAAAAAAAAAAAAZmYWN0b3IAAAAAAAoAAAAAAAAAAg==",
        "AAAABQAAAAAAAAAAAAAAEUZ1bmRpbmdGZWVDbGFpbWVkAAAAAAAAAQAAAAdmbmRfY2xtAAAAAAQAAAAAAAAAB2FjY291bnQAAAAAEwAAAAAAAAAAAAAABm1hcmtldAAAAAAAEwAAAAAAAAAAAAAABXRva2VuAAAAAAAAEwAAAAAAAAAAAAAABmFtb3VudAAAAAAACgAAAAAAAAAC",
        "AAAAAAAAAD9VcGdyYWRlIHRoZSBjb250cmFjdCB3YXNtLiBPbmx5IHRoZSBzdG9yZWQgYWRtaW4gbWF5IGNhbGwgdGhpcy4AAAAAB3VwZ3JhZGUAAAAAAQAAAAAAAAANbmV3X3dhc21faGFzaAAAAAAAA+4AAAAgAAAAAA==",
        "AAAAAAAAA3pTd2VlcCBhY2N1bXVsYXRlZCBwcm90b2NvbCBmZWVzIGZvciBhIG1hcmtldC90b2tlbiB0byB0aGUgbWFya2V0J3MKY29uZmlndXJlZCBmZWUgcmVjaXBpZW50LiBGRUVfS0VFUEVSIG9ubHkuCgpUaGUgZGVzdGluYXRpb24gaXMgbmV2ZXIgY2FsbGVyLXN1cHBsaWVkOiBpdCBpcyB0aGUgcGVyLW1hcmtldCBvdmVycmlkZQpzZXQgYnkgYHNldF9tYXJrZXRfZmVlX3JlY2VpdmVyYCwgb3IgZWxzZSB0aGUgcHJvdG9jb2wgcmVjaXBpZW50IHNldCBieQpgc2V0X2ZlZV9yZWNlaXZlcmAuIFdpdGggbmVpdGhlciBjb25maWd1cmVkIHRoZSBjYWxsIHJldmVydHMgd2l0aApgRmVlUmVjZWl2ZXJOb3RTZXRgIGFuZCB0aGUgZmVlcyBzdGF5IGNsYWltYWJsZS4KCkJlZm9yZSB3aXRoZHJhd2luZywgdGhlIGNsYWltYWJsZSBhbW91bnQgaXMgdmFsaWRhdGVkIGFnYWluc3QgdGhlIHBvb2wncwp0cmFja2VkIGBwb29sX2Ftb3VudGAgYW5kIHJlamVjdGVkIHdpdGggYEluc3VmZmljaWVudFBvb2xCYWxhbmNlYCBpZiBpdAp3b3VsZCBvdmVyZHJhZnQgaXQgKGlzc3VlICMyNTQpLiBUaGUgYWN0dWFsIFNFUC00MSBiYWxhbmNlIGlzIHRoZW4gcmVhZCBhcwphIHNlY29uZCBndWFyZCBhbmQgdGhlIHRyYW5zZmVyIGlzIGNhcHBlZCBhdCBgbWluKGNsYWltYWJsZSwgcG9vbF9iYWxhbmNlKWAuCkluIHByYWN0aWNlIGFsbCB0aHJlZSB2YWx1ZXMgYWdyZWUgYmVjYXVzZSBmZWVzIGFyZSBhY2NydWVkIHdpdGggY2VpbGluZwpyb3VuZGluZywgc28gdGhlIHBvb2wgYWx3YXlzIGhvbGRzIGF0IGxlYXN0IGFzIG1hbnkgdG9rZW5zIGFzIGFyZSByZWNvcmRlZAphcyBjbGFpbWFibGUuIGBwb29sX2Ftb3VudGAgaXMgZGVjcmVtZW50ZWQgYnkgZXhhY3RseSB3aGF0IGlzIHRyYW5zZmVycmVkLgAAAAAACmNsYWltX2ZlZXMAAAAAAAMAAAAAAAAABmtlZXBlcgAAAAAAEwAAAAAAAAAGbWFya2V0AAAAAAATAAAAAAAAAAV0b2tlbgAAAAAAABMAAAABAAAACg==",
        "AAAAAAAAAAAAAAAKaW5pdGlhbGl6ZQAAAAAAAwAAAAAAAAAFYWRtaW4AAAAAAAATAAAAAAAAAApyb2xlX3N0b3JlAAAAAAATAAAAAAAAAApkYXRhX3N0b3JlAAAAAAATAAAAAA==",
        "AAAAAAAAAkNBY2NydWUgYSBVSSBmZWUgb24gYmVoYWxmIG9mIGEgcmVjZWl2ZXIuCgpOT1RFIChpc3N1ZSAjNjAyKTogZGVzcGl0ZSB0aGUgbmFtZSwgbm8gcmVhbCB0cmFkZSBwYXRoIGNhbGxzIHRoaXMgeWV0IOKAlApgZXhjaGFuZ2Vfcm91dGVyYCwgYG9yZGVyX2hhbmRsZXJgLCBhbmQgYHN3YXBfdXRpbHNgIG5ldmVyIGludm9rZSBpdCwgYW5kCm5vdGhpbmcgY29tcHV0ZXMgYGFtb3VudGAgZnJvbSBgZ2V0X3VpX2ZlZV9mYWN0b3JgLiBUaGUKYHNldF91aV9mZWVfZmFjdG9yYC9gYWNjcnVlX3VpX2ZlZWAgVUktZmVlLWF0dHJpYnV0aW9uIGZlYXR1cmUgaXMKY29uZmlndXJhdGlvbi1hbmQtYWNjcnVhbCBzY2FmZm9sZGluZyBvbmx5OyBpdCBoYXMgbm8gZWZmZWN0IG9uIGFueSByZWFsCnN3YXAgb3IgdHJhZGUgdW50aWwgYSBjYWxsZXIgd2l0aCBDT05UUk9MTEVSIHdpcmVzIHRoaXMgaW50byBleGVjdXRpb24uCgpPbmx5IGEgY2FsbGVyIHRoYXQgaG9sZHMgdGhlIENPTlRST0xMRVIgcm9sZSBtYXkgYWNjcnVlIGZlZXM7IHRoaXMgcHJldmVudHMKYXJiaXRyYXJ5IGluZmxhdGlvbiBvZiBhIHJlY2VpdmVyJ3MgYmFsYW5jZS4AAAAADWFjY3J1ZV91aV9mZWUAAAAAAAAEAAAAAAAAAApjb250cm9sbGVyAAAAAAATAAAAAAAAAAV0b2tlbgAAAAAAABMAAAAAAAAAC3VpX3JlY2VpdmVyAAAAABMAAAAAAAAABmFtb3VudAAAAAAACgAAAAA=",
        "AAAAAAAAAQJDbGFpbSBhbGwgYWNjcnVlZCBVSSBmZWVzIGZvciB0aGUgY2FsbGluZyByZWNlaXZlci4KCkEgcmVjZWl2ZXIgbWF5IG9ubHkgY2xhaW0gdGhlaXIgb3duIGJhbGFuY2Ug4oCUIHBhc3NpbmcgYSBkaWZmZXJlbnQgYWRkcmVzcyBhcwpgdWlfcmVjZWl2ZXJgIHdpbGwgZmFpbCB0aGUgYHJlcXVpcmVfYXV0aCgpYCBjaGVjay4KClRoZSB3aXRoZHJhd2FsIGlzIGNhcHBlZCBhdCB0aGUgcG9vbCdzIGFjdHVhbCB0b2tlbiBiYWxhbmNlIChpc3N1ZSAjMjU0KS4AAAAAAA1jbGFpbV91aV9mZWVzAAAAAAAAAwAAAAAAAAALdWlfcmVjZWl2ZXIAAAAAEwAAAAAAAAAGbWFya2V0AAAAAAATAAAAAAAAAAV0b2tlbgAAAAAAABMAAAABAAAACg==",
        "AAAAAAAAAEZSZXR1cm4gdGhlIGFjY3VtdWxhdGVkIHByb3RvY29sIGZlZSBhbW91bnQgZm9yIGEgZ2l2ZW4gbWFya2V0ICsgdG9rZW4uAAAAAAAOY2xhaW1hYmxlX2ZlZXMAAAAAAAIAAAAAAAAABm1hcmtldAAAAAAAEwAAAAAAAAAFdG9rZW4AAAAAAAATAAAAAQAAAAo=",
        "AAAAAAAAAHdSZXR1cm4gdGhlIHJlY2lwaWVudCBgY2xhaW1fZmVlc2Agd2lsbCBwYXkgZm9yIGBtYXJrZXRgOiBpdHMgb3ZlcnJpZGUgaWYKc2V0LCBlbHNlIHRoZSBwcm90b2NvbCByZWNpcGllbnQsIGVsc2UgYE5vbmVgLgAAAAAQZ2V0X2ZlZV9yZWNlaXZlcgAAAAEAAAAAAAAABm1hcmtldAAAAAAAEwAAAAEAAAPoAAAAEw==",
        "AAAAAAAAAEdSZXR1cm4gd2hldGhlciBhdXRvLWNvbXBvdW5kIG1vZGUgaXMgZW5hYmxlZCBmb3IgYSBtYXJrZXQgKGlzc3VlICMyODUpLgAAAAAQaXNfYXV0b19jb21wb3VuZAAAAAEAAAAAAAAABm1hcmtldAAAAAAAEwAAAAEAAAAB",
        "AAAAAAAAANFTZXQgdGhlIHByb3RvY29sLXdpZGUgZmVlIHJlY2lwaWVudCAodHJlYXN1cnkpLiBBZG1pbiBvbmx5OyBlbWl0cwpgRmVlUmVjZWl2ZXJTZXRgIHdpdGggYG1hcmtldDogTm9uZWAuIFRoZSByZWNpcGllbnQgY2Fubm90IGJlIGNsZWFyZWQsCm9ubHkgcmVwbGFjZWQsIHNvIGEgY29uZmlndXJlZCBwcm90b2NvbCBuZXZlciBsb3NlcyBpdHMgZmVlIGRlc3RpbmF0aW9uLgAAAAAAABBzZXRfZmVlX3JlY2VpdmVyAAAAAQAAAAAAAAAIcmVjZWl2ZXIAAAATAAAAAA==",
        "AAAAAAAAAD5SZXR1cm4gdGhlIGFjY3VtdWxhdGVkIFVJIGZlZSBmb3IgYSAodG9rZW4sIHVpX3JlY2VpdmVyKSBwYWlyLgAAAAAAEWNsYWltYWJsZV91aV9mZWVzAAAAAAAAAgAAAAAAAAAFdG9rZW4AAAAAAAATAAAAAAAAAAt1aV9yZWNlaXZlcgAAAAATAAAAAQAAAAo=",
        "AAAAAAAAAEJSZXR1cm4gdGhlIHN0b3JlZCBVSSBmZWUgZmFjdG9yIGZvciBhIGdpdmVuIHJlY2VpdmVyICgwIGlmIHVuc2V0KS4AAAAAABFnZXRfdWlfZmVlX2ZhY3RvcgAAAAAAAAEAAAAAAAAAC3VpX3JlY2VpdmVyAAAAABMAAAABAAAACg==",
        "AAAAAAAAASdFbmFibGUgb3IgZGlzYWJsZSBhdXRvLWNvbXBvdW5kIG1vZGUgZm9yIGEgbWFya2V0IChhZG1pbiBvbmx5LCBpc3N1ZSAjMjg1KS4KCldoZW4gZW5hYmxlZCwgcG9zaXRpb24gZmVlcyBhcmUgcmV0YWluZWQgaW4gYHBvb2xfYW1vdW50YCAodGhleSBhcmUgYWxyZWFkeQphZGRlZCB0byB0aGUgcG9vbCBvbiBldmVyeSBvcmRlciBleGVjdXRpb24pIGFuZCBgY2xhaW1fZmVlc2AgcmV0dXJucyAwIGZvcgp0aGlzIG1hcmtldC4gVG9nZ2xpbmcgdGhlIGZsYWcgZG9lcyBub3QgZGlzdHVyYiBleGlzdGluZyBwb3NpdGlvbnMuAAAAABFzZXRfYXV0b19jb21wb3VuZAAAAAAAAAIAAAAAAAAABm1hcmtldAAAAAAAEwAAAAAAAAAHZW5hYmxlZAAAAAABAAAAAA==",
        "AAAAAAAAANtTZXQgdGhlIFVJIGZlZSBmYWN0b3IgZm9yIGEgZ2l2ZW4gcmVjZWl2ZXIuIE9ubHkgdGhlIHN0b3JlZCBhZG1pbiBtYXkgY2FsbC4KCmBmYWN0b3JgIG11c3QgYmUg4omkIEZMT0FUX1BSRUNJU0lPTiAoMTBeMzAsIGkuZS4gMTAwJSkuIEEgZmFjdG9yIGFib3ZlIHRoaXMKaXMgbm9uc2Vuc2ljYWwgKD4gMTAwJSBmZWUpIGFuZCBpcyByZWplY3RlZCB3aXRoIGBJbnZhbGlkQW1vdW50YC4AAAAAEXNldF91aV9mZWVfZmFjdG9yAAAAAAAAAgAAAAAAAAALdWlfcmVjZWl2ZXIAAAAAEwAAAAAAAAAGZmFjdG9yAAAAAAAKAAAAAA==",
        "AAAAAAAAAKJDbGFpbSBmdW5kaW5nIGZlZXMgZWFybmVkIGJ5IGEgcG9zaXRpb24gYWNjb3VudC4gQW55b25lIGNhbiBjYWxsIGZvciB0aGVpciBvd24gYWNjb3VudC4KClRoZSB3aXRoZHJhd2FsIGlzIGNhcHBlZCBhdCB0aGUgcG9vbCdzIGFjdHVhbCB0b2tlbiBiYWxhbmNlIChpc3N1ZSAjMjU0KS4AAAAAABJjbGFpbV9mdW5kaW5nX2ZlZXMAAAAAAAMAAAAAAAAAB2FjY291bnQAAAAAEwAAAAAAAAAGbWFya2V0AAAAAAATAAAAAAAAAAV0b2tlbgAAAAAAABMAAAABAAAACg==",
        "AAAAAAAAAKBSZWNvcmQgZmVlIGFjY3J1YWwgYW5kIGVtaXQgRmVlQWNjcnVlZCBldmVudCB3aXRoIGZlZV90eXBlIGJyZWFrZG93biAoSXNzdWUgIzUxNSkuCmZlZV90eXBlOiAxID0gUG9zaXRpb24sIDIgPSBGdW5kaW5nLCAzID0gQm9ycm93aW5nLCA0ID0gU3dhcCwgNSA9IExpcXVpZGF0aW9uAAAAEnJlY29yZF9mZWVfYWNjcnVhbAAAAAAABQAAAAAAAAAGY2FsbGVyAAAAAAATAAAAAAAAAAZtYXJrZXQAAAAAABMAAAAAAAAABXRva2VuAAAAAAAAEwAAAAAAAAAIZmVlX3R5cGUAAAAEAAAAAAAAAAZhbW91bnQAAAAAAAoAAAAA",
        "AAAAAAAAADhSZXR1cm4gYG1hcmtldGAncyBmZWUtcmVjaXBpZW50IG92ZXJyaWRlLCBpZiBjb25maWd1cmVkLgAAABdnZXRfbWFya2V0X2ZlZV9yZWNlaXZlcgAAAAABAAAAAAAAAAZtYXJrZXQAAAAAABMAAAABAAAD6AAAABM=",
        "AAAAAAAAAO5TZXQgKGBTb21lYCkgb3IgY2xlYXIgKGBOb25lYCkgYG1hcmtldGAncyBmZWUtcmVjaXBpZW50IG92ZXJyaWRlIGZvcgppc3N1ZXItIG9yIHBhcnRuZXItc3BlY2lmaWMgZGVzdGluYXRpb25zLiBBZG1pbiBvbmx5OyBlbWl0cwpgRmVlUmVjZWl2ZXJTZXRgIHdpdGggYG1hcmtldDogU29tZShtYXJrZXQpYC4gVGhlIG92ZXJyaWRlIG1heSBub3QgYmUgdGhlCm1hcmtldCBwb29sIGl0c2VsZiBvciB0aGlzIGhhbmRsZXIuAAAAAAAXc2V0X21hcmtldF9mZWVfcmVjZWl2ZXIAAAAAAgAAAAAAAAAGbWFya2V0AAAAAAATAAAAAAAAAAhyZWNlaXZlcgAAA+gAAAATAAAAAA==",
        "AAAAAAAAADZSZXR1cm4gdGhlIHByb3RvY29sLXdpZGUgZmVlIHJlY2lwaWVudCwgaWYgY29uZmlndXJlZC4AAAAAABlnZXRfcHJvdG9jb2xfZmVlX3JlY2VpdmVyAAAAAAAAAAAAAAEAAAPoAAAAEw==" ]),
      options
    )
  }
  public readonly fromJSON = {
    claim_all_fees: this.txFromJSON<BatchClaimResult>,
        upgrade: this.txFromJSON<null>,
        claim_fees: this.txFromJSON<u128>,
        initialize: this.txFromJSON<null>,
        accrue_ui_fee: this.txFromJSON<null>,
        claim_ui_fees: this.txFromJSON<u128>,
        claimable_fees: this.txFromJSON<u128>,
        get_fee_receiver: this.txFromJSON<Option<string>>,
        is_auto_compound: this.txFromJSON<boolean>,
        set_fee_receiver: this.txFromJSON<null>,
        claimable_ui_fees: this.txFromJSON<u128>,
        get_ui_fee_factor: this.txFromJSON<u128>,
        set_auto_compound: this.txFromJSON<null>,
        set_ui_fee_factor: this.txFromJSON<null>,
        claim_funding_fees: this.txFromJSON<u128>,
        record_fee_accrual: this.txFromJSON<null>,
        get_market_fee_receiver: this.txFromJSON<Option<string>>,
        set_market_fee_receiver: this.txFromJSON<null>,
        get_protocol_fee_receiver: this.txFromJSON<Option<string>>
  }
}