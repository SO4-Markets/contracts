# AGENTS.md

Operating rules for any AI agent making changes to this repository.

These are not suggestions. A change that does not satisfy the gate below is not finished,
regardless of how correct the diff looks.

---

## The one rule

**Never report work as complete until the full verification gate passes on your working tree.**

You must run it yourself. You may not infer that it passes because your change was small, because
the logic is obviously right, because CI is green, or because you only touched tests, comments, or
documentation. Run the commands. Read the output.

---

## The verification gate

Run in this order. Every step must exit zero.

```bash
make check          # cargo check --workspace
make lint           # cargo clippy --workspace -- -D warnings
cargo clippy --workspace --all-targets -- -D warnings
make test           # cargo test --workspace
make build          # stellar contract build  (all 23 contracts → WASM)
```

Two of these are not in the Makefile by accident of history, and matter most:

- **`cargo clippy --workspace --all-targets`** — plain `make lint` does *not* compile `#[cfg(test)]`
  modules or `tests/` directories. Without `--all-targets`, test code is invisible to static checking
  and can rot for weeks while every check reports green.
- **`make build`** — `cargo check` and `cargo test` compile for the host target. Contract-macro
  constraints (below) are only enforced when building the actual contract, so a workspace that checks
  and tests cleanly can still be undeployable.

If a step fails, fix it or stop and report it. Do not proceed past a failing step and do not
describe the remaining steps as passing.

### Scoping when the full gate is slow

Full builds here are slow. While iterating, narrow with `make test-one PACKAGE=<crate>` or
`cargo check -p <crate>`. That is fine mid-task. It is not fine as the final answer — run the
complete gate before reporting done.

---

## Why this exists

On 2026-07-25 a single 35-character contract function name landed on `main`. It broke the workspace
build for over a month while merges continued on top of it. Underneath it sat four more crates whose
test code no longer compiled, so 301 tests written during that period had never once executed.

Every one of those failures would have been caught by the gate above. None of them were caught by CI.

---

## Soroban constraints that break the build silently

These are enforced by contract macros at build time, not by `cargo check`. Violating them produces a
compile error only during `make build`, or — worse — a runtime panic on-chain.

| Constraint | Limit | Notes |
|---|---|---|
| Contract function name | **32 characters** | Current longest in-repo is 30. Two characters of headroom. Count before you name. |
| `symbol_short!` literal | **9 characters** | Longer literals will not compile. |
| Ledger entries per invocation | ~40 reads | Several handlers deliberately skip work to stay under budget — read the `NOTE:` comments before "fixing" an omission. |
| WASM size | Per-contract budget | Enforced by `.github/workflows/wasm-size.yml`. Check with `make wasm-sizes` after `make build-release`. |

Before naming any new public contract function, count the characters.

---

## Deployability

A change is only done if all 23 contracts in `CONTRACTS` (see `mx/common.mk`) still build and deploy.

- **Adding a contract**: it must be added to the `CONTRACTS` list in `mx/common.mk`, or it will never
  be built, deployed, or upgraded — and nothing will warn you.
- **Changing an `initialize` signature**: every call site changes with it, including those inside
  `#[cfg(test)]` modules of *other* crates. This is exactly how three test modules were left broken.
  Grep the whole workspace, not just the crate you are editing.
- **Changing any public entrypoint**: regenerate bindings and say so explicitly in your summary.
  Removing or renaming an entrypoint is a breaking change for the frontend — call it out, never
  bury it.
- **Changing a stored type's shape**: persisted data does not migrate itself. State the upgrade
  implication in your summary, even for temporary storage.
- **Verify locally** with `make build`, and for deploy-path changes `make deploy-all NETWORK=testnet SOURCE=<key>`
  followed by `make testnet-smoke`.

---

## What CI does and does not cover

Do not treat a green CI run as evidence your change is sound. As currently configured:

| Workflow | Runs | Covers |
|---|---|---|
| `lint.yml` | `cargo fmt --check`, `cargo clippy --workspace` | Library targets only — **not test code** |
| `wasm-size.yml` | `cargo build --target ... --release` | Build + size budget |
| `auth-audit.yml` | `docs/auth-audit.md` sync check | Docs only |

All three trigger on `pull_request` only. **Nothing verifies `main` after a merge**, so two
individually-green PRs can merge into a broken branch with no signal. There is no `cargo test` job at
all.

If you are asked to improve CI, the highest-value changes are: add `push: [main]` triggers, add
`--all-targets` to the clippy invocation, and add a `cargo test --workspace` job.

---

## Tests

- A test that passes without exercising the behaviour it names is worse than no test — it converts an
  open question into false confidence. Before trusting an existing test as proof, check that its
  assertion can actually fail.
- When you fix a bug, add a test that fails before the fix and passes after. Verify both directions.
- Never adjust a test to match broken behaviour to make a suite go green. If a test and the code
  disagree, determine which one is wrong and say so.
- `make test-snap` rewrites snapshot files. Only run it when snapshot churn is the intended change,
  and review the diff — snapshots are large and easily hide real regressions.

---

## Reporting

- State plainly which gate steps you ran and what they returned. If you skipped one, say which and why.
- If tests fail, show the output. Do not summarise a failure as a caveat.
- If you could not finish part of the task, finish everything else and state exactly what is
  outstanding. Scaling work down is the maintainer's decision, not yours.
- Do not claim a contract is deployable unless `make build` succeeded in your working tree.

---

## Repository map

| Path | Contents |
|---|---|
| `contracts/*` | Deployable Soroban contracts (23, listed in `mx/common.mk`) |
| `libs/*` | Shared logic — `math`, `keys`, `types`, position/pricing/swap/market utils |
| `tests/` | Cross-contract integration tests (`gmx-integration-tests`) |
| `mx/*.mk` | Make workflows: build, test, deploy, upgrade, tokens, oracle |
| `docs/` | Architecture, security, TTL strategy, auth audit |
| `.agents/skills/` | Stellar/Soroban reference material |

Money-path code — `libs/increase_position_utils`, `libs/decrease_position_utils`,
`libs/pricing_utils`, `libs/market_utils` — carries real accounting invariants. Fees, `pool_amount`,
`collateral_sum`, and the funding/borrowing snapshots on a position are coupled across the increase
and decrease paths. Changing one side without the other has already caused double-charging bugs here.
Read both paths before editing either.
