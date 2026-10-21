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
  1: {message:"AllocationTooHigh"},
  2: {message:"MissingInsuranceFund"},
  3: {message:"MissingMarketPool"},
  4: {message:"MissingTreasury"},
  5: {message:"Unauthorized"}
}


export interface PenaltySplit {
  insurance_share: u128;
  treasury_share: u128;
}


export interface ShortfallCoverage {
  covered_by_fund: u128;
  pool_remainder: u128;
}







export interface Client {
  /**
   * Construct and simulate a cover_shortfall transaction. Returns an `AssembledTransaction` object which will have a `result` field containing the result of the simulation. If this transaction changes contract state, you will need to call `signAndSend()` on the returned object.
   */
  cover_shortfall: ({data_store, role_store, caller, market, token, shortfall_amount}: {data_store: string, role_store: string, caller: string, market: string, token: string, shortfall_amount: u128}, options?: MethodOptions) => Promise<AssembledTransaction<ShortfallCoverage>>

  /**
   * Construct and simulate a configure_treasury transaction. Returns an `AssembledTransaction` object which will have a `result` field containing the result of the simulation. If this transaction changes contract state, you will need to call `signAndSend()` on the returned object.
   * Register the protocol treasury address. `route_liquidation_penalty` transfers
   * exclusively to this address rather than a caller-supplied one.
   */
  configure_treasury: ({data_store, caller, treasury}: {data_store: string, caller: string, treasury: string}, options?: MethodOptions) => Promise<AssembledTransaction<null>>

  /**
   * Construct and simulate a configure_market_pool transaction. Returns an `AssembledTransaction` object which will have a `result` field containing the result of the simulation. If this transaction changes contract state, you will need to call `signAndSend()` on the returned object.
   * Register the market's actual pool/vault address. `cover_shortfall` transfers
   * exclusively to this address rather than a caller-supplied one.
   */
  configure_market_pool: ({data_store, caller, market, pool}: {data_store: string, caller: string, market: string, pool: string}, options?: MethodOptions) => Promise<AssembledTransaction<null>>

  /**
   * Construct and simulate a preview_penalty_split transaction. Returns an `AssembledTransaction` object which will have a `result` field containing the result of the simulation. If this transaction changes contract state, you will need to call `signAndSend()` on the returned object.
   */
  preview_penalty_split: ({data_store, market, liquidation_penalty}: {data_store: string, market: string, liquidation_penalty: u128}, options?: MethodOptions) => Promise<AssembledTransaction<PenaltySplit>>

  /**
   * Construct and simulate a configure_insurance_fund transaction. Returns an `AssembledTransaction` object which will have a `result` field containing the result of the simulation. If this transaction changes contract state, you will need to call `signAndSend()` on the returned object.
   */
  configure_insurance_fund: ({data_store, role_store, caller, market, fund, allocation_bps}: {data_store: string, role_store: string, caller: string, market: string, fund: string, allocation_bps: u32}, options?: MethodOptions) => Promise<AssembledTransaction<null>>

  /**
   * Construct and simulate a route_liquidation_penalty transaction. Returns an `AssembledTransaction` object which will have a `result` field containing the result of the simulation. If this transaction changes contract state, you will need to call `signAndSend()` on the returned object.
   */
  route_liquidation_penalty: ({data_store, role_store, market, token, source, liquidation_penalty}: {data_store: string, role_store: string, market: string, token: string, source: string, liquidation_penalty: u128}, options?: MethodOptions) => Promise<AssembledTransaction<PenaltySplit>>

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
      new ContractSpec([ "AAAABAAAAAAAAAAAAAAABUVycm9yAAAAAAAABQAAAAAAAAARQWxsb2NhdGlvblRvb0hpZ2gAAAAAAAABAAAAAAAAABRNaXNzaW5nSW5zdXJhbmNlRnVuZAAAAAIAAAAAAAAAEU1pc3NpbmdNYXJrZXRQb29sAAAAAAAAAwAAAAAAAAAPTWlzc2luZ1RyZWFzdXJ5AAAAAAQAAAAAAAAADFVuYXV0aG9yaXplZAAAAAU=",
        "AAAAAQAAAAAAAAAAAAAADFBlbmFsdHlTcGxpdAAAAAIAAAAAAAAAD2luc3VyYW5jZV9zaGFyZQAAAAAKAAAAAAAAAA50cmVhc3VyeV9zaGFyZQAAAAAACg==",
        "AAAAAQAAAAAAAAAAAAAAEVNob3J0ZmFsbENvdmVyYWdlAAAAAAAAAgAAAAAAAAAPY292ZXJlZF9ieV9mdW5kAAAAAAoAAAAAAAAADnBvb2xfcmVtYWluZGVyAAAAAAAK",
        "AAAABQAAAAAAAAAAAAAAElRyZWFzdXJ5Q29uZmlndXJlZAAAAAAAAQAAAAhpZl90cmVhcwAAAAEAAAAAAAAACHRyZWFzdXJ5AAAAEwAAAAAAAAAC",
        "AAAABQAAAgdJc3N1ZSAjNTgxOiByb3V0ZV9saXF1aWRhdGlvbl9wZW5hbHR5J3MgdHJlYXN1cnktc2hhcmUgdHJhbnNmZXIgaGFkIG5vIGV2ZW50IG9mCml0cyBvd24g4oCUIGl0IHdhcyBvbmx5IHJlcG9ydGVkIGFzIGEgZmllbGQgb24gYEluc3VyYW5jZVBlbmFsdHlSb3V0ZWRgLCB3aGljaCBpcwpwdWJsaXNoZWQgZXhjbHVzaXZlbHkgZnJvbSB0aGUgYGluc3VyYW5jZV9zaGFyZSA+IDBgIGJyYW5jaC4gV2hlbiBhIG1hcmtldCdzCmBhbGxvY2F0aW9uX2Jwc2AgaXMgMCAob3Igb3RoZXJ3aXNlIHJvdXRlcyBub3RoaW5nIHRvIHRoZSBpbnN1cmFuY2UgZnVuZCksIHRoZQp0cmVhc3VyeSBzdGlsbCByZWNlaXZlcyBhIHJlYWwgdG9rZW4gdHJhbnNmZXIgd2l0aCBubyBjb3JyZXNwb25kaW5nIGV2ZW50LgpUaGlzIGV2ZW50IGlzIHB1Ymxpc2hlZCB1bmNvbmRpdGlvbmFsbHkgd2hlbmV2ZXIgYHRyZWFzdXJ5X3NoYXJlID4gMGAsIGluZGVwZW5kZW50Cm9mIHdoZXRoZXIgdGhlIGluc3VyYW5jZS1mdW5kIGxlZyBhbHNvIHJhbi4AAAAAAAAAABNUcmVhc3VyeVNoYXJlUm91dGVkAAAAAAEAAAAHaWZfdHBlbgAAAAAEAAAAAAAAAAZtYXJrZXQAAAAAABMAAAAAAAAAAAAAAAV0b2tlbgAAAAAAABMAAAAAAAAAAAAAAAh0cmVhc3VyeQAAABMAAAAAAAAAAAAAAA50cmVhc3VyeV9zaGFyZQAAAAAACgAAAAAAAAAC",
        "AAAABQAAAAAAAAAAAAAAFE1hcmtldFBvb2xDb25maWd1cmVkAAAAAQAAAAdpZl9wb29sAAAAAAIAAAAAAAAABm1hcmtldAAAAAAAEwAAAAAAAAAAAAAABHBvb2wAAAATAAAAAAAAAAI=",
        "AAAABQAAAAAAAAAAAAAAFkluc3VyYW5jZVBlbmFsdHlSb3V0ZWQAAAAAAAEAAAAGaWZfcGVuAAAAAAAFAAAAAAAAAAZtYXJrZXQAAAAAABMAAAAAAAAAAAAAAAV0b2tlbgAAAAAAABMAAAAAAAAAAAAAAARmdW5kAAAAEwAAAAAAAAAAAAAAD2luc3VyYW5jZV9zaGFyZQAAAAAKAAAAAAAAAAAAAAAOdHJlYXN1cnlfc2hhcmUAAAAAAAoAAAAAAAAAAg==",
        "AAAABQAAAAAAAAAAAAAAF0luc3VyYW5jZUZ1bmRDb25maWd1cmVkAAAAAAEAAAAGaWZfY2ZnAAAAAAADAAAAAAAAAAZtYXJrZXQAAAAAABMAAAAAAAAAAAAAAARmdW5kAAAAEwAAAAAAAAAAAAAADmFsbG9jYXRpb25fYnBzAAAAAAAEAAAAAAAAAAI=",
        "AAAABQAAAAAAAAAAAAAAGUluc3VyYW5jZVNob3J0ZmFsbENvdmVyZWQAAAAAAAABAAAAB2lmX2RyYXcAAAAABgAAAAAAAAAGbWFya2V0AAAAAAATAAAAAAAAAAAAAAAFdG9rZW4AAAAAAAATAAAAAAAAAAAAAAAEZnVuZAAAABMAAAAAAAAAAAAAABNyZXF1ZXN0ZWRfc2hvcnRmYWxsAAAAAAoAAAAAAAAAAAAAAA9jb3ZlcmVkX2J5X2Z1bmQAAAAACgAAAAAAAAAAAAAADnBvb2xfcmVtYWluZGVyAAAAAAAKAAAAAAAAAAI=",
        "AAAAAAAAAAAAAAAPY292ZXJfc2hvcnRmYWxsAAAAAAYAAAAAAAAACmRhdGFfc3RvcmUAAAAAABMAAAAAAAAACnJvbGVfc3RvcmUAAAAAABMAAAAAAAAABmNhbGxlcgAAAAAAEwAAAAAAAAAGbWFya2V0AAAAAAATAAAAAAAAAAV0b2tlbgAAAAAAABMAAAAAAAAAEHNob3J0ZmFsbF9hbW91bnQAAAAKAAAAAQAAB9AAAAARU2hvcnRmYWxsQ292ZXJhZ2UAAAA=",
        "AAAAAAAAAIxSZWdpc3RlciB0aGUgcHJvdG9jb2wgdHJlYXN1cnkgYWRkcmVzcy4gYHJvdXRlX2xpcXVpZGF0aW9uX3BlbmFsdHlgIHRyYW5zZmVycwpleGNsdXNpdmVseSB0byB0aGlzIGFkZHJlc3MgcmF0aGVyIHRoYW4gYSBjYWxsZXItc3VwcGxpZWQgb25lLgAAABJjb25maWd1cmVfdHJlYXN1cnkAAAAAAAMAAAAAAAAACmRhdGFfc3RvcmUAAAAAABMAAAAAAAAABmNhbGxlcgAAAAAAEwAAAAAAAAAIdHJlYXN1cnkAAAATAAAAAA==",
        "AAAAAAAAAItSZWdpc3RlciB0aGUgbWFya2V0J3MgYWN0dWFsIHBvb2wvdmF1bHQgYWRkcmVzcy4gYGNvdmVyX3Nob3J0ZmFsbGAgdHJhbnNmZXJzCmV4Y2x1c2l2ZWx5IHRvIHRoaXMgYWRkcmVzcyByYXRoZXIgdGhhbiBhIGNhbGxlci1zdXBwbGllZCBvbmUuAAAAABVjb25maWd1cmVfbWFya2V0X3Bvb2wAAAAAAAAEAAAAAAAAAApkYXRhX3N0b3JlAAAAAAATAAAAAAAAAAZjYWxsZXIAAAAAABMAAAAAAAAABm1hcmtldAAAAAAAEwAAAAAAAAAEcG9vbAAAABMAAAAA",
        "AAAAAAAAAAAAAAAVcHJldmlld19wZW5hbHR5X3NwbGl0AAAAAAAAAwAAAAAAAAAKZGF0YV9zdG9yZQAAAAAAEwAAAAAAAAAGbWFya2V0AAAAAAATAAAAAAAAABNsaXF1aWRhdGlvbl9wZW5hbHR5AAAAAAoAAAABAAAH0AAAAAxQZW5hbHR5U3BsaXQ=",
        "AAAAAAAAAAAAAAAYY29uZmlndXJlX2luc3VyYW5jZV9mdW5kAAAABgAAAAAAAAAKZGF0YV9zdG9yZQAAAAAAEwAAAAAAAAAKcm9sZV9zdG9yZQAAAAAAEwAAAAAAAAAGY2FsbGVyAAAAAAATAAAAAAAAAAZtYXJrZXQAAAAAABMAAAAAAAAABGZ1bmQAAAATAAAAAAAAAA5hbGxvY2F0aW9uX2JwcwAAAAAABAAAAAA=",
        "AAAAAAAAAAAAAAAZcm91dGVfbGlxdWlkYXRpb25fcGVuYWx0eQAAAAAAAAYAAAAAAAAACmRhdGFfc3RvcmUAAAAAABMAAAAAAAAACnJvbGVfc3RvcmUAAAAAABMAAAAAAAAABm1hcmtldAAAAAAAEwAAAAAAAAAFdG9rZW4AAAAAAAATAAAAAAAAAAZzb3VyY2UAAAAAABMAAAAAAAAAE2xpcXVpZGF0aW9uX3BlbmFsdHkAAAAACgAAAAEAAAfQAAAADFBlbmFsdHlTcGxpdA==" ]),
      options
    )
  }
  public readonly fromJSON = {
    cover_shortfall: this.txFromJSON<ShortfallCoverage>,
        configure_treasury: this.txFromJSON<null>,
        configure_market_pool: this.txFromJSON<null>,
        preview_penalty_split: this.txFromJSON<PenaltySplit>,
        configure_insurance_fund: this.txFromJSON<null>,
        route_liquidation_penalty: this.txFromJSON<PenaltySplit>
  }
}