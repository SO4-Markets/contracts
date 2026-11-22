# [CLOB-139 / CLOB-140] Worst-Case Market Shapes & Protocol Limit Tuning

## Overview
This document records synthetic worst-case market stress analysis and protocol limit gating rules.

### CLOB-139: Worst-Case Market Shapes Resource Usage
- **Deep Book Multi-Fill Sweeps**: Bounded by `MAX_MATCHES_PER_CALL = 50`.
- **Alternating Price Levels**: Validated to fit within transaction CPU budget (under 10M instructions).
- **Expired Head Orders**: Swept incrementally with bounded deletion overhead.
- **Max Batch Placement**: Validated against memory footprint under 2MB.

### CLOB-140: Limit Increase Gating Policy
- No protocol constant or limit may be increased without corresponding benchmark evidence added to `docs/benchmarks/resource-baseline.json`.
- State shapes with high cleanup overhead must require explicit sweeper incentives or caller-paid gas headroom.
