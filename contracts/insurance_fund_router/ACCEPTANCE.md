# Acceptance Criteria Mapping

- Successful liquidation routes percentage to insurance fund: implemented in `route_liquidation_penalty`.
- Shortfall first draws from insurance fund before hitting pool: implemented in `cover_shortfall`.
- Allocation of 0 bps disables insurance routing: `route_liquidation_penalty` computes 0 share and skips fund transfer.
- Fund absorbs shortfall and pool remains unchanged: covered by helper semantics where `pool_remainder == 0`.
- Shortfall exceeds fund: covered by helper semantics where `covered_by_fund == fund_balance` and `pool_remainder > 0`.

The remaining integration step is to call these helpers from the existing liquidation execution flow.
