# Contributing

SO4.market contracts are deployed as a connected protocol graph. Treat
`.deployed/<network>.env` as the source of truth for a network's current
deployment.

## Deployment Rules

- Use `make deploy-all NETWORK=testnet SOURCE=<key>` only for the first full
  deployment of a network.
- If `.deployed/<network>.env` already exists, do not redeploy the protocol graph
  just to test code changes. Use an upgrade command.
- Use `make deploy-force NETWORK=<network> SOURCE=<key>` only when you
  intentionally want a brand-new protocol deployment with new addresses.
- Use `make deploy-contract CONTRACT=<name> NETWORK=<network> SOURCE=<key>` only
  for standalone debugging. It does not update `.deployed/<network>.env`, does
  not initialize dependencies, and does not wire the contract into the protocol.

## Upgrade Rules

- Use `make upgrade-contract CONTRACT=<name> NETWORK=<network> SOURCE=<key>` for
  normal in-place contract changes.
- Use `make upgrade-all NETWORK=<network> SOURCE=<key>` only when every contract
  listed in `UPGRADE_CONTRACTS` exposes the required upgrade entrypoint.
- Upgradeable contracts must implement an admin-gated function equivalent to:

```rust
pub fn upgrade(env: Env, new_wasm_hash: BytesN<32>) {
    let admin: Address = env.storage().instance().get(&InstanceKey::Admin).unwrap();
    admin.require_auth();
    env.deployer().update_current_contract_wasm(new_wasm_hash);
}
```

- Do not change storage keys, enum variant order, or stored value types in an
  upgrade unless you also write and test an explicit migration path.
- Keep initialization separate from upgrades. `initialize` should run once;
  `upgrade` should preserve existing instance and persistent storage.

## Address Files

Full deployments write:

```sh
.deployed/testnet.env
.deployed/mainnet.env
.deployed/local.env
```

Test token setup writes:

```sh
.deployed/tokens-testnet.env
```

Use `make addresses NETWORK=<network>` to inspect the active deployment before
running any upgrade.

## PR Checklist

Before opening a review request, confirm every item below. Reviewers will use this list to decide whether to merge or request changes.

### Scope

- [ ] The PR addresses **one logical change** — a single issue, bug fix, or tightly related set of concerns.
- [ ] No unrelated refactors, formatting fixes, or drive-by cleanups are included. Open a separate PR for those.
- [ ] Public function signatures and storage key names are backward-compatible unless an explicit migration path is documented and tested.

### Tests

- [ ] Every new or modified function has at least one test that covers the happy path.
- [ ] Every new or modified function that can revert has at least one `#[should_panic]` (or `try_*`) test that exercises the revert condition.
- [ ] `cargo test --workspace` passes locally with no failures or ignored tests introduced by this PR.

### Build

- [ ] `cargo check --workspace` produces zero errors.
- [ ] `cargo clippy --workspace -- -D warnings` produces zero warnings.
- [ ] `stellar contract build` completes successfully (wasm artefacts are produced, not committed).

### Documentation

- [ ] Public functions have a doc comment if their behaviour is non-obvious.
- [ ] If observable behaviour changes (new entrypoints, changed error codes, new storage keys), `README.md` is updated to match.
- [ ] New domain terms introduced by the PR are added to the [glossary](README.md#protocol-glossary).

### Storage Safety (for handler changes)

- [ ] No existing `#[contracttype]` enum has had variants reordered or removed — only appended.
- [ ] No existing persistent-storage value type has changed without an explicit migration.
- [ ] New handler state follows the **local persistent storage model** (see [Local Storage Policy](#local-storage-policy)).

### Upgrade Safety (for contracts with an `upgrade` entrypoint)

- [ ] The `upgrade` function follows the admin-gated pattern in [Upgrade Rules](#upgrade-rules).
- [ ] A test verifies that an unauthorized caller reverts and that storage is intact after a successful upgrade.

## Architectural Guidelines

### Local Storage Policy
When implementing or modifying handlers (e.g. deposit, withdrawal, order, liquidation, or ADL logic), follow the **local persistent storage model** (Issue #2). Transient request states and position records must be stored in the contract's own persistent storage using unique enum keys, rather than in the shared global `data_store`. This maintains Soroban rent (TTL) isolation, enforces strict access boundaries, and optimizes CPU instruction consumption.

### Contract Responsibility Matrix
Before introducing new contract types or modifying existing ones, consult the **Contract Responsibility Matrix** in [README.md](README.md#contract-responsibility-matrix) (Issue #4). Ensure all new code complies with the specified storage access rules, caller roles, and upgrade capabilities.

