# Security Assumptions

- `configure_insurance_fund` relies on `data_store` controller/admin checks through its own setter functions.
- `route_liquidation_penalty` requires the penalty source to authorize transfers.
- `cover_shortfall` assumes the configured fund address can authorize fund-to-pool transfers, or that a future fund contract exposes a controlled draw function.
- Allocation bps is capped at 10,000.
