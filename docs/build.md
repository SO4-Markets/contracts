# Build

## Contract size budget

Soroban contracts are deployed as WASM, and smaller binaries are cheaper to upload, easier to keep within network limits, and less likely to regress when dependencies or generated bindings change. Size tracking keeps those changes visible during review.

Current optimized contract sizes:

<!-- size-table:start -->
| Contract | WASM size (bytes) | Δ from baseline |
| --- | ---: | ---: |
| adl_handler | 15715 | - |
| data_store | 11405 | - |
| deposit_handler | 18273 | - |
| deposit_vault | 3081 | - |
| exchange_router | 22672 | - |
| fee_handler | 9315 | - |
| liquidation_handler | 15471 | - |
| market_factory | 11640 | - |
| market_token | 7212 | - |
| oracle | 13008 | - |
| order_handler | 42897 | - |
| order_vault | 3604 | - |
| reader | 37622 | - |
| referral_storage | 6727 | - |
| role_store | 5393 | - |
| test_faucet | 4202 | - |
| test_token | 6750 | - |
| withdrawal_handler | 14824 | - |
| withdrawal_vault | 2763 | - |
<!-- size-table:end -->

The baseline is stored in `scripts/size_baseline.json` and must be updated manually after intentional size changes by running:

```bash
make update-size-baseline
```
