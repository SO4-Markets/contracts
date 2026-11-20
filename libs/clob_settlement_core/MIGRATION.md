# Migrating from `&'static str` errors to `SettlementError`

This document is for contributors adding functions to `clob_settlement_core`
or migrating call sites that previously returned `Result<_, &'static str>`.

## Why `&'static str` errors cannot cross a contract boundary

Soroban encodes errors as `ScError { kind: Contract, code: u32 }`.  A
`&'static str` has no numeric representation and cannot be serialised into an
`ScVal`, so the Soroban host panics when it encounters one at a contract
boundary.  Every existing contract in `contracts/*` avoids this by using
`#[contracterror]` enums; `clob_settlement_core` follows the same pattern.

## Before (do not write this)

```rust
pub fn validate_tick(tick: i64, tick_size: i64) -> Result<(), &'static str> {
    if tick_size <= 0 {
        return Err("tick size must be positive");
    }
    if tick % tick_size != 0 {
        return Err("tick not aligned to grid");
    }
    Ok(())
}
```

A contract calling this must do:

```rust
// fragile: any reword silently breaks the match
validate_tick(tick, tick_size).map_err(|s| match s {
    "tick size must be positive" => Error::InvalidTickSize,
    "tick not aligned to grid"  => Error::InvalidTick,
    _                           => Error::Unknown,
})?;
```

## After (write this)

```rust
use clob_settlement_core::SettlementError;

pub fn validate_tick(tick: i64, tick_size: i64) -> Result<(), SettlementError> {
    if tick_size <= 0 {
        return Err(SettlementError::InvalidTickSize);
    }
    if tick % tick_size != 0 {
        return Err(SettlementError::InvalidTick);
    }
    Ok(())
}
```

A contract calling this uses `?` or `.map_err` to a contract-local alias:

```rust
// In the contract's own #[contracterror] enum, reserve a range for
// settlement errors, e.g.:
//   SettlementInvalidTick        = 1000,
//   SettlementInvalidTickSize    = 1001,
// and implement:
//   impl From<SettlementError> for ContractError { … }
//
// Then at the call site:
validate_tick(tick, tick_size)?;
```

## Discriminant ranges

| Range   | Category               |
|---------|------------------------|
| 1–49    | Validation             |
| 50–99   | Fee / quote arithmetic |
| 100–149 | Fill / settlement      |
| 150–199 | Deposit / withdrawal   |
| 200–249 | Market-config          |
| 250–299 | Registry               |
| 300–349 | Deployment             |

New variants must stay within their category range.  If a category fills up,
open a PR to allocate a new range before adding variants.

## Adding a new variant

1. Choose the appropriate category range.
2. Pick the next unused discriminant in that range.
3. Add the variant to `SettlementError` in `src/lib.rs`.
4. Update `MIGRATION.md` if the variant warrants a usage example.
5. Never renumber or reorder existing variants.
