# CLOB Storage Entry Classification and TTL Strategy (Issues #915 & #916)

## 1. Overview

Soroban state entries require rent (measured in ledger sequences) to remain live. If an entry's Time-To-Live (TTL) drops to zero without being extended, the entry is archived by the network.

To guarantee that active markets and user funds remain accessible while avoiding unbounded protocol expenditure on dead state, the CLOB implements an explicit **3-Tier Storage Classification** with defined TTL extension thresholds and non-renewal of abandoned zero-balance state.

---

## 2. Storage Classification Tiers

| Tier | Storage Entries Included | Lifetime Objective | TTL Threshold | Renewal Target |
|---|---|---|---|---|
| **Tier 1: Permanent-Critical** | Contract instances, admin/roles, market configuration, market registry index, protocol fee buckets | Must never expire under any protocol state. | 518,400 ledgers (~30 days) | 1,036,800 ledgers (~60 days) |
| **Tier 2: Live-While-Active** | Non-zero trader balances (`available > 0` or `locked > 0`), open resting orders, active price levels, FIFO queue nodes | Maintained while active; renewed on user interaction or fill. | 259,200 ledgers (~15 days) | 518,400 ledgers (~30 days) |
| **Tier 3: Disposable / Indexable** | Zero-balance trader accounts, cancelled/settled order records, expired orders, temporary price signatures | Short-lived or unextended. Archived/cleaned without protocol subsidy. | No renewal for zero-balance; 120 ledgers for prices | No extension (allowed to archive or cleaned) |

---

## 3. Threshold Constants and Parameters

All values are specified in ledger sequences (nominal 5 seconds per ledger):

```rust
/// Tier 1: Permanent-Critical threshold (~30 days)
pub const PERMANENT_CRITICAL_THRESHOLD: u32 = 518_400;
/// Tier 1: Permanent-Critical target (~60 days)
pub const PERMANENT_CRITICAL_TARGET: u32 = 1_036_800;

/// Tier 2: Live-While-Active threshold (~15 days)
pub const LIVE_ACTIVE_THRESHOLD: u32 = 259_200;
/// Tier 2: Live-While-Active target (~30 days)
pub const LIVE_ACTIVE_TARGET: u32 = 518_400;

/// Tier 3: Temporary price TTL (~10 minutes)
pub const TEMPORARY_PRICE_TTL: u32 = 120;
```

---

## 4. Renewal Policies and Rules

### Rule 1: Active Non-Zero Balances Are Renewed
Whenever a trader interacts with the exchange (deposit, order placement, fill, cancel, or withdrawal), if their resulting balance is non-zero (`available > 0 || locked > 0`), the balance storage entry TTL is extended to `LIVE_ACTIVE_TARGET` if its remaining TTL is below `LIVE_ACTIVE_THRESHOLD`.

### Rule 2: Zero-Balance Entries Are Never Renewed (Issue #916)
If a trader's balance reaches zero (`available == 0 && locked == 0`):
- The entry is **NOT** extended.
- Unrelated market operations (e.g., other traders' fills, order cleanup batches, fee sweeps) MUST NOT call `extend_ttl` on zero-balance entries.
- In bulk operations, any trader entry with zero balance is filtered out from TTL renewal pipelines.

### Rule 3: Resting Orders are Renewed on Mutation
When an order is partially filled or reduced, its TTL is extended. When an order is fully filled or cancelled, the storage entry is removed or allowed to expire.

---

## 5. Rent and Restore Responsibilities

The economic cost of Soroban storage rent and potential entry restoration is allocated as follows:

| Stakeholder | Responsibility | Mechanism |
|---|---|---|
| **Protocol / Treasury** | Permanent-Critical configuration, registry, and keeper infrastructure entries. | Funded via collected protocol trading fees (`protocol_fees`). |
| **Active Traders** | Live orders and non-zero balances during active usage. | Gas and rent paid as part of transaction fees during order placement and execution. |
| **Inactive Traders** | Restoring long-abandoned accounts that had non-zero balances. | If an inactive trader with funds leaves the account untouched past `LIVE_ACTIVE_TARGET`, they pay the Soroban restore footprint transaction fee upon return. Internal balances and proofs are intact. |
| **Keepers** | Order execution and expired order cleanup. | Compensated by user-escrowed `execution_fee` attached to every order. |

---

## 6. Archive and Restore Scenarios

1. **Active Account Interaction**:
   Trader deposits or trades $\rightarrow$ balance remains non-zero $\rightarrow$ TTL bumped to ~30 days $\rightarrow$ state never archived under regular usage.

2. **Account Depletion (Safe Non-Renewal)**:
   Trader withdraws all funds $\rightarrow$ `available = 0, locked = 0` $\rightarrow$ TTL is not extended $\rightarrow$ entry archives safely without wasting protocol rent $\rightarrow$ no liability exists.

3. **Restoration of Inactive Non-Zero Balance**:
   Trader leaves funds dormant for > 30 days $\rightarrow$ entry archived by Soroban network $\rightarrow$ trader submits a Stellar transaction with restore footprint $\rightarrow$ entry revived $\rightarrow$ `SafeCustodyLedger::reconcile_with_vault` passes with 100% precision.
