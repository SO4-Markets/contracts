# Insurance Fund Router

Implements the core accounting and transfer helpers for issue #213.

## Entry points

- `configure_insurance_fund(data_store, caller, market, fund, allocation_bps)`
- `route_liquidation_penalty(data_store, market, token, source, treasury, liquidation_penalty)`
- `cover_shortfall(data_store, market, token, pool, shortfall_amount)`
- `preview_penalty_split(data_store, market, liquidation_penalty)`

## Integration notes

The existing liquidation flow can call `route_liquidation_penalty` after a successful liquidation and `cover_shortfall` before charging uncovered losses to the pool.

Allocation of `0` bps keeps current behaviour by routing the full penalty to treasury and skipping insurance-fund transfer.
