# Handler Wiring Notes

Suggested integration points:

1. After liquidation penalty calculation: call `route_liquidation_penalty`.
2. Before recording a market pool deficit: call `cover_shortfall`.
3. Record only the returned `pool_remainder` as pool loss.
