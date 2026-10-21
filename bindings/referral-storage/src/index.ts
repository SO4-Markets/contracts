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
  4: {message:"CodeAlreadyTaken"},
  5: {message:"CodeNotFound"},
  6: {message:"InvalidTier"},
  7: {message:"InvalidInput"},
  8: {message:"NotCodeOwner"},
  9: {message:"CodeTooLong"},
  10: {message:"InvalidCodeCharacters"},
  11: {message:"EmptyCode"},
  12: {message:"InvalidTierConfig"},
  13: {message:"SelfReferralNotAllowed"}
}


export interface TierConfig {
  discount_share_bps: u32;
  total_rebate_bps: u32;
}

export type ReferralKey = {tag: "CodeOwner", values: readonly [Buffer]} | {tag: "TraderCode", values: readonly [string]} | {tag: "ReferrerTier", values: readonly [string]} | {tag: "TierConfig", values: readonly [u32]} | {tag: "ReferrerVolume", values: readonly [string]} | {tag: "TierUpgradeThreshold", values: readonly [u32]};






export interface Client {
  /**
   * Construct and simulate a upgrade transaction. Returns an `AssembledTransaction` object which will have a `result` field containing the result of the simulation. If this transaction changes contract state, you will need to call `signAndSend()` on the returned object.
   * Upgrade the contract wasm. Only the stored admin may call this.
   */
  upgrade: ({new_wasm_hash}: {new_wasm_hash: Buffer}, options?: MethodOptions) => Promise<AssembledTransaction<null>>

  /**
   * Construct and simulate a initialize transaction. Returns an `AssembledTransaction` object which will have a `result` field containing the result of the simulation. If this transaction changes contract state, you will need to call `signAndSend()` on the returned object.
   */
  initialize: ({admin}: {admin: string}, options?: MethodOptions) => Promise<AssembledTransaction<null>>

  /**
   * Construct and simulate a renew_code transaction. Returns an `AssembledTransaction` object which will have a `result` field containing the result of the simulation. If this transaction changes contract state, you will need to call `signAndSend()` on the returned object.
   * Self-service keep-alive for a registered code (issue #445).
   * 
   * `register_code` only sets the `CodeOwner` entry's initial TTL, and that
   * TTL is otherwise only bumped as a side effect of a trader interacting
   * with the code. A code nobody is actively trading with can therefore run
   * out its TTL; once archived, `register_code`'s existence check reads as
   * "unregistered" and a different address can claim the same code string,
   * silently taking over any trader still linked to it. Calling this lets
   * the current owner extend the TTL at will, independent of trader
   * activity. Only the current owner may call it.
   */
  renew_code: ({caller, code}: {caller: string, code: Buffer}, options?: MethodOptions) => Promise<AssembledTransaction<null>>

  /**
   * Construct and simulate a register_code transaction. Returns an `AssembledTransaction` object which will have a `result` field containing the result of the simulation. If this transaction changes contract state, you will need to call `signAndSend()` on the returned object.
   * Register a new referral code; caller becomes the owner.
   * 
   * `code` must be 1–20 bytes of `[A-Za-z0-9_-]`. Reverts with:
   * - `EmptyCode` if `code` is empty
   * - `CodeTooLong` if `code.len() > MAX_REFERRAL_CODE_LENGTH`
   * - `InvalidCodeCharacters` if `code` contains disallowed bytes
   * - `CodeAlreadyTaken` if the code is already registered
   */
  register_code: ({caller, code}: {caller: string, code: Buffer}, options?: MethodOptions) => Promise<AssembledTransaction<null>>

  /**
   * Construct and simulate a get_code_owner transaction. Returns an `AssembledTransaction` object which will have a `result` field containing the result of the simulation. If this transaction changes contract state, you will need to call `signAndSend()` on the returned object.
   * Return the owner address for a given referral code, or None if unregistered.
   */
  get_code_owner: ({code}: {code: Buffer}, options?: MethodOptions) => Promise<AssembledTransaction<Option<string>>>

  /**
   * Construct and simulate a set_tier_config transaction. Returns an `AssembledTransaction` object which will have a `result` field containing the result of the simulation. If this transaction changes contract state, you will need to call `signAndSend()` on the returned object.
   * Configure the rebate/discount parameters for a tier (admin only).
   */
  set_tier_config: ({admin, tier, config}: {admin: string, tier: u32, config: TierConfig}, options?: MethodOptions) => Promise<AssembledTransaction<null>>

  /**
   * Construct and simulate a set_order_handler transaction. Returns an `AssembledTransaction` object which will have a `result` field containing the result of the simulation. If this transaction changes contract state, you will need to call `signAndSend()` on the returned object.
   * Register the authorized order_handler address (admin only).
   * Only this address may call `increment_referrer_volume`.
   */
  set_order_handler: ({admin, order_handler}: {admin: string, order_handler: string}, options?: MethodOptions) => Promise<AssembledTransaction<null>>

  /**
   * Construct and simulate a set_referrer_tier transaction. Returns an `AssembledTransaction` object which will have a `result` field containing the result of the simulation. If this transaction changes contract state, you will need to call `signAndSend()` on the returned object.
   * Set the tier for a referrer (admin only).
   */
  set_referrer_tier: ({admin, referrer, tier}: {admin: string, referrer: string, tier: u32}, options?: MethodOptions) => Promise<AssembledTransaction<null>>

  /**
   * Construct and simulate a get_trader_referrer transaction. Returns an `AssembledTransaction` object which will have a `result` field containing the result of the simulation. If this transaction changes contract state, you will need to call `signAndSend()` on the returned object.
   * Look up the referral code for a trader, and return the referrer's address.
   */
  get_trader_referrer: ({trader}: {trader: string}, options?: MethodOptions) => Promise<AssembledTransaction<Option<string>>>

  /**
   * Construct and simulate a set_referrer_volume transaction. Returns an `AssembledTransaction` object which will have a `result` field containing the result of the simulation. If this transaction changes contract state, you will need to call `signAndSend()` on the returned object.
   * Admin-only override for a referrer's stored lifetime volume (issue #444).
   * 
   * `increment_referrer_volume`'s auto-upgrade re-evaluates tier thresholds
   * purely from this value, which otherwise only ever grows. A manual
   * `set_referrer_tier` downgrade is therefore not durable on its own: the
   * very next trade reads the unchanged, still-large cumulative volume and
   * silently re-upgrades the referrer right back to (or above) the tier an
   * admin just removed. Call this alongside a manual tier change to make
   * the downgrade (or any other volume correction) stick.
   */
  set_referrer_volume: ({admin, referrer, volume_usd}: {admin: string, referrer: string, volume_usd: u128}, options?: MethodOptions) => Promise<AssembledTransaction<null>>

  /**
   * Construct and simulate a get_trader_discount_bps transaction. Returns an `AssembledTransaction` object which will have a `result` field containing the result of the simulation. If this transaction changes contract state, you will need to call `signAndSend()` on the returned object.
   * Return the fee discount bps for a trader given their referral code, or 0 if none.
   */
  get_trader_discount_bps: ({trader}: {trader: string}, options?: MethodOptions) => Promise<AssembledTransaction<u32>>

  /**
   * Construct and simulate a transfer_code_ownership transaction. Returns an `AssembledTransaction` object which will have a `result` field containing the result of the simulation. If this transaction changes contract state, you will need to call `signAndSend()` on the returned object.
   * Transfer ownership of a registered referral code to a new address.
   * 
   * Only the current code owner may call this. Requires auth from `from`.
   * The new owner (`to`) immediately becomes the code's referrer for fee calculations.
   */
  transfer_code_ownership: ({from, to, code}: {from: string, to: string, code: Buffer}, options?: MethodOptions) => Promise<AssembledTransaction<null>>

  /**
   * Construct and simulate a get_trader_referral_code transaction. Returns an `AssembledTransaction` object which will have a `result` field containing the result of the simulation. If this transaction changes contract state, you will need to call `signAndSend()` on the returned object.
   * Return the referral code for a trader, or None.
   */
  get_trader_referral_code: ({trader}: {trader: string}, options?: MethodOptions) => Promise<AssembledTransaction<Option<Buffer>>>

  /**
   * Construct and simulate a set_trader_referral_code transaction. Returns an `AssembledTransaction` object which will have a `result` field containing the result of the simulation. If this transaction changes contract state, you will need to call `signAndSend()` on the returned object.
   * Set the referral code for a trader (links them to a referrer).
   */
  set_trader_referral_code: ({trader, code}: {trader: string, code: Buffer}, options?: MethodOptions) => Promise<AssembledTransaction<null>>

  /**
   * Construct and simulate a increment_referrer_volume transaction. Returns an `AssembledTransaction` object which will have a `result` field containing the result of the simulation. If this transaction changes contract state, you will need to call `signAndSend()` on the returned object.
   * Called by the authorized order_handler after each trade settlement.
   * Increments `referrer`'s lifetime volume and auto-upgrades their tier if
   * the new cumulative total crosses any tier threshold (tier only goes up).
   */
  increment_referrer_volume: ({caller, referrer, volume_usd}: {caller: string, referrer: string, volume_usd: u128}, options?: MethodOptions) => Promise<AssembledTransaction<null>>

  /**
   * Construct and simulate a set_tier_upgrade_threshold transaction. Returns an `AssembledTransaction` object which will have a `result` field containing the result of the simulation. If this transaction changes contract state, you will need to call `signAndSend()` on the returned object.
   * Set the cumulative volume threshold that triggers an upgrade to `tier` (admin only).
   * Tiers 0–2 are valid. Tier 0 is the base and has no threshold.
   */
  set_tier_upgrade_threshold: ({admin, tier, threshold_usd}: {admin: string, tier: u32, threshold_usd: u128}, options?: MethodOptions) => Promise<AssembledTransaction<null>>

  /**
   * Construct and simulate a get_referrer_cumulative_volume transaction. Returns an `AssembledTransaction` object which will have a `result` field containing the result of the simulation. If this transaction changes contract state, you will need to call `signAndSend()` on the returned object.
   * Return the lifetime cumulative trading volume for a referrer, in USD (FLOAT_PRECISION).
   */
  get_referrer_cumulative_volume: ({referrer}: {referrer: string}, options?: MethodOptions) => Promise<AssembledTransaction<u128>>

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
      new ContractSpec([ "AAAABAAAAAAAAAAAAAAABUVycm9yAAAAAAAADQAAAAAAAAAOTm90SW5pdGlhbGl6ZWQAAAAAAAEAAAAAAAAAEkFscmVhZHlJbml0aWFsaXplZAAAAAAAAgAAAAAAAAAMVW5hdXRob3JpemVkAAAAAwAAAAAAAAAQQ29kZUFscmVhZHlUYWtlbgAAAAQAAAAAAAAADENvZGVOb3RGb3VuZAAAAAUAAAAAAAAAC0ludmFsaWRUaWVyAAAAAAYAAAAAAAAADEludmFsaWRJbnB1dAAAAAcAAAAAAAAADE5vdENvZGVPd25lcgAAAAgAAAAAAAAAC0NvZGVUb29Mb25nAAAAAAkAAAAAAAAAFUludmFsaWRDb2RlQ2hhcmFjdGVycwAAAAAAAAoAAAAAAAAACUVtcHR5Q29kZQAAAAAAAAsAAAAAAAAAEUludmFsaWRUaWVyQ29uZmlnAAAAAAAADAAAAAAAAAAWU2VsZlJlZmVycmFsTm90QWxsb3dlZAAAAAAADQ==",
        "AAAAAQAAAAAAAAAAAAAAClRpZXJDb25maWcAAAAAAAIAAAAAAAAAEmRpc2NvdW50X3NoYXJlX2JwcwAAAAAABAAAAAAAAAAQdG90YWxfcmViYXRlX2JwcwAAAAQ=",
        "AAAAAgAAAAAAAAAAAAAAC1JlZmVycmFsS2V5AAAAAAYAAAABAAAAAAAAAAlDb2RlT3duZXIAAAAAAAABAAAADgAAAAEAAAAAAAAAClRyYWRlckNvZGUAAAAAAAEAAAATAAAAAQAAAAAAAAAMUmVmZXJyZXJUaWVyAAAAAQAAABMAAAABAAAAAAAAAApUaWVyQ29uZmlnAAAAAAABAAAABAAAAAEAAAAAAAAADlJlZmVycmVyVm9sdW1lAAAAAAABAAAAEwAAAAEAAAAAAAAAFFRpZXJVcGdyYWRlVGhyZXNob2xkAAAAAQAAAAQ=",
        "AAAABQAAAAAAAAAAAAAADVRyYWRlckNvZGVTZXQAAAAAAAABAAAAB3JlZl9zZXQAAAAAAgAAAAAAAAAGdHJhZGVyAAAAAAATAAAAAAAAAAAAAAAEY29kZQAAAA4AAAAAAAAAAg==",
        "AAAABQAAAAAAAAAAAAAADkNvZGVSZWdpc3RlcmVkAAAAAAABAAAAB3JlZl9yZWcAAAAAAgAAAAAAAAAGY2FsbGVyAAAAAAATAAAAAAAAAAAAAAAEY29kZQAAAA4AAAAAAAAAAg==",
        "AAAABQAAAAAAAAAAAAAAD1JlZmVycmVyVGllclNldAAAAAABAAAADHJlZl9zZXRfdGllcgAAAAQAAAAAAAAACHJlZmVycmVyAAAAEwAAAAAAAAAAAAAACG9sZF90aWVyAAAABAAAAAAAAAAAAAAACG5ld190aWVyAAAABAAAAAAAAAAAAAAABWFkbWluAAAAAAAAEwAAAAAAAAAC",
        "AAAABQAAAAAAAAAAAAAAFFJlZmVycmVyVGllclVwZ3JhZGVkAAAAAQAAAAdyZWZfdXBnAAAAAAQAAAAAAAAACHJlZmVycmVyAAAAEwAAAAAAAAAAAAAACG9sZF90aWVyAAAABAAAAAAAAAAAAAAACG5ld190aWVyAAAABAAAAAAAAAAAAAAAEWN1bXVsYXRpdmVfdm9sdW1lAAAAAAAACgAAAAAAAAAC",
        "AAAABQAAAAAAAAAAAAAAGENvZGVPd25lcnNoaXBUcmFuc2ZlcnJlZAAAAAEAAAAHcmVmX3hmcgAAAAADAAAAAAAAAARjb2RlAAAADgAAAAAAAAAAAAAABGZyb20AAAATAAAAAAAAAAAAAAACdG8AAAAAABMAAAAAAAAAAg==",
        "AAAAAAAAAD9VcGdyYWRlIHRoZSBjb250cmFjdCB3YXNtLiBPbmx5IHRoZSBzdG9yZWQgYWRtaW4gbWF5IGNhbGwgdGhpcy4AAAAAB3VwZ3JhZGUAAAAAAQAAAAAAAAANbmV3X3dhc21faGFzaAAAAAAAA+4AAAAgAAAAAA==",
        "AAAAAAAAAAAAAAAKaW5pdGlhbGl6ZQAAAAAAAQAAAAAAAAAFYWRtaW4AAAAAAAATAAAAAA==",
        "AAAAAAAAAlRTZWxmLXNlcnZpY2Uga2VlcC1hbGl2ZSBmb3IgYSByZWdpc3RlcmVkIGNvZGUgKGlzc3VlICM0NDUpLgoKYHJlZ2lzdGVyX2NvZGVgIG9ubHkgc2V0cyB0aGUgYENvZGVPd25lcmAgZW50cnkncyBpbml0aWFsIFRUTCwgYW5kIHRoYXQKVFRMIGlzIG90aGVyd2lzZSBvbmx5IGJ1bXBlZCBhcyBhIHNpZGUgZWZmZWN0IG9mIGEgdHJhZGVyIGludGVyYWN0aW5nCndpdGggdGhlIGNvZGUuIEEgY29kZSBub2JvZHkgaXMgYWN0aXZlbHkgdHJhZGluZyB3aXRoIGNhbiB0aGVyZWZvcmUgcnVuCm91dCBpdHMgVFRMOyBvbmNlIGFyY2hpdmVkLCBgcmVnaXN0ZXJfY29kZWAncyBleGlzdGVuY2UgY2hlY2sgcmVhZHMgYXMKInVucmVnaXN0ZXJlZCIgYW5kIGEgZGlmZmVyZW50IGFkZHJlc3MgY2FuIGNsYWltIHRoZSBzYW1lIGNvZGUgc3RyaW5nLApzaWxlbnRseSB0YWtpbmcgb3ZlciBhbnkgdHJhZGVyIHN0aWxsIGxpbmtlZCB0byBpdC4gQ2FsbGluZyB0aGlzIGxldHMKdGhlIGN1cnJlbnQgb3duZXIgZXh0ZW5kIHRoZSBUVEwgYXQgd2lsbCwgaW5kZXBlbmRlbnQgb2YgdHJhZGVyCmFjdGl2aXR5LiBPbmx5IHRoZSBjdXJyZW50IG93bmVyIG1heSBjYWxsIGl0LgAAAApyZW5ld19jb2RlAAAAAAACAAAAAAAAAAZjYWxsZXIAAAAAABMAAAAAAAAABGNvZGUAAAAOAAAAAA==",
        "AAAAAAAAAUdSZWdpc3RlciBhIG5ldyByZWZlcnJhbCBjb2RlOyBjYWxsZXIgYmVjb21lcyB0aGUgb3duZXIuCgpgY29kZWAgbXVzdCBiZSAx4oCTMjAgYnl0ZXMgb2YgYFtBLVphLXowLTlfLV1gLiBSZXZlcnRzIHdpdGg6Ci0gYEVtcHR5Q29kZWAgaWYgYGNvZGVgIGlzIGVtcHR5Ci0gYENvZGVUb29Mb25nYCBpZiBgY29kZS5sZW4oKSA+IE1BWF9SRUZFUlJBTF9DT0RFX0xFTkdUSGAKLSBgSW52YWxpZENvZGVDaGFyYWN0ZXJzYCBpZiBgY29kZWAgY29udGFpbnMgZGlzYWxsb3dlZCBieXRlcwotIGBDb2RlQWxyZWFkeVRha2VuYCBpZiB0aGUgY29kZSBpcyBhbHJlYWR5IHJlZ2lzdGVyZWQAAAAADXJlZ2lzdGVyX2NvZGUAAAAAAAACAAAAAAAAAAZjYWxsZXIAAAAAABMAAAAAAAAABGNvZGUAAAAOAAAAAA==",
        "AAAAAAAAAExSZXR1cm4gdGhlIG93bmVyIGFkZHJlc3MgZm9yIGEgZ2l2ZW4gcmVmZXJyYWwgY29kZSwgb3IgTm9uZSBpZiB1bnJlZ2lzdGVyZWQuAAAADmdldF9jb2RlX293bmVyAAAAAAABAAAAAAAAAARjb2RlAAAADgAAAAEAAAPoAAAAEw==",
        "AAAAAAAAAEFDb25maWd1cmUgdGhlIHJlYmF0ZS9kaXNjb3VudCBwYXJhbWV0ZXJzIGZvciBhIHRpZXIgKGFkbWluIG9ubHkpLgAAAAAAAA9zZXRfdGllcl9jb25maWcAAAAAAwAAAAAAAAAFYWRtaW4AAAAAAAATAAAAAAAAAAR0aWVyAAAABAAAAAAAAAAGY29uZmlnAAAAAAfQAAAAClRpZXJDb25maWcAAAAAAAA=",
        "AAAAAAAAAHNSZWdpc3RlciB0aGUgYXV0aG9yaXplZCBvcmRlcl9oYW5kbGVyIGFkZHJlc3MgKGFkbWluIG9ubHkpLgpPbmx5IHRoaXMgYWRkcmVzcyBtYXkgY2FsbCBgaW5jcmVtZW50X3JlZmVycmVyX3ZvbHVtZWAuAAAAABFzZXRfb3JkZXJfaGFuZGxlcgAAAAAAAAIAAAAAAAAABWFkbWluAAAAAAAAEwAAAAAAAAANb3JkZXJfaGFuZGxlcgAAAAAAABMAAAAA",
        "AAAAAAAAAClTZXQgdGhlIHRpZXIgZm9yIGEgcmVmZXJyZXIgKGFkbWluIG9ubHkpLgAAAAAAABFzZXRfcmVmZXJyZXJfdGllcgAAAAAAAAMAAAAAAAAABWFkbWluAAAAAAAAEwAAAAAAAAAIcmVmZXJyZXIAAAATAAAAAAAAAAR0aWVyAAAABAAAAAA=",
        "AAAAAAAAAEpMb29rIHVwIHRoZSByZWZlcnJhbCBjb2RlIGZvciBhIHRyYWRlciwgYW5kIHJldHVybiB0aGUgcmVmZXJyZXIncyBhZGRyZXNzLgAAAAAAE2dldF90cmFkZXJfcmVmZXJyZXIAAAAAAQAAAAAAAAAGdHJhZGVyAAAAAAATAAAAAQAAA+gAAAAT",
        "AAAAAAAAAiRBZG1pbi1vbmx5IG92ZXJyaWRlIGZvciBhIHJlZmVycmVyJ3Mgc3RvcmVkIGxpZmV0aW1lIHZvbHVtZSAoaXNzdWUgIzQ0NCkuCgpgaW5jcmVtZW50X3JlZmVycmVyX3ZvbHVtZWAncyBhdXRvLXVwZ3JhZGUgcmUtZXZhbHVhdGVzIHRpZXIgdGhyZXNob2xkcwpwdXJlbHkgZnJvbSB0aGlzIHZhbHVlLCB3aGljaCBvdGhlcndpc2Ugb25seSBldmVyIGdyb3dzLiBBIG1hbnVhbApgc2V0X3JlZmVycmVyX3RpZXJgIGRvd25ncmFkZSBpcyB0aGVyZWZvcmUgbm90IGR1cmFibGUgb24gaXRzIG93bjogdGhlCnZlcnkgbmV4dCB0cmFkZSByZWFkcyB0aGUgdW5jaGFuZ2VkLCBzdGlsbC1sYXJnZSBjdW11bGF0aXZlIHZvbHVtZSBhbmQKc2lsZW50bHkgcmUtdXBncmFkZXMgdGhlIHJlZmVycmVyIHJpZ2h0IGJhY2sgdG8gKG9yIGFib3ZlKSB0aGUgdGllciBhbgphZG1pbiBqdXN0IHJlbW92ZWQuIENhbGwgdGhpcyBhbG9uZ3NpZGUgYSBtYW51YWwgdGllciBjaGFuZ2UgdG8gbWFrZQp0aGUgZG93bmdyYWRlIChvciBhbnkgb3RoZXIgdm9sdW1lIGNvcnJlY3Rpb24pIHN0aWNrLgAAABNzZXRfcmVmZXJyZXJfdm9sdW1lAAAAAAMAAAAAAAAABWFkbWluAAAAAAAAEwAAAAAAAAAIcmVmZXJyZXIAAAATAAAAAAAAAAp2b2x1bWVfdXNkAAAAAAAKAAAAAA==",
        "AAAAAAAAAFFSZXR1cm4gdGhlIGZlZSBkaXNjb3VudCBicHMgZm9yIGEgdHJhZGVyIGdpdmVuIHRoZWlyIHJlZmVycmFsIGNvZGUsIG9yIDAgaWYgbm9uZS4AAAAAAAAXZ2V0X3RyYWRlcl9kaXNjb3VudF9icHMAAAAAAQAAAAAAAAAGdHJhZGVyAAAAAAATAAAAAQAAAAQ=",
        "AAAAAAAAANxUcmFuc2ZlciBvd25lcnNoaXAgb2YgYSByZWdpc3RlcmVkIHJlZmVycmFsIGNvZGUgdG8gYSBuZXcgYWRkcmVzcy4KCk9ubHkgdGhlIGN1cnJlbnQgY29kZSBvd25lciBtYXkgY2FsbCB0aGlzLiBSZXF1aXJlcyBhdXRoIGZyb20gYGZyb21gLgpUaGUgbmV3IG93bmVyIChgdG9gKSBpbW1lZGlhdGVseSBiZWNvbWVzIHRoZSBjb2RlJ3MgcmVmZXJyZXIgZm9yIGZlZSBjYWxjdWxhdGlvbnMuAAAAF3RyYW5zZmVyX2NvZGVfb3duZXJzaGlwAAAAAAMAAAAAAAAABGZyb20AAAATAAAAAAAAAAJ0bwAAAAAAEwAAAAAAAAAEY29kZQAAAA4AAAAA",
        "AAAAAAAAAC9SZXR1cm4gdGhlIHJlZmVycmFsIGNvZGUgZm9yIGEgdHJhZGVyLCBvciBOb25lLgAAAAAYZ2V0X3RyYWRlcl9yZWZlcnJhbF9jb2RlAAAAAQAAAAAAAAAGdHJhZGVyAAAAAAATAAAAAQAAA+gAAAAO",
        "AAAAAAAAAD5TZXQgdGhlIHJlZmVycmFsIGNvZGUgZm9yIGEgdHJhZGVyIChsaW5rcyB0aGVtIHRvIGEgcmVmZXJyZXIpLgAAAAAAGHNldF90cmFkZXJfcmVmZXJyYWxfY29kZQAAAAIAAAAAAAAABnRyYWRlcgAAAAAAEwAAAAAAAAAEY29kZQAAAA4AAAAA",
        "AAAAAAAAANRDYWxsZWQgYnkgdGhlIGF1dGhvcml6ZWQgb3JkZXJfaGFuZGxlciBhZnRlciBlYWNoIHRyYWRlIHNldHRsZW1lbnQuCkluY3JlbWVudHMgYHJlZmVycmVyYCdzIGxpZmV0aW1lIHZvbHVtZSBhbmQgYXV0by11cGdyYWRlcyB0aGVpciB0aWVyIGlmCnRoZSBuZXcgY3VtdWxhdGl2ZSB0b3RhbCBjcm9zc2VzIGFueSB0aWVyIHRocmVzaG9sZCAodGllciBvbmx5IGdvZXMgdXApLgAAABlpbmNyZW1lbnRfcmVmZXJyZXJfdm9sdW1lAAAAAAAAAwAAAAAAAAAGY2FsbGVyAAAAAAATAAAAAAAAAAhyZWZlcnJlcgAAABMAAAAAAAAACnZvbHVtZV91c2QAAAAAAAoAAAAA",
        "AAAAAAAAAJRTZXQgdGhlIGN1bXVsYXRpdmUgdm9sdW1lIHRocmVzaG9sZCB0aGF0IHRyaWdnZXJzIGFuIHVwZ3JhZGUgdG8gYHRpZXJgIChhZG1pbiBvbmx5KS4KVGllcnMgMOKAkzIgYXJlIHZhbGlkLiBUaWVyIDAgaXMgdGhlIGJhc2UgYW5kIGhhcyBubyB0aHJlc2hvbGQuAAAAGnNldF90aWVyX3VwZ3JhZGVfdGhyZXNob2xkAAAAAAADAAAAAAAAAAVhZG1pbgAAAAAAABMAAAAAAAAABHRpZXIAAAAEAAAAAAAAAA10aHJlc2hvbGRfdXNkAAAAAAAACgAAAAA=",
        "AAAAAAAAAFdSZXR1cm4gdGhlIGxpZmV0aW1lIGN1bXVsYXRpdmUgdHJhZGluZyB2b2x1bWUgZm9yIGEgcmVmZXJyZXIsIGluIFVTRCAoRkxPQVRfUFJFQ0lTSU9OKS4AAAAAHmdldF9yZWZlcnJlcl9jdW11bGF0aXZlX3ZvbHVtZQAAAAAAAQAAAAAAAAAIcmVmZXJyZXIAAAATAAAAAQAAAAo=" ]),
      options
    )
  }
  public readonly fromJSON = {
    upgrade: this.txFromJSON<null>,
        initialize: this.txFromJSON<null>,
        renew_code: this.txFromJSON<null>,
        register_code: this.txFromJSON<null>,
        get_code_owner: this.txFromJSON<Option<string>>,
        set_tier_config: this.txFromJSON<null>,
        set_order_handler: this.txFromJSON<null>,
        set_referrer_tier: this.txFromJSON<null>,
        get_trader_referrer: this.txFromJSON<Option<string>>,
        set_referrer_volume: this.txFromJSON<null>,
        get_trader_discount_bps: this.txFromJSON<u32>,
        transfer_code_ownership: this.txFromJSON<null>,
        get_trader_referral_code: this.txFromJSON<Option<Buffer>>,
        set_trader_referral_code: this.txFromJSON<null>,
        increment_referrer_volume: this.txFromJSON<null>,
        set_tier_upgrade_threshold: this.txFromJSON<null>,
        get_referrer_cumulative_volume: this.txFromJSON<u128>
  }
}