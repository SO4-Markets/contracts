# CLOB Implementation Status

> **Last updated:** 2026-11-19
> **Keep this file current.** It is the single authoritative record of what
> exists, what compiles, and what must land before any other CLOB issue can
> make forward progress.

---

## What exists in this repository

| Artifact | Lines | Cargo.toml | Workspace member | Tests |
|---|---|---|---|---|
| `libs/clob_settlement_core` | ~4 791 | ✗ none | ✗ | 158 unit tests — **never run** |
| `libs/clob_client` | ~347 | ✓ | ✗ not listed | 4 tests — **never run** |
| `tests/clob_model.rs` | ~1 894 | — | via `tests/` | ✓ run with `cargo test` |
| `tests/clob_properties.rs` | — | — | via `tests/` | ✓ run with `cargo test` |
| `libs/settlement_invariants` | — | ✓ | ✓ | nothing to point at |
| `libs/rollback_harness` | — | ✓ | ✓ | nothing to point at |
| `docs/` | 13 CLOB docs | — | — | 9 describe the above as implemented |

**There is no CLOB contract.** `contracts/` contains the 23 legacy GMX-style
perps contracts. `tests/clob_model.rs:3` is the only file that states this
explicitly (in a file comment).

---

## Known compile errors in `libs/clob_settlement_core`

The library has no `Cargo.toml` and is not reachable by `cargo build`. When a
manifest is added, the following must be fixed before the library compiles:

1. Four of eight source files are not declared as modules in `lib.rs` (or
   equivalent root) — they are dead code from the compiler's perspective.
2. Five independent compile errors exist across the declared sources.

Both points must be resolved as part of adding the manifest (see **Priority
queue** below).

---

## Arithmetic spec drift

`tests/clob_model.rs` and `tests/clob_properties.rs` test two in-memory
engines against each other and against an arithmetic spec. That spec
contradicts `libs/clob_settlement_core` on three points:

| Point | `clob_model` / spec | `clob_settlement_core` |
|---|---|---|
| Fee scale | spec value | different constant |
| Maker rounding | spec behaviour | different behaviour |
| Tick ceiling | spec formula | different formula |

Before any on-chain CLOB contract is written, one of the two must be declared
canonical and the other brought into alignment.

---

## Priority queue — what must land first

Every open CLOB issue in this repository is downstream of these two steps.
Nothing else can be unblocked until they are done.

1. **Add `Cargo.toml` to `libs/clob_settlement_core`** and add it to the
   workspace `members` list in the root `Cargo.toml`. This is the single
   largest blocker: without it the library cannot be compiled, the 158 tests
   cannot run, and no other library can depend on it.

2. **Fix the five compile errors and declare the missing four modules** in the
   same PR as step 1, so the library compiles and `cargo test -p
   clob_settlement_core` passes on the first CI run.

After those two steps the following become unblocked:

- Add `libs/clob_client` to workspace `members`
- Resolve the arithmetic spec drift (pick one spec, update the other)
- Wire `libs/settlement_invariants` and `libs/rollback_harness` to real code
- Write the CLOB contract under `contracts/`

---

## How to keep this file current

- When a PR adds `Cargo.toml` to `clob_settlement_core`, update the table row
  and tick the **Priority queue** item.
- When compile errors are fixed, remove them from the list above.
- When the arithmetic drift is resolved, update the **Arithmetic spec drift**
  table to reflect the chosen canonical spec.
- When a CLOB contract is added under `contracts/`, add a row to the
  **What exists** table and remove the "no CLOB contract" callout.
