# SO4 Contracts Cleanup Map

This repository is a Soroban port of GMX Synthetics contracts. The open-source
issue campaign produced useful work, but also left historical docs, stale claims,
and non-Soroban artifacts in the tree. Use this file as the cleanup map before
opening new implementation work.

## Current Source Of Truth

| Area | Source |
|---|---|
| Architecture decisions | `IMPLEMENTATION_PLAN.md` |
| Operator docs | `README.md`, `mx/README.md` |
| Contributor rules | `CONTRIBUTING.md` |
| Security posture | `SECURITY_REVIEW.md` |
| Historical issue context | `issues_v2.md` |
| Actual implementation state | Rust code and `cargo test --workspace` |

## Canonical Architecture

- Requests and positions are stored locally in their handler contracts.
- `data_store` is for shared config, pool accounting, lists, and fee/accounting
  values, not for primary request/position structs.
- User token movement enters through `exchange_router`; handlers snapshot vault
  receipts instead of pulling arbitrary user balances.
- The v1 oracle path is custom keeper signatures with ed25519 verification.
- Core storage and vault contracts are immutable unless the Rust code exposes an
  `upgrade` entrypoint.

## Upgrade Reality

Upgradeable in the current Rust code:

- `deposit_handler`
- `order_handler`
- `liquidation_handler`
- `fee_handler`
- `referral_storage`
- `reader`
- `exchange_router`

Immutable in the current Rust code:

- `role_store`
- `data_store`
- `oracle`
- `market_factory`
- `market_token`
- `deposit_vault`
- `withdrawal_vault`
- `order_vault`
- `withdrawal_handler`
- `adl_handler`

## Quarantine Candidates

These directories are not workspace members and are written against MultiversX
APIs, not Soroban:

- `libs/deposit_flow`
- `libs/withdrawal_flow`
- `libs/position_list`
- `libs/storage_ttl`

Do not copy patterns from these directories into Soroban contracts. The next
cleanup PR should either delete them, move them under an explicit archive, or
port any still-useful idea into active Soroban crates.

## Recommended Cleanup PR Order

1. Delete or archive the MultiversX artifacts listed above.
2. Reconcile `issues_v2.md` into a smaller live backlog with statuses:
   `done`, `partial`, `deferred`, `obsolete`, `needs-audit`.
3. Verify README manual deployment commands against generated contract specs or
   `stellar contract inspect` output after a fresh build.
4. Run `cargo test --workspace` and record any failing crates as the real launch
   blocker list.
5. Add a small `docs/config-keys.md` catalog generated from `libs/keys`.
6. Add a small `docs/events.md` catalog from emitted event topics.
7. Decide whether immutable-but-stateful `withdrawal_handler` and `adl_handler`
   should remain immutable for v1 or receive upgrade entrypoints before launch.

## Non-Negotiable Launch Checks

- No stale docs claiming all protocol state lives in `data_store`.
- No production path using test-only oracle price helpers.
- No role-gated public function without a negative authorization test.
- No custody contract upgradeability unless explicitly re-reviewed.
- No workspace-visible non-Soroban crates.
- No mainnet deployment without Stellar multisig admin custody.
