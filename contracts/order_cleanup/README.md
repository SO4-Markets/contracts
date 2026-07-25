# Order Cleanup

Implements issue #214 as a small helper contract around the existing `order_handler` storage and cancellation flow.

## What it adds

- `set_order_expiry(...)` to configure timeout by order type.
- `cancel_expired_order(...)` callable by anyone after the timeout.
- `preview_expired_order(...)` for read-only status checks.
- `record_manual_refund(...)` as an audit event for admin/manual recovery when an order record is already gone.

## Integration requirement

Grant this helper contract the `ORDER_KEEPER` role so it can call `order_handler.cancel_order` after it verifies expiry. The existing handler then refunds collateral and removes the order from indexes.

## Default

Unset expiry uses `2880`, matching the requested default.
