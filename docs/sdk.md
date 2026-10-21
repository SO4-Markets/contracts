# SO4 client SDKs

The Rust client is in [`libs/clob_client`](../libs/clob_client) and the TypeScript
package is the repository root package. Both expose the same request shapes:
deposit, withdrawal, place, cancel, and exact-integer quote calculations.

Amounts are represented by `bigint` in TypeScript and `u128`/`i128` in Rust.
Decimal helpers parse strings; they never use JavaScript floating point and
reject values that are not exactly lot- or tick-aligned. Callers must choose
the protocol rounding mode when calculating a quote.

`Transaction.simulate()` and the Rust `Simulator` trait are optional transport
hooks. They return decoded receipts, errors, and canonical events before a
transaction is submitted. A transport should map its network response to the
documented receipt shape rather than silently converting failures into
successful results.

Run the local checks with:

```sh
npm test
cargo test -p clob-client
cargo run -p clob-client --example testnet_flow
```

The examples use a dry-run simulator. Replace it with the network transport
for a testnet flow, retaining the same builders and receipt decoders.
