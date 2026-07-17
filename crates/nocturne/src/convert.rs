//! Conversions between the 32-byte EIP-712 `Word` and typed integers.
//!
//! The wire structs (`Offer`, `Market`) store `uint256` fields as big-endian `Word`s so the
//! hashing path is a byte-for-byte mirror of the contract. Everything above it (builders,
//! simulation) works in `U256`, so these helpers bridge the two.

use crate::Word;

/// 256-bit unsigned integer - the same `ruint` type alloy / reth / foundry use.
pub use ruint::aliases::U256;

/// A big-endian `uint256` word from a `U256`.
#[inline]
pub fn u256_to_word(x: U256) -> Word {
    x.to_be_bytes::<32>()
}

/// A `U256` from a big-endian `uint256` word.
#[inline]
pub fn word_to_u256(w: &Word) -> U256 {
    U256::from_be_bytes(*w)
}

/// A big-endian `uint256` word holding `x`.
#[inline]
pub fn word_from_u128(x: u128) -> Word {
    let mut w = [0u8; 32];
    w[16..].copy_from_slice(&x.to_be_bytes());
    w
}

/// A big-endian `uint256` word holding `x`.
#[inline]
pub fn word_from_u64(x: u64) -> Word {
    let mut w = [0u8; 32];
    w[24..].copy_from_slice(&x.to_be_bytes());
    w
}

/// `Some(v)` if the word fits in a `u128` (top 16 bytes zero), else `None`.
#[inline]
pub fn word_to_u128(w: &Word) -> Option<u128> {
    if w[..16].iter().any(|&b| b != 0) {
        return None;
    }
    let mut b = [0u8; 16];
    b.copy_from_slice(&w[16..]);
    Some(u128::from_be_bytes(b))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrips() {
        for x in [0u128, 1, 1000, u64::MAX as u128, u128::MAX] {
            assert_eq!(word_to_u128(&word_from_u128(x)), Some(x));
            assert_eq!(word_to_u256(&word_from_u128(x)), U256::from(x));
        }
        for x in [0u64, 1, 31337, u64::MAX] {
            assert_eq!(word_to_u128(&word_from_u64(x)), Some(x as u128));
        }
    }

    #[test]
    fn u256_word_roundtrip() {
        let big = U256::from(u128::MAX) * U256::from(1_000_000u64);
        assert_eq!(word_to_u256(&u256_to_word(big)), big);
    }

    #[test]
    fn word_to_u128_rejects_oversized() {
        // A value with a high byte set does not fit in u128.
        let big = u256_to_word(U256::from(u128::MAX) + U256::from(1u64));
        assert_eq!(word_to_u128(&big), None);
    }
}
