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
  4: {message:"TokenNotEnabled"},
  5: {message:"InvalidAmount"},
  6: {message:"ClaimTooSoon"},
  7: {message:"MainnetNotAllowed"},
  /**
   * claim_many called with more than MAX_CLAIM_BATCH_SIZE tokens (issue #609).
   */
  8: {message:"BatchSizeLimitExceeded"}
}

export interface Client {
  /**
   * Construct and simulate a admin transaction. Returns an `AssembledTransaction` object which will have a `result` field containing the result of the simulation. If this transaction changes contract state, you will need to call `signAndSend()` on the returned object.
   */
  admin: (options?: MethodOptions) => Promise<AssembledTransaction<string>>

  /**
   * Construct and simulate a claim transaction. Returns an `AssembledTransaction` object which will have a `result` field containing the result of the simulation. If this transaction changes contract state, you will need to call `signAndSend()` on the returned object.
   */
  claim: ({account, token}: {account: string, token: string}, options?: MethodOptions) => Promise<AssembledTransaction<i128>>

  /**
   * Construct and simulate a set_token transaction. Returns an `AssembledTransaction` object which will have a `result` field containing the result of the simulation. If this transaction changes contract state, you will need to call `signAndSend()` on the returned object.
   */
  set_token: ({caller, token, claim_amount}: {caller: string, token: string, claim_amount: i128}, options?: MethodOptions) => Promise<AssembledTransaction<null>>

  /**
   * Construct and simulate a claim_many transaction. Returns an `AssembledTransaction` object which will have a `result` field containing the result of the simulation. If this transaction changes contract state, you will need to call `signAndSend()` on the returned object.
   * Claim multiple tokens in a single transaction.
   * 
   * Authorizes `account` once for the whole call rather than once per token
   * (issue #399) — looping `Self::claim` per token previously called
   * `account.require_auth()` once per iteration within the same invocation,
   * which hit a Soroban auth-reuse bug and failed for real signed transactions.
   */
  claim_many: ({account, tokens}: {account: string, tokens: Array<string>}, options?: MethodOptions) => Promise<AssembledTransaction<Array<i128>>>

  /**
   * Construct and simulate a initialize transaction. Returns an `AssembledTransaction` object which will have a `result` field containing the result of the simulation. If this transaction changes contract state, you will need to call `signAndSend()` on the returned object.
   */
  initialize: ({admin, cooldown_ledgers}: {admin: string, cooldown_ledgers: u32}, options?: MethodOptions) => Promise<AssembledTransaction<null>>

  /**
   * Construct and simulate a claim_amount transaction. Returns an `AssembledTransaction` object which will have a `result` field containing the result of the simulation. If this transaction changes contract state, you will need to call `signAndSend()` on the returned object.
   */
  claim_amount: ({token}: {token: string}, options?: MethodOptions) => Promise<AssembledTransaction<i128>>

  /**
   * Construct and simulate a remove_token transaction. Returns an `AssembledTransaction` object which will have a `result` field containing the result of the simulation. If this transaction changes contract state, you will need to call `signAndSend()` on the returned object.
   */
  remove_token: ({caller, token}: {caller: string, token: string}, options?: MethodOptions) => Promise<AssembledTransaction<null>>

  /**
   * Construct and simulate a set_cooldown transaction. Returns an `AssembledTransaction` object which will have a `result` field containing the result of the simulation. If this transaction changes contract state, you will need to call `signAndSend()` on the returned object.
   */
  set_cooldown: ({caller, cooldown_ledgers}: {caller: string, cooldown_ledgers: u32}, options?: MethodOptions) => Promise<AssembledTransaction<null>>

  /**
   * Construct and simulate a cooldown_ledgers transaction. Returns an `AssembledTransaction` object which will have a `result` field containing the result of the simulation. If this transaction changes contract state, you will need to call `signAndSend()` on the returned object.
   */
  cooldown_ledgers: (options?: MethodOptions) => Promise<AssembledTransaction<u32>>

  /**
   * Construct and simulate a last_claim_ledger transaction. Returns an `AssembledTransaction` object which will have a `result` field containing the result of the simulation. If this transaction changes contract state, you will need to call `signAndSend()` on the returned object.
   */
  last_claim_ledger: ({account, token}: {account: string, token: string}, options?: MethodOptions) => Promise<AssembledTransaction<u32>>

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
      new ContractSpec([ "AAAABAAAAAAAAAAAAAAABUVycm9yAAAAAAAACAAAAAAAAAASQWxyZWFkeUluaXRpYWxpemVkAAAAAAABAAAAAAAAAA5Ob3RJbml0aWFsaXplZAAAAAAAAgAAAAAAAAAMVW5hdXRob3JpemVkAAAAAwAAAAAAAAAPVG9rZW5Ob3RFbmFibGVkAAAAAAQAAAAAAAAADUludmFsaWRBbW91bnQAAAAAAAAFAAAAAAAAAAxDbGFpbVRvb1Nvb24AAAAGAAAAAAAAABFNYWlubmV0Tm90QWxsb3dlZAAAAAAAAAcAAABKY2xhaW1fbWFueSBjYWxsZWQgd2l0aCBtb3JlIHRoYW4gTUFYX0NMQUlNX0JBVENIX1NJWkUgdG9rZW5zIChpc3N1ZSAjNjA5KS4AAAAAABZCYXRjaFNpemVMaW1pdEV4Y2VlZGVkAAAAAAAI",
        "AAAAAAAAAAAAAAAFYWRtaW4AAAAAAAAAAAAAAQAAABM=",
        "AAAAAAAAAAAAAAAFY2xhaW0AAAAAAAACAAAAAAAAAAdhY2NvdW50AAAAABMAAAAAAAAABXRva2VuAAAAAAAAEwAAAAEAAAAL",
        "AAAAAAAAAAAAAAAJc2V0X3Rva2VuAAAAAAAAAwAAAAAAAAAGY2FsbGVyAAAAAAATAAAAAAAAAAV0b2tlbgAAAAAAABMAAAAAAAAADGNsYWltX2Ftb3VudAAAAAsAAAAA",
        "AAAAAAAAAU5DbGFpbSBtdWx0aXBsZSB0b2tlbnMgaW4gYSBzaW5nbGUgdHJhbnNhY3Rpb24uCgpBdXRob3JpemVzIGBhY2NvdW50YCBvbmNlIGZvciB0aGUgd2hvbGUgY2FsbCByYXRoZXIgdGhhbiBvbmNlIHBlciB0b2tlbgooaXNzdWUgIzM5OSkg4oCUIGxvb3BpbmcgYFNlbGY6OmNsYWltYCBwZXIgdG9rZW4gcHJldmlvdXNseSBjYWxsZWQKYGFjY291bnQucmVxdWlyZV9hdXRoKClgIG9uY2UgcGVyIGl0ZXJhdGlvbiB3aXRoaW4gdGhlIHNhbWUgaW52b2NhdGlvbiwKd2hpY2ggaGl0IGEgU29yb2JhbiBhdXRoLXJldXNlIGJ1ZyBhbmQgZmFpbGVkIGZvciByZWFsIHNpZ25lZCB0cmFuc2FjdGlvbnMuAAAAAAAKY2xhaW1fbWFueQAAAAAAAgAAAAAAAAAHYWNjb3VudAAAAAATAAAAAAAAAAZ0b2tlbnMAAAAAA+oAAAATAAAAAQAAA+oAAAAL",
        "AAAAAAAAAAAAAAAKaW5pdGlhbGl6ZQAAAAAAAgAAAAAAAAAFYWRtaW4AAAAAAAATAAAAAAAAABBjb29sZG93bl9sZWRnZXJzAAAABAAAAAA=",
        "AAAAAAAAAAAAAAAMY2xhaW1fYW1vdW50AAAAAQAAAAAAAAAFdG9rZW4AAAAAAAATAAAAAQAAAAs=",
        "AAAAAAAAAAAAAAAMcmVtb3ZlX3Rva2VuAAAAAgAAAAAAAAAGY2FsbGVyAAAAAAATAAAAAAAAAAV0b2tlbgAAAAAAABMAAAAA",
        "AAAAAAAAAAAAAAAMc2V0X2Nvb2xkb3duAAAAAgAAAAAAAAAGY2FsbGVyAAAAAAATAAAAAAAAABBjb29sZG93bl9sZWRnZXJzAAAABAAAAAA=",
        "AAAAAAAAAAAAAAAQY29vbGRvd25fbGVkZ2VycwAAAAAAAAABAAAABA==",
        "AAAAAAAAAAAAAAARbGFzdF9jbGFpbV9sZWRnZXIAAAAAAAACAAAAAAAAAAdhY2NvdW50AAAAABMAAAAAAAAABXRva2VuAAAAAAAAEwAAAAEAAAAE" ]),
      options
    )
  }
  public readonly fromJSON = {
    admin: this.txFromJSON<string>,
        claim: this.txFromJSON<i128>,
        set_token: this.txFromJSON<null>,
        claim_many: this.txFromJSON<Array<i128>>,
        initialize: this.txFromJSON<null>,
        claim_amount: this.txFromJSON<i128>,
        remove_token: this.txFromJSON<null>,
        set_cooldown: this.txFromJSON<null>,
        cooldown_ledgers: this.txFromJSON<u32>,
        last_claim_ledger: this.txFromJSON<u32>
  }
}