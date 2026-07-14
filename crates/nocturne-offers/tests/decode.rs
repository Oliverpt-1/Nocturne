//! Decoder tests: a small local ABI encoder mirrors Solidity `abi.encode(offer)`, then we
//! round-trip through [`decode_offer`]. State-getter decoders are checked against hand-built
//! word layouts, and malformed input must error rather than panic.
//!
//! A contract-anchored fixture (bytes printed by `fixtures/GenDecode.t.sol` against the
//! Midnight contracts) is baked in below and decoded to assert exact fields.

use nocturne_offers::*;

// ---- minimal ABI encoder (Solidity `abi.encode` for the Offer tuple) --------

fn addr_word(a: &Address) -> [u8; 32] {
    let mut w = [0u8; 32];
    w[12..].copy_from_slice(a);
    w
}
fn u128_word(x: u128) -> [u8; 32] {
    let mut w = [0u8; 32];
    w[16..].copy_from_slice(&x.to_be_bytes());
    w
}
fn usize_word(x: usize) -> [u8; 32] {
    let mut w = [0u8; 32];
    w[24..].copy_from_slice(&(x as u64).to_be_bytes());
    w
}
fn bool_word(b: bool) -> [u8; 32] {
    let mut w = [0u8; 32];
    w[31] = b as u8;
    w
}

fn encode_collateral_array(cps: &[CollateralParams]) -> Vec<u8> {
    let mut out = usize_word(cps.len()).to_vec();
    for cp in cps {
        out.extend_from_slice(&addr_word(&cp.token));
        out.extend_from_slice(&cp.lltv);
        out.extend_from_slice(&cp.liquidation_cursor);
        out.extend_from_slice(&addr_word(&cp.oracle));
    }
    out
}

fn encode_bytes(data: &[u8]) -> Vec<u8> {
    let mut out = usize_word(data.len()).to_vec();
    out.extend_from_slice(data);
    let pad = (32 - data.len() % 32) % 32;
    out.extend(std::iter::repeat(0u8).take(pad));
    out
}

fn encode_market_tuple(m: &Market) -> Vec<u8> {
    let cp_blob = encode_collateral_array(&m.collateral_params);
    let head_len = 8 * 32;
    let mut out = Vec::new();
    out.extend_from_slice(&m.chain_id);
    out.extend_from_slice(&addr_word(&m.midnight));
    out.extend_from_slice(&addr_word(&m.loan_token));
    out.extend_from_slice(&usize_word(head_len)); // collateralParams offset
    out.extend_from_slice(&m.maturity);
    out.extend_from_slice(&m.rcf_threshold);
    out.extend_from_slice(&addr_word(&m.enter_gate));
    out.extend_from_slice(&addr_word(&m.liquidator_gate));
    out.extend_from_slice(&cp_blob);
    out
}

fn encode_offer_tuple(o: &Offer) -> Vec<u8> {
    let market_blob = encode_market_tuple(&o.market);
    let cb_blob = encode_bytes(&o.callback_data);
    let head_len = 15 * 32;
    let market_offset = head_len;
    let cb_offset = head_len + market_blob.len();

    let mut out = Vec::new();
    out.extend_from_slice(&usize_word(market_offset));
    out.extend_from_slice(&bool_word(o.buy));
    out.extend_from_slice(&addr_word(&o.maker));
    out.extend_from_slice(&o.start);
    out.extend_from_slice(&o.expiry);
    out.extend_from_slice(&o.tick);
    out.extend_from_slice(&o.group);
    out.extend_from_slice(&addr_word(&o.callback));
    out.extend_from_slice(&usize_word(cb_offset));
    out.extend_from_slice(&addr_word(&o.receiver_if_maker_is_seller));
    out.extend_from_slice(&addr_word(&o.ratifier));
    out.extend_from_slice(&bool_word(o.reduce_only));
    out.extend_from_slice(&u128_word(o.max_units));
    out.extend_from_slice(&u128_word(o.max_assets));
    out.extend_from_slice(&o.continuous_fee_cap);
    out.extend_from_slice(&market_blob);
    out.extend_from_slice(&cb_blob);
    out
}

fn abi_encode_offer(o: &Offer) -> Vec<u8> {
    let mut out = usize_word(0x20).to_vec();
    out.extend_from_slice(&encode_offer_tuple(o));
    out
}

// ---- fixtures ----------------------------------------------------------------

fn cp(token: u8, lltv: u64, cursor: u64, oracle: u8) -> CollateralParams {
    CollateralParams {
        token: [token; 20],
        lltv: word_from_u64(lltv),
        liquidation_cursor: word_from_u64(cursor),
        oracle: [oracle; 20],
    }
}

fn sample_offer(cps: Vec<CollateralParams>, callback_data: Vec<u8>, buy: bool) -> Offer {
    Offer {
        market: Market {
            chain_id: word_from_u64(1),
            midnight: [0x11; 20],
            loan_token: [0x22; 20],
            collateral_params: cps,
            maturity: word_from_u64(1_800_000_000),
            rcf_threshold: word_from_u64(1000),
            enter_gate: [0xa1; 20],
            liquidator_gate: [0xa2; 20],
        },
        buy,
        maker: [0x33; 20],
        start: word_from_u64(0),
        expiry: word_from_u64(2_000_000_000),
        tick: word_from_u64(3372),
        group: word_from_u64(7),
        callback: [0x44; 20],
        callback_data,
        receiver_if_maker_is_seller: [0x55; 20],
        ratifier: [0xbb; 20],
        reduce_only: !buy,
        max_units: 1_000_000,
        max_assets: 999,
        continuous_fee_cap: word_from_u64(42),
    }
}

fn assert_offer_eq(a: &Offer, b: &Offer) {
    // Cheapest exhaustive check: identical EIP-712 leaf and identical re-encoding.
    assert_eq!(hash_offer(a), hash_offer(b), "hash_offer mismatch");
    assert_eq!(abi_encode_offer(a), abi_encode_offer(b), "re-encode mismatch");

    // Explicit field spot-checks for good measure.
    assert_eq!(a.buy, b.buy);
    assert_eq!(a.maker, b.maker);
    assert_eq!(a.callback, b.callback);
    assert_eq!(a.callback_data, b.callback_data);
    assert_eq!(a.reduce_only, b.reduce_only);
    assert_eq!(a.max_units, b.max_units);
    assert_eq!(a.max_assets, b.max_assets);
    assert_eq!(a.continuous_fee_cap, b.continuous_fee_cap);
    assert_eq!(a.market.chain_id, b.market.chain_id);
    assert_eq!(a.market.midnight, b.market.midnight);
    assert_eq!(a.market.loan_token, b.market.loan_token);
    assert_eq!(a.market.enter_gate, b.market.enter_gate);
    assert_eq!(a.market.liquidator_gate, b.market.liquidator_gate);
    assert_eq!(
        a.market.collateral_params.len(),
        b.market.collateral_params.len()
    );
    for (x, y) in a
        .market
        .collateral_params
        .iter()
        .zip(&b.market.collateral_params)
    {
        assert_eq!(x.token, y.token);
        assert_eq!(x.lltv, y.lltv);
        assert_eq!(x.liquidation_cursor, y.liquidation_cursor);
        assert_eq!(x.oracle, y.oracle);
    }
}

// ---- offer round-trip --------------------------------------------------------

#[test]
fn round_trip_empty_callback_one_collateral() {
    let o = sample_offer(vec![cp(0x33, 860_000_000_000_000_000, 1, 0x44)], vec![], true);
    let decoded = decode_offer(&abi_encode_offer(&o)).unwrap();
    assert_offer_eq(&o, &decoded);
}

#[test]
fn round_trip_nonempty_callback() {
    let data = vec![0xde, 0xad, 0xbe, 0xef, 0x01, 0x02, 0x03];
    let o = sample_offer(vec![cp(0x33, 5, 6, 0x44)], data, false);
    let decoded = decode_offer(&abi_encode_offer(&o)).unwrap();
    assert_offer_eq(&o, &decoded);
    assert_eq!(decoded.callback_data, vec![0xde, 0xad, 0xbe, 0xef, 0x01, 0x02, 0x03]);
}

#[test]
fn round_trip_two_collateral_params() {
    let cps = vec![
        cp(0x33, 800_000_000_000_000_000, 1, 0x44),
        cp(0x66, 900_000_000_000_000_000, 2, 0x77),
    ];
    let o = sample_offer(cps, vec![0xaa; 40], true);
    let decoded = decode_offer(&abi_encode_offer(&o)).unwrap();
    assert_offer_eq(&o, &decoded);
    assert_eq!(decoded.market.collateral_params.len(), 2);
}

#[test]
fn round_trip_zero_collateral_params() {
    let o = sample_offer(vec![], vec![0x01], false);
    let decoded = decode_offer(&abi_encode_offer(&o)).unwrap();
    assert_offer_eq(&o, &decoded);
    assert!(decoded.market.collateral_params.is_empty());
}

// ---- market state ------------------------------------------------------------

fn small_word(value: u64, n_bytes: usize) -> [u8; 32] {
    let mut w = [0u8; 32];
    let b = value.to_be_bytes();
    w[32 - n_bytes..].copy_from_slice(&b[8 - n_bytes..]);
    w
}

fn build_market_state(
    total_units: u128,
    loss_factor: u128,
    withdrawable: u128,
    cf_credit: u128,
    cbp: [u16; 7],
    continuous_fee: u32,
    tick_spacing: u8,
) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(&u128_word(total_units));
    out.extend_from_slice(&u128_word(loss_factor));
    out.extend_from_slice(&u128_word(withdrawable));
    out.extend_from_slice(&u128_word(cf_credit));
    for c in cbp {
        out.extend_from_slice(&small_word(c as u64, 2));
    }
    out.extend_from_slice(&small_word(continuous_fee as u64, 4));
    out.extend_from_slice(&small_word(tick_spacing as u64, 1));
    out
}

#[test]
fn decode_market_state_fields_and_projections() {
    let cbp = [1u16, 2, 3, 4, 5, 6, 7];
    let bytes = build_market_state(
        1_000_000,
        123,
        50,
        7,
        cbp,
        250,
        60,
    );
    let ms = decode_market_state(&bytes).unwrap();
    assert_eq!(ms.total_units, 1_000_000);
    assert_eq!(ms.loss_factor, 123);
    assert_eq!(ms.withdrawable, 50);
    assert_eq!(ms.continuous_fee_credit, 7);
    assert_eq!(ms.settlement_fee_cbp, cbp);
    assert_eq!(ms.continuous_fee, 250);
    assert_eq!(ms.tick_spacing, 60);

    let sim = ms.to_sim_market();
    assert_eq!(sim.tick_spacing, 60);
    assert_eq!(sim.continuous_fee, 250);
    assert_eq!(sim.settlement_fee_cbp, cbp);
    assert!(!sim.loss_factor_maxed);

    let snap = ms.to_market_snapshot();
    assert_eq!(snap.tick_spacing, 60);
    assert_eq!(snap.continuous_fee, 250);
    assert!(!snap.loss_factor_maxed);
}

#[test]
fn decode_market_state_loss_factor_maxed() {
    let bytes = build_market_state(0, u128::MAX, 0, 0, [0; 7], 0, 1);
    let ms = decode_market_state(&bytes).unwrap();
    assert!(ms.to_sim_market().loss_factor_maxed);
    assert!(ms.to_market_snapshot().loss_factor_maxed);
}

// ---- position ----------------------------------------------------------------

#[test]
fn decode_position_fields_and_projection() {
    let mut bytes = Vec::new();
    for v in [100u128, 5, 999, 1_700_000_000, 40, 0b1010] {
        bytes.extend_from_slice(&u128_word(v));
    }
    let p = decode_position(&bytes).unwrap();
    assert_eq!(p.credit, 100);
    assert_eq!(p.pending_fee, 5);
    assert_eq!(p.last_loss_factor, 999);
    assert_eq!(p.last_accrual, 1_700_000_000);
    assert_eq!(p.debt, 40);
    assert_eq!(p.collateral_bitmap, 0b1010);

    let sp = p.to_sim_position();
    assert_eq!(sp.credit, 100);
    assert_eq!(sp.debt, 40);
    assert_eq!(sp.pending_fee, 5);
}

#[test]
fn decode_consumed_single_word() {
    let bytes = u128_word(123_456).to_vec();
    assert_eq!(decode_consumed(&bytes).unwrap(), 123_456);
}

// ---- malformed input ---------------------------------------------------------

#[test]
fn truncated_offer_is_too_short_not_panic() {
    let o = sample_offer(vec![cp(0x33, 5, 6, 0x44)], vec![0x01, 0x02], true);
    let full = abi_encode_offer(&o);
    // Cut off the tail so a dynamic-field read runs past the end.
    let truncated = &full[..full.len() - 40];
    assert!(matches!(
        decode_offer(truncated),
        Err(DecodeError::TooShort { .. })
    ));
}

#[test]
fn empty_input_is_too_short() {
    assert!(matches!(decode_offer(&[]), Err(DecodeError::TooShort { .. })));
    assert!(matches!(decode_market_state(&[]), Err(DecodeError::TooShort { .. })));
    assert!(matches!(decode_position(&[]), Err(DecodeError::TooShort { .. })));
    assert!(matches!(decode_consumed(&[]), Err(DecodeError::TooShort { .. })));
}

#[test]
fn truncated_market_state_is_too_short() {
    let bytes = build_market_state(1, 2, 3, 4, [0; 7], 5, 6);
    assert!(matches!(
        decode_market_state(&bytes[..bytes.len() - 1]),
        Err(DecodeError::TooShort { .. })
    ));
}

#[test]
fn bad_bool_word_errors() {
    let o = sample_offer(vec![], vec![], true);
    let mut bytes = abi_encode_offer(&o);
    // Offer `buy` is head word 1 of the tuple; tuple starts at byte 32.
    // Corrupt the last byte of that word to an invalid bool value.
    let buy_word_end = 32 + 2 * 32; // end of head word index 1
    bytes[buy_word_end - 1] = 5;
    assert!(matches!(decode_offer(&bytes), Err(DecodeError::InvalidBool)));
}

#[test]
fn oversized_u128_errors() {
    // A consumed word with a high byte set does not fit in u128.
    let mut w = [0u8; 32];
    w[0] = 1;
    assert!(matches!(decode_consumed(&w), Err(DecodeError::IntegerOverflow)));
}

// ---- contract-anchored fixture (from fixtures/GenDecode.t.sol) ---------------

/// Decode a hex string (no `0x`) into bytes.
fn from_hex(s: &str) -> Vec<u8> {
    let s = s.trim();
    let s = s.strip_prefix("0x").unwrap_or(s);
    assert!(s.len() % 2 == 0, "odd hex length");
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).expect("valid hex"))
        .collect()
}

// abi.encode(offer) for the offer built in fixtures/GenDecode.t.sol, printed by
// `forge test --match-contract GenDecode -vv` against the Midnight contracts
// (rev f47568c9e45a9b70830b82a130b47393dcafec33).
const FIXTURE_OFFER_HEX: &str = "0x000000000000000000000000000000000000000000000000000000000000002000000000000000000000000000000000000000000000000000000000000001e00000000000000000000000000000000000000000000000000000000000000001000000000000000000000000abababababababababababababababababababab000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000773594000000000000000000000000000000000000000000000000000000000000000d2c0000000000000000000000000000000000000000000000000000000000000007000000000000000000000000444444444444444444444444444444444444444400000000000000000000000000000000000000000000000000000000000004000000000000000000000000005555555555555555555555555555555555555555000000000000000000000000bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000f424000000000000000000000000000000000000000000000000000000000000003e7000000000000000000000000000000000000000000000000000000000000002a0000000000000000000000000000000000000000000000000000000000000001000000000000000000000000111111111111111111111111111111111111111100000000000000000000000022222222222222222222222222222222222222220000000000000000000000000000000000000000000000000000000000000100000000000000000000000000000000000000000000000000000000006b49d20000000000000000000000000000000000000000000000000000000000000003e8000000000000000000000000a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1000000000000000000000000a2a2a2a2a2a2a2a2a2a2a2a2a2a2a2a2a2a2a2a2000000000000000000000000000000000000000000000000000000000000000200000000000000000000000033333333333333333333333333333333333333330000000000000000000000000000000000000000000000000bef55718ad600000000000000000000000000000000000000000000000000000000000000000001000000000000000000000000444444444444444444444444444444444444444400000000000000000000000066666666666666666666666666666666666666660000000000000000000000000000000000000000000000000c7d713b49da0000000000000000000000000000000000000000000000000000000000000000000200000000000000000000000077777777777777777777777777777777777777770000000000000000000000000000000000000000000000000000000000000004deadbeef00000000000000000000000000000000000000000000000000000000";

#[test]
fn decode_contract_encoded_offer() {
    if FIXTURE_OFFER_HEX.is_empty() {
        // No fixture baked (forge unavailable); the self-contained round-trip tests cover us.
        return;
    }
    let bytes = from_hex(FIXTURE_OFFER_HEX);
    let o = decode_offer(&bytes).unwrap();

    // Field expectations mirror the offer constructed in GenDecode.t.sol.
    assert_eq!(o.market.chain_id, word_from_u64(1));
    assert_eq!(o.market.midnight, [0x11u8; 20]);
    assert_eq!(o.market.loan_token, [0x22u8; 20]);
    assert_eq!(o.market.collateral_params.len(), 2);
    assert_eq!(o.market.collateral_params[0].token, [0x33u8; 20]);
    assert_eq!(
        o.market.collateral_params[0].lltv,
        word_from_u64(860_000_000_000_000_000)
    );
    assert_eq!(o.market.collateral_params[1].oracle, [0x77u8; 20]);
    assert_eq!(o.market.maturity, word_from_u64(1_800_000_000));
    assert!(o.buy);
    assert_eq!(o.maker, [0xabu8; 20]);
    assert_eq!(o.tick, word_from_u64(3372));
    assert_eq!(o.callback_data, vec![0xde, 0xad, 0xbe, 0xef]);
    assert_eq!(o.max_units, 1_000_000);
    assert_eq!(o.max_assets, 999);
    assert_eq!(o.continuous_fee_cap, word_from_u64(42));

    // Re-encoding the decoded offer must reproduce the exact contract bytes.
    assert_eq!(abi_encode_offer(&o), bytes);
}
