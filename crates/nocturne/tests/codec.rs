//! Contract-anchored parity for the ABI calldata codec.
//!
//! The expected bytes below are produced by `fixtures/GenCodec.t.sol` run against the Midnight
//! contracts at rev f47568c9e45a9b70830b82a130b47393dcafec33 (Forge 1.7.1). The fixture emits
//! Solidity's own `abi.encode` / `abi.encodeCall` output for a fixed Offer + Signature + proof +
//! take params; here we rebuild the identical inputs in Rust and assert byte-for-byte equality.
//!
//! Regenerate: see the header of `crates/nocturne/fixtures/GenCodec.t.sol`.

use nocturne::*;

// ---- baked from GenCodec.t.sol ----

const TAKE_SELECTOR_HEX: &str = "6a14c9ef";
const CANCEL_ROOT_SELECTOR_HEX: &str = "bb1f12aa";

const RATIFIER_DATA_HEX: &str = "\
000000000000000000000000000000000000000000000000000000000000001c\
1111111111111111111111111111111111111111111111111111111111111111\
2222222222222222222222222222222222222222222222222222222222222222\
3333333333333333333333333333333333333333333333333333333333333333\
0000000000000000000000000000000000000000000000000000000000000002\
00000000000000000000000000000000000000000000000000000000000000c0\
0000000000000000000000000000000000000000000000000000000000000002\
4444444444444444444444444444444444444444444444444444444444444444\
5555555555555555555555555555555555555555555555555555555555555555";

const TAKE_CALLDATA_HEX: &str = "\
6a14c9ef\
00000000000000000000000000000000000000000000000000000000000000e0\
0000000000000000000000000000000000000000000000000000000000000520\
000000000000000000000000000000000000000000000000000000000003d090\
000000000000000000000000cccccccccccccccccccccccccccccccccccccccc\
000000000000000000000000dddddddddddddddddddddddddddddddddddddddd\
000000000000000000000000eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee\
0000000000000000000000000000000000000000000000000000000000000660\
00000000000000000000000000000000000000000000000000000000000001e0\
0000000000000000000000000000000000000000000000000000000000000001\
0000000000000000000000001234567890123456789012345678901234567890\
0000000000000000000000000000000000000000000000000000000000000001\
0000000000000000000000000000000000000000000000000000000077359400\
000000000000000000000000000000000000000000000000000000000000002a\
0000000000000000000000000000000000000000000000000000000000000007\
0000000000000000000000009999999999999999999999999999999999999999\
0000000000000000000000000000000000000000000000000000000000000400\
000000000000000000000000aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\
000000000000000000000000bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb\
0000000000000000000000000000000000000000000000000000000000000001\
00000000000000000000000000000000000000000000000000000000000f4240\
000000000000000000000000000000000000000000000000000000000007a120\
000000000000000000000000000000000000000000000000000000000001e240\
0000000000000000000000000000000000000000000000000000000000000001\
0000000000000000000000001111111111111111111111111111111111111111\
0000000000000000000000002222222222222222222222222222222222222222\
0000000000000000000000000000000000000000000000000000000000000100\
000000000000000000000000000000000000000000000000000000006b49d200\
00000000000000000000000000000000000000000000000000000000000003e8\
0000000000000000000000007777777777777777777777777777777777777777\
0000000000000000000000008888888888888888888888888888888888888888\
0000000000000000000000000000000000000000000000000000000000000002\
0000000000000000000000003333333333333333333333333333333333333333\
0000000000000000000000000000000000000000000000000bef55718ad60000\
0000000000000000000000000000000000000000000000000000000000000001\
0000000000000000000000004444444444444444444444444444444444444444\
0000000000000000000000005555555555555555555555555555555555555555\
0000000000000000000000000000000000000000000000000cb2bba6f17b8000\
0000000000000000000000000000000000000000000000000000000000000002\
0000000000000000000000006666666666666666666666666666666666666666\
0000000000000000000000000000000000000000000000000000000000000004\
deadbeef00000000000000000000000000000000000000000000000000000000\
0000000000000000000000000000000000000000000000000000000000000120\
000000000000000000000000000000000000000000000000000000000000001c\
1111111111111111111111111111111111111111111111111111111111111111\
2222222222222222222222222222222222222222222222222222222222222222\
3333333333333333333333333333333333333333333333333333333333333333\
0000000000000000000000000000000000000000000000000000000000000002\
00000000000000000000000000000000000000000000000000000000000000c0\
0000000000000000000000000000000000000000000000000000000000000002\
4444444444444444444444444444444444444444444444444444444444444444\
5555555555555555555555555555555555555555555555555555555555555555\
0000000000000000000000000000000000000000000000000000000000000002\
cafe000000000000000000000000000000000000000000000000000000000000";

const CANCEL_ROOT_CALLDATA_HEX: &str = "\
bb1f12aa\
0000000000000000000000001234567890123456789012345678901234567890\
3333333333333333333333333333333333333333333333333333333333333333";

// ---- helpers ----

fn hx(s: &str) -> Vec<u8> {
    assert!(s.len() % 2 == 0, "odd-length hex");
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).expect("valid hex"))
        .collect()
}

fn word32(byte: u8) -> Word {
    [byte; 32]
}
fn addr(byte: u8) -> Address {
    [byte; 20]
}

/// Rebuild the exact same inputs the fixture used.
fn fixture_offer() -> Offer {
    let market = Market {
        chain_id: word_from_u64(1),
        midnight: addr(0x11),
        loan_token: addr(0x22),
        collateral_params: vec![
            CollateralParams {
                token: addr(0x33),
                lltv: word_from_u128(860_000_000_000_000_000),
                liquidation_cursor: word_from_u64(1),
                oracle: addr(0x44),
            },
            CollateralParams {
                token: addr(0x55),
                lltv: word_from_u128(915_000_000_000_000_000),
                liquidation_cursor: word_from_u64(2),
                oracle: addr(0x66),
            },
        ],
        maturity: word_from_u64(1_800_000_000),
        rcf_threshold: word_from_u64(1000),
        enter_gate: addr(0x77),
        liquidator_gate: addr(0x88),
    };
    Offer {
        market,
        buy: true,
        maker: [0x12, 0x34, 0x56, 0x78, 0x90].repeat(4).try_into().unwrap(),
        start: word_from_u64(1),
        expiry: word_from_u64(2_000_000_000),
        tick: word_from_u64(42),
        group: word_from_u64(7),
        callback: addr(0x99),
        callback_data: vec![0xde, 0xad, 0xbe, 0xef],
        receiver_if_maker_is_seller: addr(0xaa),
        ratifier: addr(0xbb),
        reduce_only: true,
        max_units: 1_000_000,
        max_assets: 500_000,
        continuous_fee_cap: word_from_u64(123_456),
    }
}

fn fixture_sig() -> Sig {
    Sig {
        r: word32(0x11),
        s: word32(0x22),
        v: 28,
    }
}

// ---- selectors ----

#[test]
fn selectors_match_solidity() {
    assert_eq!(TAKE_SELECTOR.to_vec(), hx(TAKE_SELECTOR_HEX));
    assert_eq!(CANCEL_ROOT_SELECTOR.to_vec(), hx(CANCEL_ROOT_SELECTOR_HEX));
}

// ---- ratifier data ----

#[test]
fn ratifier_data_matches_solidity() {
    let sig = fixture_sig();
    let root = word32(0x33);
    let proof = [word32(0x44), word32(0x55)];
    let got = encode_ratifier_data(&sig, &root, 2, &proof);
    assert_eq!(got, hx(RATIFIER_DATA_HEX));
}

// ---- take calldata ----

#[test]
fn take_calldata_matches_solidity() {
    let offer = fixture_offer();
    let sig = fixture_sig();
    let root = word32(0x33);
    let proof = [word32(0x44), word32(0x55)];
    let ratifier_data = encode_ratifier_data(&sig, &root, 2, &proof);

    let got = encode_take_calldata(
        &offer,
        &ratifier_data,
        U256::from(250_000u64),
        &addr(0xcc),
        &addr(0xdd),
        &addr(0xee),
        &[0xca, 0xfe],
    );
    assert_eq!(got, hx(TAKE_CALLDATA_HEX));
    // selector-prefixed, and the argument block is a whole number of 32-byte words.
    assert_eq!(&got[..4], &TAKE_SELECTOR);
    assert_eq!((got.len() - 4) % 32, 0);
}

// ---- cancelRoot calldata ----

#[test]
fn cancel_root_calldata_matches_solidity() {
    let maker: Address = [0x12, 0x34, 0x56, 0x78, 0x90].repeat(4).try_into().unwrap();
    let root = word32(0x33);
    let got = encode_cancel_root_calldata(&maker, &root);
    assert_eq!(got, hx(CANCEL_ROOT_CALLDATA_HEX));
    assert_eq!(&got[..4], &CANCEL_ROOT_SELECTOR);
    assert_eq!(got.len(), 4 + 64); // selector + two static words
}

// ---- structural checks independent of the fixture ----

#[test]
fn empty_proof_and_bytes_encode_cleanly() {
    // No proof elements, empty ratifier/callback data: still valid ABI (offsets + zero lengths).
    let sig = Sig {
        r: word32(1),
        s: word32(2),
        v: 27,
    };
    let rd = encode_ratifier_data(&sig, &word32(9), 0, &[]);
    // head: v, r, s, root, leafIndex, proof-offset (6 words) + proof length word = 7 words.
    assert_eq!(rd.len(), 7 * 32);
    // proof offset points just past the head.
    assert_eq!(word_to_u128(&rd[160..192].try_into().unwrap()), Some(192));
    // proof length is zero.
    assert_eq!(rd[192..224], [0u8; 32]);
}

#[test]
fn take_calldata_embeds_ratifier_data_verbatim() {
    // The ratifierData bytes must appear byte-for-byte inside the take calldata tail.
    let offer = fixture_offer();
    let ratifier_data = encode_ratifier_data(
        &fixture_sig(),
        &word32(0x33),
        2,
        &[word32(0x44), word32(0x55)],
    );
    let calldata = encode_take_calldata(
        &offer,
        &ratifier_data,
        U256::from(1u64),
        &addr(0xcc),
        &addr(0xdd),
        &addr(0xee),
        &[],
    );
    let window = ratifier_data.len();
    assert!(
        calldata
            .windows(window)
            .any(|w| w == ratifier_data.as_slice()),
        "ratifierData should be embedded verbatim in the take calldata"
    );
}
