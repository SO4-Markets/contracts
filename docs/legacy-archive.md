# Legacy GMX-Style Implementation

The GMX-style synthetic order-book DEX was retired when the repository was reset for the SO4 CLOB
rewrite. Active development follows the CLOB architecture in [protocol.md](protocol.md). The legacy
code is not part of `main` and must not be merged back into it.

| Item | Value |
|------|-------|
| Archive branch | `gmx-synthetic-order-book-dex` |
| Archive point | `5436392dcfdfdeb3a8f4d7724a73031d6aeee8c3` (`Merge pull request #1088`) |
| Reset merged by | `280f268` (`Merge pull request #1090`) |

The archive point is the last `main` commit before the reset. The archive branch points at that
commit and holds the full legacy tree: contracts, scripts, tests, docs, and `.github/` workflows.

To inspect the legacy implementation:

```bash
git fetch origin
git checkout gmx-synthetic-order-book-dex
```
