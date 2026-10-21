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
  4: {message:"InvalidAmount"}
}

export interface Client {
  /**
   * Construct and simulate a initialize transaction. Returns an `AssembledTransaction` object which will have a `result` field containing the result of the simulation. If this transaction changes contract state, you will need to call `signAndSend()` on the returned object.
   * One-time setup: store admin and role_store addresses (issue #781 —
   * admin was previously authenticated but never persisted, despite this
   * doc comment already claiming otherwise).
   */
  initialize: ({admin, role_store}: {admin: string, role_store: string}, options?: MethodOptions) => Promise<AssembledTransaction<null>>

  /**
   * Construct and simulate a transfer_out transaction. Returns an `AssembledTransaction` object which will have a `result` field containing the result of the simulation. If this transaction changes contract state, you will need to call `signAndSend()` on the returned object.
   * Transfer `amount` of `token` out to `receiver`. CONTROLLER-gated.
   */
  transfer_out: ({caller, token, receiver, amount}: {caller: string, token: string, receiver: string, amount: i128}, options?: MethodOptions) => Promise<AssembledTransaction<null>>

  /**
   * Construct and simulate a record_transfer_in transaction. Returns an `AssembledTransaction` object which will have a `result` field containing the result of the simulation. If this transaction changes contract state, you will need to call `signAndSend()` on the returned object.
   * Snapshot the balance of `token` in this vault and return the received delta.
   * 
   * # Balance invariant (issue #47)
   * 
   * The returned delta is `current_on_chain_balance − last_recorded_balance`.
   * A positive delta means tokens arrived since the last snapshot; the caller
   * (order_handler.create_order) treats this as the collateral amount and
   * reverts the transaction if the delta is ≤ 0.
   * 
   * After every transfer-out the vault re-snapshots in the same call, so the
   * recorded balance always equals the actual on-chain balance.  This prevents
   * double-counting: a second `record_transfer_in` before any new deposit
   * returns 0, which order_handler will reject.
   */
  record_transfer_in: ({caller, token}: {caller: string, token: string}, options?: MethodOptions) => Promise<AssembledTransaction<i128>>

  /**
   * Construct and simulate a get_recorded_balance transaction. Returns an `AssembledTransaction` object which will have a `result` field containing the result of the simulation. If this transaction changes contract state, you will need to call `signAndSend()` on the returned object.
   * Return the last recorded balance for a token.
   */
  get_recorded_balance: ({token}: {token: string}, options?: MethodOptions) => Promise<AssembledTransaction<i128>>

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
      new ContractSpec([ "AAAABAAAAAAAAAAAAAAABUVycm9yAAAAAAAABAAAAAAAAAASQWxyZWFkeUluaXRpYWxpemVkAAAAAAABAAAAAAAAAA5Ob3RJbml0aWFsaXplZAAAAAAAAgAAAAAAAAAMVW5hdXRob3JpemVkAAAAAwAAAAAAAAANSW52YWxpZEFtb3VudAAAAAAAAAQ=",
        "AAAAAAAAALJPbmUtdGltZSBzZXR1cDogc3RvcmUgYWRtaW4gYW5kIHJvbGVfc3RvcmUgYWRkcmVzc2VzIChpc3N1ZSAjNzgxIOKAlAphZG1pbiB3YXMgcHJldmlvdXNseSBhdXRoZW50aWNhdGVkIGJ1dCBuZXZlciBwZXJzaXN0ZWQsIGRlc3BpdGUgdGhpcwpkb2MgY29tbWVudCBhbHJlYWR5IGNsYWltaW5nIG90aGVyd2lzZSkuAAAAAAAKaW5pdGlhbGl6ZQAAAAAAAgAAAAAAAAAFYWRtaW4AAAAAAAATAAAAAAAAAApyb2xlX3N0b3JlAAAAAAATAAAAAA==",
        "AAAAAAAAAEFUcmFuc2ZlciBgYW1vdW50YCBvZiBgdG9rZW5gIG91dCB0byBgcmVjZWl2ZXJgLiBDT05UUk9MTEVSLWdhdGVkLgAAAAAAAAx0cmFuc2Zlcl9vdXQAAAAEAAAAAAAAAAZjYWxsZXIAAAAAABMAAAAAAAAABXRva2VuAAAAAAAAEwAAAAAAAAAIcmVjZWl2ZXIAAAATAAAAAAAAAAZhbW91bnQAAAAAAAsAAAAA",
        "AAAAAAAAAoBTbmFwc2hvdCB0aGUgYmFsYW5jZSBvZiBgdG9rZW5gIGluIHRoaXMgdmF1bHQgYW5kIHJldHVybiB0aGUgcmVjZWl2ZWQgZGVsdGEuCgojIEJhbGFuY2UgaW52YXJpYW50IChpc3N1ZSAjNDcpCgpUaGUgcmV0dXJuZWQgZGVsdGEgaXMgYGN1cnJlbnRfb25fY2hhaW5fYmFsYW5jZSDiiJIgbGFzdF9yZWNvcmRlZF9iYWxhbmNlYC4KQSBwb3NpdGl2ZSBkZWx0YSBtZWFucyB0b2tlbnMgYXJyaXZlZCBzaW5jZSB0aGUgbGFzdCBzbmFwc2hvdDsgdGhlIGNhbGxlcgoob3JkZXJfaGFuZGxlci5jcmVhdGVfb3JkZXIpIHRyZWF0cyB0aGlzIGFzIHRoZSBjb2xsYXRlcmFsIGFtb3VudCBhbmQKcmV2ZXJ0cyB0aGUgdHJhbnNhY3Rpb24gaWYgdGhlIGRlbHRhIGlzIOKJpCAwLgoKQWZ0ZXIgZXZlcnkgdHJhbnNmZXItb3V0IHRoZSB2YXVsdCByZS1zbmFwc2hvdHMgaW4gdGhlIHNhbWUgY2FsbCwgc28gdGhlCnJlY29yZGVkIGJhbGFuY2UgYWx3YXlzIGVxdWFscyB0aGUgYWN0dWFsIG9uLWNoYWluIGJhbGFuY2UuICBUaGlzIHByZXZlbnRzCmRvdWJsZS1jb3VudGluZzogYSBzZWNvbmQgYHJlY29yZF90cmFuc2Zlcl9pbmAgYmVmb3JlIGFueSBuZXcgZGVwb3NpdApyZXR1cm5zIDAsIHdoaWNoIG9yZGVyX2hhbmRsZXIgd2lsbCByZWplY3QuAAAAEnJlY29yZF90cmFuc2Zlcl9pbgAAAAAAAgAAAAAAAAAGY2FsbGVyAAAAAAATAAAAAAAAAAV0b2tlbgAAAAAAABMAAAABAAAACw==",
        "AAAAAAAAAC1SZXR1cm4gdGhlIGxhc3QgcmVjb3JkZWQgYmFsYW5jZSBmb3IgYSB0b2tlbi4AAAAAAAAUZ2V0X3JlY29yZGVkX2JhbGFuY2UAAAABAAAAAAAAAAV0b2tlbgAAAAAAABMAAAABAAAACw==" ]),
      options
    )
  }
  public readonly fromJSON = {
    initialize: this.txFromJSON<null>,
        transfer_out: this.txFromJSON<null>,
        record_transfer_in: this.txFromJSON<i128>,
        get_recorded_balance: this.txFromJSON<i128>
  }
}