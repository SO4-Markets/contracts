# Test Coverage Notes

The contract includes unit coverage for:

- 0 bps allocation routing full penalty to treasury.
- bps allocation splitting penalty between insurance fund and treasury.
- shortfall coverage where the fund is exhausted before the pool absorbs the remainder.

Full end-to-end integration with `liquidation_handler` should wire the helper calls into the successful liquidation path.
