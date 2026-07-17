//! Parity of the `Authorization` (hot-key delegation) signing against the real
//! `EcrecoverAuthorizer`.
//!
//! The expected values are produced by `fixtures/GenAuthorize.t.sol`, which drives the actual
//! `EcrecoverAuthorizer.setIsAuthorized` from the Midnight contracts repo (rev
//! f47568c9e45a9b70830b82a130b47393dcafec33) with a fixed authorizer key and asserts the
//! contract accepts the signature — then prints them. See that file's header to regenerate.
//! The constants are baked so `cargo test` stays standalone.

use k256::ecdsa::SigningKey;
use nocturne::*;

// ---- constants emitted by fixtures/GenAuthorize.t.sol ----
const CHAIN_ID: u64 = 31337;
const AUTHORIZER_CONTRACT: &str = "0x2e234DAe75C793f67A35089C9d99245E1C58470b";
const AUTHORIZER: &str = "0xe05fcC23807536bEe418f142D19fa0d21BB0cfF7";
const AUTHORIZED: &str = "0x2222222222222222222222222222222222222222";
const IS_AUTHORIZED: bool = true;
const NONCE: u64 = 0;
const DEADLINE: u64 = 2_000_000_000;
const EXPECT_HASHSTRUCT: &str = "0x02fb240d0d7687f2eed977c66ad50dbca50f6a99c90ef30ba355c4c0617d5a54";
const EXPECT_DIGEST: &str = "0xdb54bbfe5a0580f55767dd4d3ce8e721427fe7b01985c65f0fa9179fa3f045a9";
const SIG_R: &str = "0xec7d9f6dc496c805eff6053c28221ba98d85aba16d4a418b6d29e513366311fd";
const SIG_S: &str = "0x0711d5de9b77f131e3825ca507e23ee9a4c5f88a0739f881d0d475abf073e3e9";
const SIG_V: u8 = 28;

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

fn u256_word(x: u64) -> Word {
    let mut w = [0u8; 32];
    w[24..].copy_from_slice(&x.to_be_bytes());
    w
}

/// Byte-for-byte reconstruction of `makeAuthorization()` from the Solidity fixture.
fn fixture_authorization() -> Authorization {
    Authorization::new(
        hx20(AUTHORIZER),
        hx20(AUTHORIZED),
        IS_AUTHORIZED,
        U256::from(NONCE),
        U256::from(DEADLINE),
    )
}

#[test]
fn typehash_matches_onchain_constant() {
    assert_eq!(
        authorization_typehash(),
        AUTHORIZATION_TYPEHASH,
        "computed typehash must equal the baked on-chain constant"
    );
}

#[test]
fn hash_struct_matches_the_authorizer() {
    let a = fixture_authorization();
    assert_eq!(
        hash_authorization(&a),
        hx32(EXPECT_HASHSTRUCT),
        "hashStruct mismatch vs EcrecoverAuthorizer.setIsAuthorized"
    );
}

#[test]
fn digest_matches_the_authorizer() {
    let a = fixture_authorization();
    let digest = authorization_digest(&a, u256_word(CHAIN_ID), &hx20(AUTHORIZER_CONTRACT));
    assert_eq!(digest, hx32(EXPECT_DIGEST), "digest mismatch vs EcrecoverAuthorizer assembly");
}

#[test]
fn recovered_signer_is_the_authorizer() {
    let a = fixture_authorization();
    let sig = Sig { r: hx32(SIG_R), s: hx32(SIG_S), v: SIG_V };
    assert_eq!(
        recover_authorization(&a, u256_word(CHAIN_ID), &hx20(AUTHORIZER_CONTRACT), &sig),
        Some(hx20(AUTHORIZER)),
        "recover must return the authorizer the contract recovered"
    );
}

#[test]
fn sign_authorization_round_trip() {
    // Self-contained: sign with a fresh key and confirm the recovered signer is that key.
    let sk = SigningKey::from_bytes(&[0x5eu8; 32].into()).unwrap();
    let signer = signer_address(&sk);
    let contract = [0xC0u8; 20];
    let chain_id = u256_word(1);

    let a = Authorization::new(
        signer,
        [0xabu8; 20],
        true,
        U256::from(7u64),
        U256::from(1_900_000_000u64),
    );

    let sig = sign_authorization(&sk, &a, chain_id, &contract);
    assert_eq!(
        recover_authorization(&a, chain_id, &contract, &sig),
        Some(signer),
        "round-trip: recovered signer must equal the signing key's address"
    );

    // A different domain (verifyingContract) yields a different digest -> different recovery.
    let other_contract = [0xC1u8; 20];
    assert_ne!(
        recover_authorization(&a, chain_id, &other_contract, &sig),
        Some(signer),
        "signature must not verify under a different verifyingContract"
    );
}
