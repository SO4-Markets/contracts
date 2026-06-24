# Design Notes

The router is intentionally separate so liquidation accounting can adopt the reserve path without altering unrelated handler state. This also makes the 0 bps no-regression path explicit.
