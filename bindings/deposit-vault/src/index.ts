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
   * admin was previously authenticated but never persisted).
   */
  initialize: ({admin, role_store}: {admin: string, role_store: string}, options?: MethodOptions) => Promise<AssembledTransaction<null>>

  /**
   * Construct and simulate a transfer_out transaction. Returns an `AssembledTransaction` object which will have a `result` field containing the result of the simulation. If this transaction changes contract state, you will need to call `signAndSend()` on the returned object.
   * Transfer `amount` of `token` from this vault to `receiver`.
   * Callable by **any** CONTROLLER holder, not just deposit_handler —
   * every holder can move vault funds (see module docs, issue #718).
   */
  transfer_out: ({caller, token, receiver, amount}: {caller: string, token: string, receiver: string, amount: i128}, options?: MethodOptions) => Promise<AssembledTransaction<null>>

  /**
   * Construct and simulate a record_transfer_in transaction. Returns an `AssembledTransaction` object which will have a `result` field containing the result of the simulation. If this transaction changes contract state, you will need to call `signAndSend()` on the returned object.
   * Snapshot the balance of `token` in this vault.
   * Returns the amount received since the last snapshot (delta).
   * Called by deposit_handler right after the user's transfer lands.
   * Callable by **any** CONTROLLER holder, not just deposit_handler
   * (see module docs, issue #718).
   */
  record_transfer_in: ({caller, token}: {caller: string, token: string}, options?: MethodOptions) => Promise<AssembledTransaction<i128>>

  /**
   * Construct and simulate a get_recorded_balance transaction. Returns an `AssembledTransaction` object which will have a `result` field containing the result of the simulation. If this transaction changes contract state, you will need to call `signAndSend()` on the returned object.
   * Read the last recorded balance for a token (for diagnostics).
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
        "AAAAAAAAAH1PbmUtdGltZSBzZXR1cDogc3RvcmUgYWRtaW4gYW5kIHJvbGVfc3RvcmUgYWRkcmVzc2VzIChpc3N1ZSAjNzgxIOKAlAphZG1pbiB3YXMgcHJldmlvdXNseSBhdXRoZW50aWNhdGVkIGJ1dCBuZXZlciBwZXJzaXN0ZWQpLgAAAAAAAAppbml0aWFsaXplAAAAAAACAAAAAAAAAAVhZG1pbgAAAAAAABMAAAAAAAAACnJvbGVfc3RvcmUAAAAAABMAAAAA",
        "AAAAAAAAAMBUcmFuc2ZlciBgYW1vdW50YCBvZiBgdG9rZW5gIGZyb20gdGhpcyB2YXVsdCB0byBgcmVjZWl2ZXJgLgpDYWxsYWJsZSBieSAqKmFueSoqIENPTlRST0xMRVIgaG9sZGVyLCBub3QganVzdCBkZXBvc2l0X2hhbmRsZXIg4oCUCmV2ZXJ5IGhvbGRlciBjYW4gbW92ZSB2YXVsdCBmdW5kcyAoc2VlIG1vZHVsZSBkb2NzLCBpc3N1ZSAjNzE4KS4AAAAMdHJhbnNmZXJfb3V0AAAABAAAAAAAAAAGY2FsbGVyAAAAAAATAAAAAAAAAAV0b2tlbgAAAAAAABMAAAAAAAAACHJlY2VpdmVyAAAAEwAAAAAAAAAGYW1vdW50AAAAAAALAAAAAA==",
        "AAAAAAAAAQtTbmFwc2hvdCB0aGUgYmFsYW5jZSBvZiBgdG9rZW5gIGluIHRoaXMgdmF1bHQuClJldHVybnMgdGhlIGFtb3VudCByZWNlaXZlZCBzaW5jZSB0aGUgbGFzdCBzbmFwc2hvdCAoZGVsdGEpLgpDYWxsZWQgYnkgZGVwb3NpdF9oYW5kbGVyIHJpZ2h0IGFmdGVyIHRoZSB1c2VyJ3MgdHJhbnNmZXIgbGFuZHMuCkNhbGxhYmxlIGJ5ICoqYW55KiogQ09OVFJPTExFUiBob2xkZXIsIG5vdCBqdXN0IGRlcG9zaXRfaGFuZGxlcgooc2VlIG1vZHVsZSBkb2NzLCBpc3N1ZSAjNzE4KS4AAAAAEnJlY29yZF90cmFuc2Zlcl9pbgAAAAAAAgAAAAAAAAAGY2FsbGVyAAAAAAATAAAAAAAAAAV0b2tlbgAAAAAAABMAAAABAAAACw==",
        "AAAAAAAAAD1SZWFkIHRoZSBsYXN0IHJlY29yZGVkIGJhbGFuY2UgZm9yIGEgdG9rZW4gKGZvciBkaWFnbm9zdGljcykuAAAAAAAAFGdldF9yZWNvcmRlZF9iYWxhbmNlAAAAAQAAAAAAAAAFdG9rZW4AAAAAAAATAAAAAQAAAAs=" ]),
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