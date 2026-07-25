# Soroban storage TTL and rent strategy

Every SO4.market ledger entry has a time-to-live (TTL), measured in ledger
sequences. Rent is paid when entries are created, enlarged, or extended. This
document describes the policy implemented by the current code, not an intended
future policy.

> **Current-state warning:** the protocol does not yet renew instance storage,
> and most long-lived persistent entries still receive only the network's
> default TTL at write time. There are no `MIN_POSITION_TTL`,
> `MAX_POSITION_TTL`, `bump_core_ttl`, or `bump_persistent_ttl` definitions in
> the Soroban workspace, and no repository-wide renewal policy. Beyond oracle
> prices and token allowances, `order_handler`, `data_store`, and
> `referral_storage` each define their own local
> `PERSISTENT_BUMP_TARGET`/`MIN_BUMP_THRESHOLD` pair and call `extend_ttl` on
> specific keys — see "Persistent storage" below for exactly which ones, and
> which related keys in those same contracts are still uncovered. Operators
> must monitor the remaining TTL of contract instances and every persistent
> entry without a documented bump until a repository-wide renewal policy
> exists.

## Storage tiers used by the protocol

Soroban network settings determine the minimum TTL assigned to newly written
entries and the maximum TTL to which an entry may be extended. Those values are
network configuration, not constants in this repository, and may change after
a protocol upgrade. Do not hard-code them in keepers.

| Tier | Current uses | Cost/lifetime characteristics | Current bump policy |
|---|---|---|---|
| Instance | Initialization flag, admin, role store, data store, oracle and vault/handler addresses; market-token metadata; test-token pause state; selected `data_store` configuration values | One contract-instance ledger entry backs all instance keys. Efficient for small, frequently read shared configuration, but growing it increases the size/rent of the shared entry. Its TTL is tied to the contract instance. | **None.** No production entrypoint calls `env.storage().instance().extend_ttl(...)`. |
| Persistent | Positions and orders in `order_handler`; pending deposits and withdrawals; `data_store` pool amounts, OI, factors, market metadata and index sets; roles, referrals, balances and token supply | Each key has an independent TTL and can be archived. Appropriate for durable protocol/user state, but every key must be monitored and renewed independently. | **Partial.** `order_handler` calls `extend_ttl` on `Order`, `OrderExpiry`, and `OrderFrozen`. `data_store`'s bytes32-set helpers (`add_bytes32_to_set`, `remove_bytes32_from_set`, and the bytes32-set getters) call it on the set entry itself, which covers the `order_list_key`/`account_order_list_key` and `position_list_key`/`account_position_list_key` enumeration indexes. `referral_storage` calls it on `CodeOwner`, `TraderCode`, `ReferrerTier`, `TierConfig`, `ReferrerVolume`, and `TierUpgradeThreshold`. Positions themselves, deposits, withdrawals, roles, token balances, `data_store`'s scalar/address-set accounting (pool amounts, OI, factors, prices), and a few specific paths noted under "Persistent storage" below are **not** covered. Reads and writes must not be assumed to renew an entry unless the exact code path calls `extend_ttl`. |
| Temporary | Signed oracle prices; SEP-41 allowances in `market_token` and `test_token` | Lowest intended lifetime. Expired temporary entries are permanently removed and cannot be restored, which is desirable for stale prices and allowances. | Oracle prices are extended to 120 ledgers. Allowances are extended to their caller-supplied expiration ledger. |

The unused `libs/storage_ttl` crate is not part of the root Cargo workspace and
uses `multiversx-sc`, not `soroban-sdk`. It is not a Soroban TTL implementation
and must not be imported as one.

## Implemented TTL constants and dynamic values

Wall-clock conversions below use the protocol's nominal five-second ledger
cadence. Ledger close time varies, so ledgers—not minutes—are authoritative.

| Name/location | Value | Approximate time | Purpose |
|---|---:|---:|---|
| `PRICE_TTL_LEDGERS` in `contracts/oracle/src/lib.rs` | 120 ledgers | 10 minutes | Keeps `TempKey::Price(token)` readable across `set_prices` and a batch of later execution transactions. |
| `LEDGER_SEQ_WINDOW` inside `oracle::set_prices` | 60 ledgers | 5 minutes | Rejects a signed price whose attested ledger is too old. This is a freshness check, not an entry TTL. |
| Timestamp freshness check inside `oracle::set_prices` | 300 seconds | 5 minutes | Rejects stale signed attestations independently of storage TTL. |
| Token allowance `ledger_gap` | `expiration_ledger - current_ledger`, saturating at zero | Caller-selected | `market_token::approve`, `market_token::spend_allowance`, and the equivalent `test_token` functions extend the temporary allowance to its declared expiration. |

`MIN_POSITION_TTL` and `MAX_POSITION_TTL` are **not defined**. Positions receive
the network's minimum persistent TTL when first written and are not explicitly
extended afterward — `order_handler`'s only position-TTL entrypoint,
`bump_position_ttl`, rewrites the stored value with a plain `set()` but never
calls `extend_ttl`. The same is true for deposits, withdrawals, balances,
roles, and most `data_store` values. Orders and referral records are the
exception: see "Persistent storage" below for the specific keys `order_handler`
and `referral_storage` do extend, and which related keys in those same
contracts they don't.

The effective wall-clock time for a network value is approximately:

```text
duration_seconds = ttl_ledgers × observed_average_ledger_close_seconds
```

Use the current ledger sequence and the entry's live-until ledger from RPC when
making operational decisions; the ten-minute figures above are explanatory,
not timestamps.

## Bump policy by tier

### Instance storage

There is currently no bump. This affects every deployed contract because its
admin and dependency addresses live in instance storage. A future core helper
should call:

```rust,ignore
env.storage()
    .instance()
    .extend_ttl(bump_threshold_ledgers, extend_to_ledgers);
```

Call such a helper near the start of every public entrypoint, before reading
instance keys. The threshold is the point below which a bump occurs;
`extend_to_ledgers` is the desired TTL measured from the current ledger. Both
must be chosen from current network limits and protocol operating objectives.

Do not place unbounded/user-specific data in instance storage. All instance keys
share one ledger entry and one lifetime.

### Persistent storage

Most durable keys still have no bump — see "Current gaps" below — but three
contracts already extend specific keys inline, each with its own local
`PERSISTENT_BUMP_TARGET` (~30 days) / `MIN_BUMP_THRESHOLD` (~15 days) pair
rather than a shared helper:

- `order_handler` extends `OrderStorageKey::Order` on `create_order` and
  `update_order`, `OrderStorageKey::OrderExpiry` on `create_order` when the
  caller supplies an expiry, and `OrderStorageKey::OrderFrozen` on
  `freeze_order`. `PositionStorageKey::Position` is **not** extended this way:
  the only position-TTL entrypoint, `bump_position_ttl`, rewrites the value
  with a plain `set()` rather than calling `extend_ttl`, and the read-only
  `get_order`/`get_position` getters don't renew anything either.
- `data_store`'s `DataKey::B32Set` helpers (`add_bytes32_to_set`,
  `remove_bytes32_from_set`, `get_bytes32_set_count`, `get_bytes32_set_at`,
  `contains_bytes32`) extend the set entry itself on every call. Every caller
  that adds/removes an order or position to/from an enumeration index goes
  through these — `order_handler`'s `order_list_key`/`account_order_list_key`
  and `increase_position_utils`/`decrease_position_utils`'s
  `position_list_key`/`account_position_list_key` are all covered this way.
  The equivalent `DataKey::AddrSet` helpers (`add_address_to_set`,
  `remove_address_from_set`) do **not** call `extend_ttl`, nor do any of
  `data_store`'s scalar getters/setters (`get_u128`/`set_u128`,
  `apply_delta_to_u128`, `get_address`/`set_address`, etc.) — pool amounts,
  OI, factors, and other `data_store` accounting values have no bump.
- `referral_storage` extends `CodeOwner`, `TraderCode`, `ReferrerTier`,
  `TierConfig`, and `ReferrerVolume` on their write paths and again on read
  paths that touch them (`get_trader_referrer`, `get_trader_discount_bps`,
  `renew_code`), and extends `TierUpgradeThreshold` for every tier the
  auto-upgrade scan in `increment_referrer_volume` reads. It does **not**
  extend `CodeOwner`'s TTL inside `transfer_code_ownership`, so a code that
  changes hands but is never subsequently read or renewed keeps whatever TTL
  it already had; the plain `get_trader_referral_code`/`get_code_owner`
  getters likewise don't renew anything.

Example of the pattern used by all three contracts:

```rust,ignore
env.storage().persistent().extend_ttl(
    &OrderStorageKey::Order(key),
    MIN_BUMP_THRESHOLD,
    PERSISTENT_BUMP_TARGET,
);
```

A future repository-wide helper must still accept the exact typed storage key
and extend the same entry that was read or written:

```rust,ignore
env.storage().persistent().extend_ttl(
    &PositionStorageKey::Position(position_key),
    bump_threshold_ledgers,
    extend_to_ledgers,
);
```

It must be called for every durable entry touched by a successful operation,
including both the primary object and its enumeration/index keys — they are
independent entries with independent TTLs. Today a position open/close does
extend the `data_store` account-position index (via `add_bytes32_to_set`/
`remove_bytes32_from_set`), but that same operation does **not** extend the
position's own `PositionStorageKey::Position` entry, and nothing about the
index write implies the primary object is covered too. This is why a generic
helper cannot infer the full set of entries to renew from any single key.

### Temporary storage

Temporary entries have explicit, purpose-specific lifetimes:

- `oracle::set_prices` and the test-only `set_prices_simple` write
  `TempKey::Price(token)`, then call
  `extend_ttl(&price_key, PRICE_TTL_LEDGERS, PRICE_TTL_LEDGERS)`.
- `market_token::approve` and `test_token::approve` compute `ledger_gap` from
  the SEP-41 `expiration_ledger`, then call
  `extend_ttl(&allowance_key, ledger_gap, ledger_gap)`.
- Spending a nonzero remainder rewrites and re-extends the allowance to the
  original expiration. A zero remainder removes the entry.

Do not add a blanket temporary-storage bump. Expiry is part of the security
model for prices and allowances.

## What happens on expiry

### Contract instance expiry

The contract instance and all instance keys become archived together. Normal
contract invocation cannot proceed until the instance ledger entry and required
contract code are restored. Calls commonly fail while trying to read the admin,
role store, data store, or initialization flag. The contract ID does not become
a fresh, safely re-initializable deployment merely because the entry is
archived.

Restoration requires a Soroban restore transaction and rent payment. Treat an
instance approaching expiry as an incident: extending before archival is
cheaper and avoids downtime.

### Persistent entry expiry

Persistent entries become archived and unavailable to ordinary contract reads.
They are **not necessarily lost forever**: Soroban persistent entries are
designed to be restored by a restore-footprint transaction, after which the
original key/value becomes readable again.

Until restoration, the protocol can behave as if data is absent. That is
dangerous here because many getters use defaults such as zero or `None`:

- an archived position or order may appear missing;
- archived pool/OI/config values may read as zero;
- an archived role assignment may make an authorized handler appear
  unauthorized;
- an archived index entry may make an existing object undiscoverable even if
  its primary entry remains live.

Never recreate or overwrite apparently missing critical state until archival
has been ruled out. Restore the complete related footprint, not just the first
key that caused a failure.

### Temporary entry expiry

Temporary entries are permanently deleted and cannot be restored. In this
protocol that means:

- an expired oracle price must be resubmitted before an execution call; and
- an expired allowance must be approved again.

No user position, order, deposit, withdrawal, role, or pool accounting value
belongs in temporary storage.

## Keeper monitoring policy

Until automatic renewal exists, a keeper or operations service should:

1. Record the current ledger sequence on every poll.
2. Query each contract-instance ledger entry and critical persistent ledger
   entry through Soroban RPC, retaining its `liveUntilLedgerSeq`.
3. Calculate `remaining = liveUntilLedgerSeq - currentLedger`.
4. Warn well before the operational renewal threshold and page when the entry
   enters the emergency window.
5. Group related entries into one inventory: contract instance/code, primary
   object, account/global index sets, roles, and relevant `data_store` keys.
6. Simulate renewal or restoration transactions before submission and alert on
   simulation failure or an unexpected rent increase.

At minimum, monitor:

- all deployed contract instances and contract-code ledger entries;
- every open position and pending order/deposit/withdrawal;
- role assignments for handlers and keepers;
- market registry/config, pool amounts, OI, funding and borrowing accumulators;
- market-token total supply and nonzero balances; and
- each primary object's global and per-account index entries.

Temporary price TTL should be monitored differently. A keeper should refresh
prices immediately before executing requests rather than trying to preserve old
prices past their freshness window.

## Patterns for new code

The helper names proposed in issue #225 are design placeholders, not current
APIs. If `bump_core_ttl` and `bump_persistent_ttl` are introduced later, keep
these responsibilities distinct:

```rust,ignore
fn bump_core_ttl(env: &Env) {
    // Extend the contract instance (and separately ensure contract code stays
    // live) when the remaining TTL crosses an operator-approved threshold.
}

fn bump_persistent_ttl<K>(env: &Env, key: &K)
where
    K: soroban_sdk::IntoVal<Env, soroban_sdk::Val>,
{
    // Extend exactly `key`; callers remain responsible for related index keys.
}
```

Rules for contributors:

- Choose instance storage only for bounded shared configuration.
- Choose persistent storage for recoverable protocol and user state.
- Choose temporary storage only when permanent expiry is intended.
- Define threshold and extend-to values in ledgers, not seconds.
- Never claim that a read/write refreshes TTL unless the same code path calls
  `extend_ttl`.
- Test the low-TTL path with Soroban test utilities before relying on it in
  production.
- Document every new durable key in the keeper inventory and ensure all related
  indexes receive the same renewal treatment.

## Current gaps

The documentation exposes, but does not silently fix, these implementation
gaps:

1. No automatic instance-storage renewal.
2. No persistent TTL renewal for positions themselves
   (`PositionStorageKey::Position`). The `data_store` account-position and
   position-list indexes that reference them **are** renewed, as a side
   effect of `add_bytes32_to_set`/`remove_bytes32_from_set` on every position
   open/close — but the primary position entry is not, so the index can
   outlive the object it points to.
3. No persistent TTL renewal for deposits, withdrawals, roles, token
   balances, or most `data_store` accounting (pool amounts, OI, factors,
   prices) and its `AddrSet` helpers. Orders, `data_store`'s `B32Set`
   enumeration indexes, and referral-code/tier/volume records are a partial
   exception — see "Persistent storage" above for exactly which keys
   `order_handler`, `data_store`, and `referral_storage` cover, and which
   related keys in those same contracts they don't.
4. No repository-wide minimum/maximum TTL policy constants — `order_handler`,
   `data_store`, and `referral_storage` each hard-code their own, identical
   `PERSISTENT_BUMP_TARGET`/`MIN_BUMP_THRESHOLD` pair rather than sharing one.
5. No on-chain TTL query/maintenance entrypoint or bundled off-chain monitor.

Adding those mechanisms changes runtime behavior and rent costs and should be a
separate, tested contract change rather than being smuggled into a documentation
issue.
