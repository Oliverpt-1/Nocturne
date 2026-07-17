//! End-to-end parity against the real `EcrecoverRatifier`.
//!
//! Unlike `parity.rs` (which checks only the typehash constants), this reconstructs a concrete
//! 4-offer tree and asserts that the Rust leaf hash, Merkle root, signed digest, and signer
//! recovery all match what the on-chain contract computes — and that the signature the contract
//! accepted also passes `nocturne::verify`.
//!
//! The expected values are produced by `fixtures/GenEndToEnd.t.sol`, which drives the actual
//! `EcrecoverRatifier.isRatified` from the Midnight contracts repo (rev
//! f47568c9e45a9b70830b82a130b47393dcafec33) and prints them. See that file's header to
//! regenerate. This test bakes the constants so `cargo test` stays standalone (same style as the
//! typehash constants in `parity.rs`).

use nocturne::*;

// ---- constants emitted by fixtures/GenEndToEnd.t.sol ----
const CHAIN_ID: u64 = 31337;
const MAKER: &str = "0xe05fcC23807536bEe418f142D19fa0d21BB0cfF7";
const RATIFIER: &str = "0x5615dEB798BB3E4dFa0139dFa1b3D433Cc23b72f";
const EXPECT_LEAF0: &str = "0x02b3aedf86dfe131c336f209c1261d12f3e8e7d72ec1c4ea4107eb050588b909";
const EXPECT_ROOT: &str = "0xbeb2926c3ab992aba1c0191c33a787439da9c162d021e4fa032a1702df20a884";
const EXPECT_DIGEST: &str = "0x25079c430938d0120f7df1ae363f75c2012067a7917798f068986ce80adaaf38";
const SIG_R: &str = "0xcbfb77f1a356e0e45cc231fc776fb052fa133a30f05428c89201df9271aad129";
const SIG_S: &str = "0x0d95bc2a6c1bb949437f7d74f9f14ac13978a470aab331ed3067216dce445bf7";
const SIG_V: u8 = 27;

fn hx32(s: &str) -> Word {
    let s = s.trim_start_matches("0x");
    let mut w = [0u8; 32];
    for i in 0..32 {
        w[i] = u8::from_str_radix(&s[2 * i..2 * i + 2], 16).unwrap();
    }
    w
}

fn hx20(s: &str) -> Address {
    let s = s.trim_start_matches("0x");
    let mut a = [0u8; 20];
    for i in 0..20 {
        a[i] = u8::from_str_radix(&s[2 * i..2 * i + 2], 16).unwrap();
    }
    a
}

fn u256(x: u64) -> Word {
    let mut w = [0u8; 32];
    w[24..].copy_from_slice(&x.to_be_bytes());
    w
}

/// Byte-for-byte reconstruction of `makeOffer(i)` from the Solidity fixture.
fn make_offer(i: u64) -> Offer {
    let market = Market {
        chain_id: u256(CHAIN_ID),
        midnight: hx20("0x1111111111111111111111111111111111111111"),
        loan_token: hx20("0x2222222222222222222222222222222222222222"),
        collateral_params: vec![CollateralParams {
            token: hx20("0x3333333333333333333333333333333333333333"),
            lltv: u256(860_000_000_000_000_000),
            liquidation_cursor: u256(1),
            oracle: hx20("0x4444444444444444444444444444444444444444"),
        }],
        maturity: u256(1_800_000_000),
        rcf_threshold: u256(1000),
        enter_gate: [0u8; 20],
        liquidator_gate: [0u8; 20],
    };
    Offer {
        market,
        buy: i % 2 == 0,
        maker: hx20(MAKER),
        start: u256(0),
        expiry: u256(2_000_000_000),
        tick: u256(i),
        group: u256(i),
        callback: [0u8; 20],
        callback_data: Vec::new(),
        receiver_if_maker_is_seller: [0u8; 20],
        ratifier: hx20(RATIFIER),
        reduce_only: false,
        max_units: 1_000_000 + i as u128,
        max_assets: 0,
        continuous_fee_cap: u256(0),
    }
}

#[test]
fn leaf_root_digest_match_the_ratifier() {
    let offers: Vec<Offer> = (0..4).map(make_offer).collect();
    let leaves: Vec<Word> = offers.iter().map(hash_offer).collect();
    let tree = OfferTree::build(leaves.clone()).unwrap();

    assert_eq!(leaves[0], hx32(EXPECT_LEAF0), "leaf0 hash mismatch vs HashLib.hashOffer");
    assert_eq!(tree.root(), hx32(EXPECT_ROOT), "root mismatch vs HashLib tree");

    let digest = tree_digest(tree.root(), tree.height(), u256(CHAIN_ID), &hx20(RATIFIER));
    assert_eq!(digest, hx32(EXPECT_DIGEST), "digest mismatch vs EcrecoverRatifier assembly");
}

#[test]
fn recovered_signer_is_the_maker() {
    let offers: Vec<Offer> = (0..4).map(make_offer).collect();
    let tree = OfferTree::build(offers.iter().map(hash_offer).collect()).unwrap();
    let digest = tree_digest(tree.root(), tree.height(), u256(CHAIN_ID), &hx20(RATIFIER));

    let sig = Sig { r: hx32(SIG_R), s: hx32(SIG_S), v: SIG_V };
    assert_eq!(recover(&digest, &sig), Some(hx20(MAKER)), "ecrecover must return the maker");
}

#[test]
fn contract_accepted_signature_passes_verify() {
    // This is the exact `(sig, root, leafIndex, proof)` the on-chain `isRatified` accepted.
    let offers: Vec<Offer> = (0..4).map(make_offer).collect();
    let tree = OfferTree::build(offers.iter().map(hash_offer).collect()).unwrap();
    let sig = Sig { r: hx32(SIG_R), s: hx32(SIG_S), v: SIG_V };

    assert!(
        verify(&offers[0], &tree.root(), 0, &tree.proof(0), &sig, u256(CHAIN_ID), &hx20(RATIFIER), &hx20(MAKER)),
        "verify must accept the signature the ratifier accepted"
    );
}
