# Routing Summary

- Penalty split: `insurance_share = penalty * allocation_bps / 10_000`.
- Treasury receives the remainder.
- Reserve draw: `covered = min(fund_balance, shortfall)`.
- Pool absorbs only the uncovered remainder.
