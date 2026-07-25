# Implementation Notes

The helper keeps the issue #213 behavior small and auditable:

- bps split uses `amount * bps / 10_000`.
- 0 bps disables fund routing.
- shortfall draw uses `min(fund_balance, shortfall)`.
- the returned remainder is the amount still absorbed by the pool.
