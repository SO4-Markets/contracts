# Swap Path Optimization

## Summary

`swap_path` sits on the hot order/swap path for `exchange_router` and `order_handler`. The previous shape used a dynamic vector type at the contract boundary, which kept the path unbounded in the type itself and made every caller pay the overhead of a general-purpose container for a value that is known to be capped.

This repo now uses a fixed-hop `SwapPath` contract type with five optional address slots and a shared `MAX_SWAP_PATH_LENGTH` constant of `5`.

## Audit

Exact `swap_path: Vec<Address>` occurrences before the change:

| File | Line |
| --- | ---: |
| `libs/types/src/lib.rs` | 102 |
| `libs/types/src/lib.rs` | 146 |
| `libs/decrease_position_utils/src/lib.rs` | 84 |

## Benchmark

Benchmark target: `cargo bench -p order-handler --bench swap_path`

Workload:
- construct a 5-hop path
- read `len`
- walk every hop
- read first/last hop

Measured instruction counts from Soroban budget:

| Representation | CPU instructions | Memory bytes |
| --- | ---: | ---: |
| `std::vec::Vec<Address>` | 16,234 | 5,811 |
| `soroban_sdk::Vec<Address>` | 20,106 | 5,947 |
| `[Option<Address>; 5]` | 16,686 | 5,811 |

Criterion runtime also put the fixed-size representation ahead of `soroban_sdk::Vec` and effectively tied with `std::vec::Vec`.

## Chosen Approach

Chosen representation: `SwapPath`, a fixed 5-slot contract type:

```rust
pub const MAX_SWAP_PATH_LENGTH: usize = 5;

#[contracttype]
pub struct SwapPath {
    pub hop0: Option<Address>,
    pub hop1: Option<Address>,
    pub hop2: Option<Address>,
    pub hop3: Option<Address>,
    pub hop4: Option<Address>,
}
```

Reasoning:
- `soroban_sdk::Vec<Address>` was the slowest option in the benchmark.
- `std::vec::Vec<Address>` posted the lowest CPU count, but it is not the right contract-facing representation for `#![no_std]` Soroban types and does not encode the 5-hop limit in the type.
- The fixed-size representation is the best deployable option here: bounded by construction, close to the best CPU result, and materially better than `soroban_sdk::Vec`.

## Before / After

Before:

```rust
pub struct CreateOrderParams {
    pub initial_collateral_token: Address,
    pub swap_path: Vec<Address>,
    pub size_delta_usd: i128,
}
```

After:

```rust
pub struct CreateOrderParams {
    pub initial_collateral_token: Address,
    pub swap_path: SwapPath,
    pub size_delta_usd: i128,
}
```

Order creation now also enforces the shared cap:

```rust
if params.swap_path.len() as usize > MAX_SWAP_PATH_LENGTH {
    panic_with_error!(&env, Error::SwapPathTooLong);
}
```

## Tradeoffs

- The fixed-hop shape is less ergonomic than a growable vector, so helper constructors (`SwapPath::new`, `SwapPath::from_array`) are part of the API now.
- The type carries a small amount of unused space for paths shorter than five hops.
- In return, the upper bound is explicit, call sites are predictable, and the contract no longer depends on a dynamic swap-path container for a bounded domain.
