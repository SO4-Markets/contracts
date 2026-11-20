//! Cross-check: assert that the pure-Rust encoding agrees with the committed
//! test vectors in scripts/key_vectors.json (issue #1168).
//!
//! This test does NOT use soroban_sdk — it reimplements the same
//! length-prefix + SHA-256 scheme in native std Rust so it can be run with
//! `cargo test -p gmx-keys` without a Soroban environment.  Any divergence
//! between this implementation and libs/keys/src/lib.rs is itself a bug and
//! should be fixed in lib.rs first, then regenerate the JSON with:
//!
//!   python3 scripts/compute_key.py <key_type> [args] > /dev/null  # sanity
//!   # re-run the generation script and commit the updated JSON

use sha2::{Digest, Sha256};
use std::path::PathBuf;

// ── encoding helpers (mirror libs/keys/src/lib.rs) ──────────────────────────

fn push_str(buf: &mut Vec<u8>, s: &str) {
    let b = s.as_bytes();
    let len = b.len() as u16;
    buf.extend_from_slice(&len.to_be_bytes());
    buf.extend_from_slice(b);
}

fn push_addr(buf: &mut Vec<u8>, addr: &str) {
    let b = addr.as_bytes();
    let len = b.len() as u16;
    buf.extend_from_slice(&len.to_be_bytes());
    buf.extend_from_slice(b);
}

fn push_bool(buf: &mut Vec<u8>, v: bool) {
    buf.push(if v { 1 } else { 0 });
}

fn push_raw32(buf: &mut Vec<u8>, hex: &str) {
    let raw = hex::decode(hex).expect("PNL_HEX must be valid hex");
    assert_eq!(raw.len(), 32, "pnl_type must be 32 bytes");
    buf.extend_from_slice(&raw);
}

fn sha256_hex(buf: &[u8]) -> String {
    let digest = Sha256::digest(buf);
    hex::encode(digest)
}

// ── key constructors ─────────────────────────────────────────────────────────

fn role_key(role: &str) -> String {
    let mut b = Vec::new();
    push_str(&mut b, role);
    sha256_hex(&b)
}

fn market_type_default() -> String {
    sha256_hex(b"DEFAULT")
}

fn max_pnl_factor_for_traders() -> String {
    let mut b = Vec::new();
    push_str(&mut b, "MAX_PNL_FACTOR_FOR_TRADERS");
    sha256_hex(&b)
}

fn max_pnl_factor_for_deposits() -> String {
    let mut b = Vec::new();
    push_str(&mut b, "MAX_PNL_FACTOR_FOR_DEPOSITS");
    sha256_hex(&b)
}

fn max_pnl_factor_for_withdrawals() -> String {
    let mut b = Vec::new();
    push_str(&mut b, "MAX_PNL_FACTOR_FOR_WITHDRAWALS");
    sha256_hex(&b)
}

fn max_pool_amount_key(market: &str, token: &str) -> String {
    let mut b = Vec::new();
    push_str(&mut b, "MAX_POOL_AMOUNT");
    push_addr(&mut b, market);
    push_addr(&mut b, token);
    sha256_hex(&b)
}

fn min_collateral_factor_key(market: &str) -> String {
    let mut b = Vec::new();
    push_str(&mut b, "MIN_COLLATERAL_FACTOR");
    push_addr(&mut b, market);
    sha256_hex(&b)
}

fn max_leverage_key(market: &str) -> String {
    let mut b = Vec::new();
    push_str(&mut b, "MAX_LEVERAGE");
    push_addr(&mut b, market);
    sha256_hex(&b)
}

fn max_open_interest_key(market: &str, is_long: bool) -> String {
    let mut b = Vec::new();
    push_str(&mut b, "MAX_OPEN_INTEREST");
    push_addr(&mut b, market);
    push_bool(&mut b, is_long);
    sha256_hex(&b)
}

fn borrowing_factor_key(market: &str, is_long: bool) -> String {
    let mut b = Vec::new();
    push_str(&mut b, "BORROWING_FACTOR");
    push_addr(&mut b, market);
    push_bool(&mut b, is_long);
    sha256_hex(&b)
}

fn borrowing_exponent_factor_key(market: &str, is_long: bool) -> String {
    let mut b = Vec::new();
    push_str(&mut b, "BORROWING_EXPONENT_FACTOR");
    push_addr(&mut b, market);
    push_bool(&mut b, is_long);
    sha256_hex(&b)
}

fn funding_factor_key(market: &str) -> String {
    let mut b = Vec::new();
    push_str(&mut b, "FUNDING_FACTOR");
    push_addr(&mut b, market);
    sha256_hex(&b)
}

fn funding_exponent_factor_key(market: &str) -> String {
    let mut b = Vec::new();
    push_str(&mut b, "FUNDING_EXPONENT_FACTOR");
    push_addr(&mut b, market);
    sha256_hex(&b)
}

fn funding_increase_factor_per_second_key(market: &str) -> String {
    let mut b = Vec::new();
    push_str(&mut b, "FUNDING_INCREASE_FACTOR_PER_SECOND");
    push_addr(&mut b, market);
    sha256_hex(&b)
}

fn funding_decrease_factor_per_second_key(market: &str) -> String {
    let mut b = Vec::new();
    push_str(&mut b, "FUNDING_DECREASE_FACTOR_PER_SECOND");
    push_addr(&mut b, market);
    sha256_hex(&b)
}

fn min_funding_factor_per_second_key(market: &str) -> String {
    let mut b = Vec::new();
    push_str(&mut b, "MIN_FUNDING_FACTOR_PER_SECOND");
    push_addr(&mut b, market);
    sha256_hex(&b)
}

fn max_funding_factor_per_second_key(market: &str) -> String {
    let mut b = Vec::new();
    push_str(&mut b, "MAX_FUNDING_FACTOR_PER_SECOND");
    push_addr(&mut b, market);
    sha256_hex(&b)
}

fn swap_impact_exponent_factor_key(market: &str) -> String {
    let mut b = Vec::new();
    push_str(&mut b, "SWAP_IMPACT_EXPONENT_FACTOR");
    push_addr(&mut b, market);
    sha256_hex(&b)
}

fn position_impact_exponent_factor_key(market: &str) -> String {
    let mut b = Vec::new();
    push_str(&mut b, "POSITION_IMPACT_EXPONENT_FACTOR");
    push_addr(&mut b, market);
    sha256_hex(&b)
}

fn min_market_tokens_for_first_deposit_key(market: &str) -> String {
    let mut b = Vec::new();
    push_str(&mut b, "MIN_MARKET_TOKENS_FOR_FIRST_DEPOSIT");
    push_addr(&mut b, market);
    sha256_hex(&b)
}

fn stable_price_key(token: &str) -> String {
    let mut b = Vec::new();
    push_str(&mut b, "STABLE_PRICE");
    push_addr(&mut b, token);
    sha256_hex(&b)
}

fn max_pnl_factor_for_adl_key(market: &str, is_long: bool) -> String {
    let mut b = Vec::new();
    push_str(&mut b, "MAX_PNL_FACTOR_FOR_ADL");
    push_addr(&mut b, market);
    push_bool(&mut b, is_long);
    sha256_hex(&b)
}

fn max_pnl_factor_key(pnl_hex: &str, market: &str, is_long: bool) -> String {
    let mut b = Vec::new();
    push_str(&mut b, "MAX_PNL_FACTOR");
    push_raw32(&mut b, pnl_hex);
    push_addr(&mut b, market);
    push_bool(&mut b, is_long);
    sha256_hex(&b)
}

fn position_fee_factor_key(market: &str, for_positive: bool) -> String {
    let mut b = Vec::new();
    push_str(&mut b, "POSITION_FEE_FACTOR");
    push_addr(&mut b, market);
    push_bool(&mut b, for_positive);
    sha256_hex(&b)
}

fn swap_fee_factor_key(market: &str, for_positive: bool) -> String {
    let mut b = Vec::new();
    push_str(&mut b, "SWAP_FEE_FACTOR");
    push_addr(&mut b, market);
    push_bool(&mut b, for_positive);
    sha256_hex(&b)
}

fn swap_impact_factor_key(market: &str, is_positive: bool) -> String {
    let mut b = Vec::new();
    push_str(&mut b, "SWAP_IMPACT_FACTOR");
    push_addr(&mut b, market);
    push_bool(&mut b, is_positive);
    sha256_hex(&b)
}

fn position_impact_factor_key(market: &str, is_positive: bool) -> String {
    let mut b = Vec::new();
    push_str(&mut b, "POSITION_IMPACT_FACTOR");
    push_addr(&mut b, market);
    push_bool(&mut b, is_positive);
    sha256_hex(&b)
}

fn keeper_public_key_key(index: u32) -> String {
    // prefix = sha256(push_str("KEEPER_PUBLIC_KEY"))
    let mut prefix_buf = Vec::new();
    push_str(&mut prefix_buf, "KEEPER_PUBLIC_KEY");
    let prefix = Sha256::digest(&prefix_buf);
    let mut b = prefix.to_vec();
    b.extend_from_slice(&index.to_be_bytes());
    sha256_hex(&b)
}

// ── hex helper (avoids adding the hex crate as a named dep) ──────────────────

mod hex {
    pub fn decode(s: &str) -> Result<Vec<u8>, ()> {
        if s.len() % 2 != 0 {
            return Err(());
        }
        (0..s.len() / 2)
            .map(|i| u8::from_str_radix(&s[i * 2..i * 2 + 2], 16).map_err(|_| ()))
            .collect()
    }

    pub fn encode(bytes: impl AsRef<[u8]>) -> String {
        bytes.as_ref().iter().map(|b| format!("{:02x}", b)).collect()
    }
}

// ── test ──────────────────────────────────────────────────────────────────────

fn load_vectors() -> (serde_json::Value, serde_json::Value) {
    // Walk up from the test binary's manifest dir to find the workspace root.
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let json_path = manifest_dir
        .parent() // libs/
        .unwrap()
        .parent() // workspace root
        .unwrap()
        .join("scripts/key_vectors.json");
    let content = std::fs::read_to_string(&json_path)
        .unwrap_or_else(|e| panic!("cannot read {}: {}", json_path.display(), e));
    let root: serde_json::Value = serde_json::from_str(&content).expect("invalid JSON");
    let meta = root["_meta"].clone();
    let vectors = root["vectors"].clone();
    (meta, vectors)
}

#[test]
fn rust_encoding_matches_committed_vectors() {
    let (meta, vectors) = load_vectors();

    let market  = meta["MARKET"].as_str().unwrap();
    let token_a = meta["TOKEN_A"].as_str().unwrap();
    let token_b = meta["TOKEN_B"].as_str().unwrap();
    let pnl_hex = meta["PNL_HEX"].as_str().unwrap();

    macro_rules! check {
        ($key:expr, $got:expr) => {{
            let expected = vectors[$key].as_str().unwrap_or_else(|| {
                panic!("vector '{}' missing from key_vectors.json", $key)
            });
            assert_eq!(
                $got, expected,
                "Rust/Python divergence for key '{}'\n  Rust     {}\n  JSON     {}",
                $key, $got, expected
            );
        }};
    }

    // role keys
    for role in ["ROLE_ADMIN","CONTROLLER","MARKET_KEEPER","ORDER_KEEPER",
                 "LIQUIDATION_KEEPER","ADL_KEEPER","FEE_KEEPER"] {
        check!(format!("role:{}", role), role_key(role));
    }

    // constant keys
    check!("max_pnl_factor_for_traders",    max_pnl_factor_for_traders());
    check!("max_pnl_factor_for_deposits",   max_pnl_factor_for_deposits());
    check!("max_pnl_factor_for_withdrawals",max_pnl_factor_for_withdrawals());
    check!("market_type_default",           market_type_default());

    // address keys
    check!("max_pool_amount:market,token_a",              max_pool_amount_key(market, token_a));
    check!("max_pool_amount:market,token_b",              max_pool_amount_key(market, token_b));
    check!("min_collateral_factor:market",                min_collateral_factor_key(market));
    check!("max_leverage:market",                         max_leverage_key(market));
    check!("funding_factor:market",                       funding_factor_key(market));
    check!("funding_exponent_factor:market",              funding_exponent_factor_key(market));
    check!("funding_increase_factor_per_second:market",   funding_increase_factor_per_second_key(market));
    check!("funding_decrease_factor_per_second:market",   funding_decrease_factor_per_second_key(market));
    check!("min_funding_factor_per_second:market",        min_funding_factor_per_second_key(market));
    check!("max_funding_factor_per_second:market",        max_funding_factor_per_second_key(market));
    check!("swap_impact_exponent_factor:market",          swap_impact_exponent_factor_key(market));
    check!("position_impact_exponent_factor:market",      position_impact_exponent_factor_key(market));
    check!("min_market_tokens_for_first_deposit:market",  min_market_tokens_for_first_deposit_key(market));
    check!("stable_price:token_a",                        stable_price_key(token_a));

    // bool keys
    check!("max_open_interest:market,long",           max_open_interest_key(market, true));
    check!("max_open_interest:market,short",          max_open_interest_key(market, false));
    check!("borrowing_factor:market,long",            borrowing_factor_key(market, true));
    check!("borrowing_factor:market,short",           borrowing_factor_key(market, false));
    check!("borrowing_exponent_factor:market,long",   borrowing_exponent_factor_key(market, true));
    check!("borrowing_exponent_factor:market,short",  borrowing_exponent_factor_key(market, false));
    check!("max_pnl_factor_for_adl:market,long",      max_pnl_factor_for_adl_key(market, true));
    check!("max_pnl_factor_for_adl:market,short",     max_pnl_factor_for_adl_key(market, false));
    check!("max_pnl_factor:long",                     max_pnl_factor_key(pnl_hex, market, true));
    check!("max_pnl_factor:short",                    max_pnl_factor_key(pnl_hex, market, false));
    check!("position_fee_factor:market,pos",          position_fee_factor_key(market, true));
    check!("position_fee_factor:market,neg",          position_fee_factor_key(market, false));
    check!("swap_fee_factor:market,pos",              swap_fee_factor_key(market, true));
    check!("swap_fee_factor:market,neg",              swap_fee_factor_key(market, false));
    check!("swap_impact_factor:market,pos",           swap_impact_factor_key(market, true));
    check!("swap_impact_factor:market,neg",           swap_impact_factor_key(market, false));
    check!("position_impact_factor:market,pos",       position_impact_factor_key(market, true));
    check!("position_impact_factor:market,neg",       position_impact_factor_key(market, false));

    // keeper key
    check!("keeper_public_key:0", keeper_public_key_key(0));
    check!("keeper_public_key:1", keeper_public_key_key(1));
}
