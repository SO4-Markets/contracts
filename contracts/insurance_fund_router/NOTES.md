# Notes

This implementation keeps the fund routing isolated in a helper contract. That avoids changing existing position/order storage layout while providing the exact split and shortfall calculations required by issue #213.

A follow-up patch can inject these calls directly into `liquidation_handler` once the liquidation penalty and shortfall values are exposed at the handler boundary.
