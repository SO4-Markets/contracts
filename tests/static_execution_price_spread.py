from pathlib import Path


INCREASE = Path("libs/increase_position_utils/src/lib.rs").read_text()
DECREASE = Path("libs/decrease_position_utils/src/lib.rs").read_text()


def test_increase_uses_worst_case_spread_side():
    assert "pick_price_for_pnl(p.is_long, true)" in INCREASE
    assert "let index_price = p.index_token_price.mid_price();" not in INCREASE


def test_decrease_uses_inverse_spread_side():
    assert "pick_price_for_pnl(p.is_long, false)" in DECREASE
    assert "let index_price_mid = p.index_token_price.mid_price();" not in DECREASE