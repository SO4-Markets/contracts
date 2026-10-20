//! Canonical market identity (#880).
//!
//! A market is identified by its unordered asset pair plus a version. The same
//! two assets always derive the same [`MarketId`] whichever order they are
//! supplied in, so a market cannot be created twice with reversed base and
//! quote. The user-facing orientation (which asset is the base) belongs to the
//! market definition, not to the id.
//!
//! # Preimage (consensus-critical)
//!
//! ```text
//! MarketId = sha256( MARKET_ID_DOMAIN || version_be_u32 || xdr(lo) || xdr(hi) )
//! ```
//!
//! where `lo <= hi` under [`Address`] ordering and `xdr(a)` is the XDR encoding
//! of the address as a Soroban value. SDKs must reproduce this byte layout
//! exactly. Native XLM is referenced by its Stellar Asset Contract address like
//! every other asset.

use soroban_sdk::{xdr::ToXdr, Address, Bytes, BytesN, Env};

/// Domain separator mixed into every market id.
pub const MARKET_ID_DOMAIN: &[u8] = b"SO4:clob:market-id:v1";

/// Identifier of a market, derived from its asset pair and version.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MarketId(BytesN<32>);

impl MarketId {
    /// Derives the id for the market trading `base` against `quote`.
    ///
    /// Returns an error if `base == quote`. Otherwise the result is identical
    /// for `(base, quote)` and `(quote, base)` at the same `version`.
    pub fn derive(
        env: &Env,
        base: &Address,
        quote: &Address,
        version: u32,
    ) -> Result<Self, &'static str> {
        validate_pair(base, quote)?;
        let (lo, hi) = canonical_pair(base, quote);

        let mut preimage = Bytes::from_slice(env, MARKET_ID_DOMAIN);
        preimage.append(&Bytes::from_array(env, &version.to_be_bytes()));
        preimage.append(&lo.clone().to_xdr(env));
        preimage.append(&hi.clone().to_xdr(env));

        Ok(Self(env.crypto().sha256(&preimage).into()))
    }

    /// Raw 32-byte id, for events, storage keys and SDK interop.
    pub fn to_bytes(&self) -> BytesN<32> {
        self.0.clone()
    }
}

/// Rejects a market whose base and quote are the same asset.
pub fn validate_pair(base: &Address, quote: &Address) -> Result<(), &'static str> {
    if base == quote {
        return Err("base and quote must differ");
    }
    Ok(())
}

/// Orders the pair so the lower address comes first. Used only for hashing.
fn canonical_pair<'a>(base: &'a Address, quote: &'a Address) -> (&'a Address, &'a Address) {
    if base <= quote {
        (base, quote)
    } else {
        (quote, base)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use soroban_sdk::testutils::Address as _;

    /// Registers a Stellar Asset Contract, the form every listed token takes.
    fn sac(env: &Env) -> Address {
        let admin = Address::generate(env);
        env.register_stellar_asset_contract_v2(admin).address()
    }

    #[test]
    fn same_inputs_derive_same_id() {
        let env = Env::default();
        let base = sac(&env);
        let quote = sac(&env);

        let first = MarketId::derive(&env, &base, &quote, 1).unwrap();
        let second = MarketId::derive(&env, &base, &quote, 1).unwrap();

        assert_eq!(first, second);
    }

    #[test]
    fn reversed_pair_derives_same_id() {
        let env = Env::default();
        let xlm = sac(&env);
        let usdc = sac(&env);

        let forward = MarketId::derive(&env, &xlm, &usdc, 1).unwrap();
        let reversed = MarketId::derive(&env, &usdc, &xlm, 1).unwrap();

        assert_eq!(forward, reversed);
    }

    #[test]
    fn identical_base_and_quote_rejected() {
        let env = Env::default();
        let token = sac(&env);

        assert!(MarketId::derive(&env, &token, &token, 1).is_err());
    }

    #[test]
    fn version_changes_id() {
        let env = Env::default();
        let base = sac(&env);
        let quote = sac(&env);

        let v1 = MarketId::derive(&env, &base, &quote, 1).unwrap();
        let v2 = MarketId::derive(&env, &base, &quote, 2).unwrap();

        assert_ne!(v1, v2);
    }

    #[test]
    fn different_pairs_derive_different_ids() {
        let env = Env::default();
        let a = sac(&env);
        let b = sac(&env);
        let c = sac(&env);

        let ab = MarketId::derive(&env, &a, &b, 1).unwrap();
        let ac = MarketId::derive(&env, &a, &c, 1).unwrap();

        assert_ne!(ab, ac);
    }

    #[test]
    fn sac_and_account_addresses_are_accepted() {
        let env = Env::default();
        let token = sac(&env);
        let account = Address::generate(&env);

        let id = MarketId::derive(&env, &token, &account, 1).unwrap();
        let swapped = MarketId::derive(&env, &account, &token, 1).unwrap();

        assert_eq!(id, swapped);
        assert_eq!(id.to_bytes().to_array().len(), 32);
    }
}
