# Liquidation Handler Integration Plan

This branch adds the insurance-fund accounting and transfer contract required for issue #213.

To wire it into `liquidation_handler` fully:

1. Store each market's insurance fund address with `configure_insurance_fund`.
2. Store each market's insurance fund allocation bps with `configure_insurance_fund`.
3. After a successful liquidation computes a liquidation penalty, call `route_liquidation_penalty`.
4. When a liquidation creates a shortfall, call `cover_shortfall` before charging the market pool.
5. Charge only `pool_remainder` to the pool after the fund draw.

This preserves the 0 bps path: when allocation is 0, the helper skips the insurance transfer and routes the full penalty to treasury.
