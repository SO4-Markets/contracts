# Issue #891 [CLOB-015]: Contract Upgradeability Strategy

## Overview

This document defines the upgrade policy for all SO4 Markets CLOB contracts. Decisions are consensus-critical and must be applied consistently across the protocol.

## Contract Upgrade Policies

### Immutable Contracts (No Upgrades Allowed)

The following contracts contain core protocol logic that must never change once initialized:

- **market_factory**: Creates markets with fixed rules. Immutability prevents accidental rule changes. New markets are issued as separate instances if the protocol needs to evolve.
- **order_handler**: Core order matching logic. Changes would risk historic consistency and break SDK assumptions about order settlement. 
- **fee_batch_sweeper**: Fee collection logic must be deterministic. Immutable ensures fees accrue as originally promised to users.

**Enforcement**: No upgrade entrypoint; rejected at contract invocation level.

### Upgradeable Contracts (Controlled Updates)

Contracts with non-core, maintenance-focused logic:

- **deposit_handler, withdrawal_handler, order_vault, deposit_vault, withdrawal_vault**: Asset management. May need updates for new token types, bridge integrations, or to fix corner cases without changing core settlement logic.
- **role_store**: Authorization logic. Role definitions can be updated after extensive testing and community review.
- **data_store**: Data indexing and query logic. May need optimization or schema adjustments.
- **reader**: Public API contract. Updates must preserve backward compatibility or provide migration path.
- **oracle**: Price feed aggregation. May need to add/remove data sources or adjust staleness windows.
- **insurance_fund_router, liquidation_handler**: Liquidation flow adjustments (e.g., new incentive structures).

**Upgrade Process**:
1. Submit new WASM hash via `UpgradeAuthority` (multi-sig address, separate from `Admin`).
2. Hash is stored in `pending_upgrade` with `proposal_timestamp`.
3. Timelock: minimum 7 days on testnet, 14 days on mainnet.
4. Execute upgrade via `confirm_upgrade(hash)` after timelock passes.
5. On execution, store old hash in `previous_versions` for rollback eligibility.

### Contracts with Custom Policies

- **market_util_reader, market_token**: Read-only queries and token minting. Usually immutable; upgradeable only if oracle feeds change. Stored in contract spec.
- **liquidation_handler**: May need adjustments for new liquidation incentive models; consider upgradeability case-by-case with governance vote.
- **referral_storage**: Protocol incentives. Upgradeable to adjust reward schemes; changes require historical data migration.

## Storage Versioning

### Approach

Each upgradeable contract contains a `StorageVersion` key:

```rust
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StorageVersion {
    pub major: u32,    // Incompatible changes (migration required)
    pub minor: u32,    // Backward-compatible additions
    pub patch: u32,    // Bug fixes, no schema change
}
```

**Storage Layout**: Stored at a well-known key to prevent collision:
```
Key: 0x_STORAGE_VERSION
Value: (major, minor, patch)
```

### Migration Rules

1. **Patch → Patch**: No migration. Old data readable by new code.
2. **Minor → Minor**: New fields are optional; old contracts see defaults.
3. **Major → Major**: WASM upgrade is blocked if major version mismatches. Manual migration required:
   - Old contract exports migration entrypoint: `migrate_to_v2(data)`.
   - New contract calls it, transforms data, and stores under new schema.
   - Requires separate `storage_migration` transaction.

### Version Check at Contract Start

```rust
fn verify_storage_version() -> Result<StorageVersion, Error> {
    let version: Option<StorageVersion> = env.storage().instance().get(&DataKey::StorageVersion);
    let version = version.ok_or(Error::NoStorageVersion)?;
    if version.major != CURRENT_MAJOR {
        return Err(Error::IncompatibleStorageVersion(version.major));
    }
    Ok(version)
}
```

## Rollback Plan

### Testnet

If an upgrade introduces a bug:
1. Pause affected market (or pause protocol).
2. Deploy a new contract with a fixed WASM.
3. Migrate state from previous version using export/import snapshot.
4. Re-activate with new contract address.

### Mainnet

Rollback is more restrictive:
1. `Admin` can initiate rollback only within 7 days of upgrade.
2. Rollback reverts WASM hash to `previous_versions[hash]`.
3. If more than 7 days have passed, rollback requires governance vote (separate infrastructure, TBD).
4. State is never reverted (immutable ledger); code changes only.

## Authority Separation

- **Admin**: Grant/revoke roles, create markets, set emergency states.
- **UpgradeAuthority**: Propose and confirm WASM upgrades. Separate account/multi-sig to enforce slow rollout.
- **PauseAuthority**: Emergency pause. Separate to allow rapid response without Admin privileges.

## Summary Table

| Contract | Policy | Upgrade Process | Storage Versioning |
|----------|--------|-----------------|-------------------|
| market_factory | Immutable | N/A | N/A |
| order_handler | Immutable | N/A | N/A |
| fee_batch_sweeper | Immutable | N/A | N/A |
| deposit_handler | Upgradeable | 7d timelock → execute | Major/minor/patch |
| withdrawal_handler | Upgradeable | 7d timelock → execute | Major/minor/patch |
| order_vault | Upgradeable | 7d timelock → execute | Major/minor/patch |
| oracle | Upgradeable | 7d timelock → execute | Major/minor/patch |
| role_store | Upgradeable | 7d timelock → execute | Major/minor/patch |
| data_store | Upgradeable | 7d timelock → execute | Major/minor/patch |
| reader | Upgradeable | 7d timelock → execute | Major/minor/patch |
| insurance_fund_router | Upgradeable (case-by-case) | 7d timelock → execute | Major/minor/patch |
| liquidation_handler | Upgradeable (case-by-case) | 7d timelock → execute | Major/minor/patch |

## Testing

All upgrade scenarios must be tested before mainnet:
- [ ] Immutable contract rejects upgrade attempt.
- [ ] Timelock prevents premature execution.
- [ ] Storage version migration enforced (major change rejected without migration).
- [ ] Rollback restores previous WASM hash within timelock window.
- [ ] Old state readable by new code (backward-compatibility).
- [ ] Events emitted for all upgrade transitions (traceability).

## Consensus and Coordination

Changes to this policy require:
1. Amendment of this document.
2. Community review and vote.
3. Updates to all deployments (testnet, mainnet).
4. SDK documentation update.

NO runtime changes to immutable contracts or upgrade logic without coordination.