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
  4: {message:"LastAdmin"}
}




export interface Client {
  /**
   * Construct and simulate a has_role transaction. Returns an `AssembledTransaction` object which will have a `result` field containing the result of the simulation. If this transaction changes contract state, you will need to call `signAndSend()` on the returned object.
   * Returns true if `account` currently holds `role`.
   */
  has_role: ({account, role}: {account: string, role: Buffer}, options?: MethodOptions) => Promise<AssembledTransaction<boolean>>

  /**
   * Construct and simulate a get_roles transaction. Returns an `AssembledTransaction` object which will have a `result` field containing the result of the simulation. If this transaction changes contract state, you will need to call `signAndSend()` on the returned object.
   * All roles currently held by `account`.
   */
  get_roles: ({account}: {account: string}, options?: MethodOptions) => Promise<AssembledTransaction<Array<Buffer>>>

  /**
   * Construct and simulate a grant_role transaction. Returns an `AssembledTransaction` object which will have a `result` field containing the result of the simulation. If this transaction changes contract state, you will need to call `signAndSend()` on the returned object.
   * Grant `role` to `account`. Caller must hold ROLE_ADMIN.
   */
  grant_role: ({caller, account, role}: {caller: string, account: string, role: Buffer}, options?: MethodOptions) => Promise<AssembledTransaction<null>>

  /**
   * Construct and simulate a initialize transaction. Returns an `AssembledTransaction` object which will have a `result` field containing the result of the simulation. If this transaction changes contract state, you will need to call `signAndSend()` on the returned object.
   * Deploy-time init: grant ROLE_ADMIN to `admin`. Can only be called once.
   */
  initialize: ({admin}: {admin: string}, options?: MethodOptions) => Promise<AssembledTransaction<null>>

  /**
   * Construct and simulate a revoke_role transaction. Returns an `AssembledTransaction` object which will have a `result` field containing the result of the simulation. If this transaction changes contract state, you will need to call `signAndSend()` on the returned object.
   * Revoke `role` from `account`. Caller must hold ROLE_ADMIN.
   * Prevents removing the last ROLE_ADMIN holder.
   */
  revoke_role: ({caller, account, role}: {caller: string, account: string, role: Buffer}, options?: MethodOptions) => Promise<AssembledTransaction<null>>

  /**
   * Construct and simulate a get_all_roles transaction. Returns an `AssembledTransaction` object which will have a `result` field containing the result of the simulation. If this transaction changes contract state, you will need to call `signAndSend()` on the returned object.
   * All role IDs that have ever been granted.
   */
  get_all_roles: (options?: MethodOptions) => Promise<AssembledTransaction<Array<Buffer>>>

  /**
   * Construct and simulate a get_role_count transaction. Returns an `AssembledTransaction` object which will have a `result` field containing the result of the simulation. If this transaction changes contract state, you will need to call `signAndSend()` on the returned object.
   * Count of distinct roles.
   */
  get_role_count: (options?: MethodOptions) => Promise<AssembledTransaction<u32>>

  /**
   * Construct and simulate a get_role_members transaction. Returns an `AssembledTransaction` object which will have a `result` field containing the result of the simulation. If this transaction changes contract state, you will need to call `signAndSend()` on the returned object.
   * Paginated list of all accounts that hold `role`.
   */
  get_role_members: ({role, start, end}: {role: Buffer, start: u32, end: u32}, options?: MethodOptions) => Promise<AssembledTransaction<Array<string>>>

  /**
   * Construct and simulate a get_role_member_count transaction. Returns an `AssembledTransaction` object which will have a `result` field containing the result of the simulation. If this transaction changes contract state, you will need to call `signAndSend()` on the returned object.
   * Count of accounts holding `role`.
   */
  get_role_member_count: ({role}: {role: Buffer}, options?: MethodOptions) => Promise<AssembledTransaction<u32>>

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
      new ContractSpec([ "AAAABAAAAAAAAAAAAAAABUVycm9yAAAAAAAABAAAAAAAAAAOTm90SW5pdGlhbGl6ZWQAAAAAAAEAAAAAAAAAEkFscmVhZHlJbml0aWFsaXplZAAAAAAAAgAAAAAAAAAMVW5hdXRob3JpemVkAAAAAwAAAAAAAAAJTGFzdEFkbWluAAAAAAAABA==",
        "AAAABQAAAAAAAAAAAAAAC1JvbGVHcmFudGVkAAAAAAEAAAAFZ3JhbnQAAAAAAAADAAAAAAAAAAZjYWxsZXIAAAAAABMAAAAAAAAAAAAAAAdhY2NvdW50AAAAABMAAAAAAAAAAAAAAARyb2xlAAAD7gAAACAAAAAAAAAAAg==",
        "AAAABQAAAAAAAAAAAAAAC1JvbGVSZXZva2VkAAAAAAEAAAAGcmV2b2tlAAAAAAADAAAAAAAAAAZjYWxsZXIAAAAAABMAAAAAAAAAAAAAAAdhY2NvdW50AAAAABMAAAAAAAAAAAAAAARyb2xlAAAD7gAAACAAAAAAAAAAAg==",
        "AAAABQAAAAAAAAAAAAAAFFJvbGVTdG9yZUluaXRpYWxpemVkAAAAAQAAAARpbml0AAAAAgAAAAAAAAAFYWRtaW4AAAAAAAATAAAAAAAAAAAAAAAKYWRtaW5fcm9sZQAAAAAD7gAAACAAAAAAAAAAAg==",
        "AAAAAAAAADFSZXR1cm5zIHRydWUgaWYgYGFjY291bnRgIGN1cnJlbnRseSBob2xkcyBgcm9sZWAuAAAAAAAACGhhc19yb2xlAAAAAgAAAAAAAAAHYWNjb3VudAAAAAATAAAAAAAAAARyb2xlAAAD7gAAACAAAAABAAAAAQ==",
        "AAAAAAAAACZBbGwgcm9sZXMgY3VycmVudGx5IGhlbGQgYnkgYGFjY291bnRgLgAAAAAACWdldF9yb2xlcwAAAAAAAAEAAAAAAAAAB2FjY291bnQAAAAAEwAAAAEAAAPqAAAD7gAAACA=",
        "AAAAAAAAADdHcmFudCBgcm9sZWAgdG8gYGFjY291bnRgLiBDYWxsZXIgbXVzdCBob2xkIFJPTEVfQURNSU4uAAAAAApncmFudF9yb2xlAAAAAAADAAAAAAAAAAZjYWxsZXIAAAAAABMAAAAAAAAAB2FjY291bnQAAAAAEwAAAAAAAAAEcm9sZQAAA+4AAAAgAAAAAA==",
        "AAAAAAAAAEdEZXBsb3ktdGltZSBpbml0OiBncmFudCBST0xFX0FETUlOIHRvIGBhZG1pbmAuIENhbiBvbmx5IGJlIGNhbGxlZCBvbmNlLgAAAAAKaW5pdGlhbGl6ZQAAAAAAAQAAAAAAAAAFYWRtaW4AAAAAAAATAAAAAA==",
        "AAAAAAAAAGhSZXZva2UgYHJvbGVgIGZyb20gYGFjY291bnRgLiBDYWxsZXIgbXVzdCBob2xkIFJPTEVfQURNSU4uClByZXZlbnRzIHJlbW92aW5nIHRoZSBsYXN0IFJPTEVfQURNSU4gaG9sZGVyLgAAAAtyZXZva2Vfcm9sZQAAAAADAAAAAAAAAAZjYWxsZXIAAAAAABMAAAAAAAAAB2FjY291bnQAAAAAEwAAAAAAAAAEcm9sZQAAA+4AAAAgAAAAAA==",
        "AAAAAAAAAClBbGwgcm9sZSBJRHMgdGhhdCBoYXZlIGV2ZXIgYmVlbiBncmFudGVkLgAAAAAAAA1nZXRfYWxsX3JvbGVzAAAAAAAAAAAAAAEAAAPqAAAD7gAAACA=",
        "AAAAAAAAABhDb3VudCBvZiBkaXN0aW5jdCByb2xlcy4AAAAOZ2V0X3JvbGVfY291bnQAAAAAAAAAAAABAAAABA==",
        "AAAAAAAAADBQYWdpbmF0ZWQgbGlzdCBvZiBhbGwgYWNjb3VudHMgdGhhdCBob2xkIGByb2xlYC4AAAAQZ2V0X3JvbGVfbWVtYmVycwAAAAMAAAAAAAAABHJvbGUAAAPuAAAAIAAAAAAAAAAFc3RhcnQAAAAAAAAEAAAAAAAAAANlbmQAAAAABAAAAAEAAAPqAAAAEw==",
        "AAAAAAAAACFDb3VudCBvZiBhY2NvdW50cyBob2xkaW5nIGByb2xlYC4AAAAAAAAVZ2V0X3JvbGVfbWVtYmVyX2NvdW50AAAAAAAAAQAAAAAAAAAEcm9sZQAAA+4AAAAgAAAAAQAAAAQ=" ]),
      options
    )
  }
  public readonly fromJSON = {
    has_role: this.txFromJSON<boolean>,
        get_roles: this.txFromJSON<Array<Buffer>>,
        grant_role: this.txFromJSON<null>,
        initialize: this.txFromJSON<null>,
        revoke_role: this.txFromJSON<null>,
        get_all_roles: this.txFromJSON<Array<Buffer>>,
        get_role_count: this.txFromJSON<u32>,
        get_role_members: this.txFromJSON<Array<string>>,
        get_role_member_count: this.txFromJSON<u32>
  }
}