//! Hardening: no panics on bad input, and serde round-trips for the wire types.

use nocturne::*;

fn u256(x: u64) -> Word {
    let mut w = [0u8; 32];
    w[24..].copy_from_slice(&x.to_be_bytes());
    w
}

fn sample_offer() -> Offer {
    let market = Market {
        chain_id: u256(1),
        midnight: [0x11; 20],
        loan_token: [0x22; 20],
        collateral_params: vec![CollateralParams {
            token: [0x33; 20],
            lltv: u256(860_000_000_000_000_000),
            liquidation_cursor: u256(1),
            oracle: [0x44; 20],
        }],
        maturity: u256(1_800_000_000),
        rcf_threshold: u256(1000),
        enter_gate: [0u8; 20],
        liquidator_gate: [0u8; 20],
    };
    Offer {
        market,
        buy: true,
        maker: [0x55; 20],
        start: u256(0),
        expiry: u256(2_000_000_000),
        tick: u256(8),
        group: u256(7),
        callback: [0u8; 20],
        callback_data: vec![1, 2, 3, 4],
        receiver_if_maker_is_seller: [0u8; 20],
        ratifier: [0xbb; 20],
        reduce_only: false,
        max_units: 1_000_000,
        max_assets: 0,
        continuous_fee_cap: u256(100),
    }
}

#[test]
fn tree_build_errors_instead_of_panicking() {
    // Non-power-of-two and empty return Err, not a panic.
    assert_eq!(
        OfferTree::build(vec![[0u8; 32]; 3]),
        Err(TreeError::NotPowerOfTwo(3))
    );
    assert_eq!(OfferTree::build(vec![]), Err(TreeError::NotPowerOfTwo(0)));
    // A valid power of two succeeds.
    assert!(OfferTree::build(vec![[0u8; 32]; 4]).is_ok());
}

#[test]
fn tree_proof_errors_instead_of_panicking_out_of_range() {
    let tree = OfferTree::build(vec![[0u8; 32]; 2]).unwrap();
    assert_eq!(
        tree.proof(2),
        Err(TreeError::LeafIndexOutOfRange {
            index: 2,
            leaves: 2,
        })
    );
}

#[test]
fn verify_leaf_rejects_oversized_proofs_without_panicking() {
    assert!(!verify_leaf(
        &[0u8; 32],
        &[0u8; 32],
        0,
        &vec![[0u8; 32]; MAX_TREE_HEIGHT + 1],
    ));
}

#[test]
fn tree_build_rejects_trees_taller_than_the_onchain_cap() {
    // HashLib.offerTreeTypeHash reverts TreeTooHigh above height 20, so 2^21 leaves must be
    // rejected up front rather than producing a tree no ratifier will ever accept.
    assert_eq!(
        OfferTree::build(vec![[0u8; 32]; 1 << (MAX_TREE_HEIGHT + 1)]),
        Err(TreeError::TooHigh(MAX_TREE_HEIGHT + 1))
    );
}

#[test]
fn offer_serde_roundtrips_and_preserves_hash() {
    let offer = sample_offer();
    let json = serde_json::to_string(&offer).unwrap();
    let back: Offer = serde_json::from_str(&json).unwrap();
    // The decoded offer hashes identically - serialization is lossless where it counts.
    assert_eq!(hash_offer(&offer), hash_offer(&back));
}

#[test]
fn sig_serde_roundtrips() {
    let sig = Sig {
        r: u256(111),
        s: u256(222),
        v: 27,
    };
    let json = serde_json::to_string(&sig).unwrap();
    let back: Sig = serde_json::from_str(&json).unwrap();
    assert_eq!(sig, back);
}
