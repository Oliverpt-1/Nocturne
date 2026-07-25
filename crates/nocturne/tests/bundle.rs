//! Bundle calldata tests: round-trip `encode_bundle_calldata` / `decode_bundle_calldata` for
//! both wrapper shapes, verify the embedded fills like bare takes, and assert that malformed
//! input (wrong selector, truncated production payload) errors instead of panicking.

use nocturne::*;

fn offer_for(maker: Address, tick: u64) -> Offer {
    Offer {
        market: Market {
            chain_id: word_from_u64(31337),
            midnight: [0x11; 20],
            loan_token: [0x22; 20],
            collateral_params: vec![CollateralParams {
                token: [0x33; 20],
                lltv: word_from_u128(770_000_000_000_000_000),
                liquidation_cursor: word_from_u128(300_000_000_000_000_000),
                oracle: [0x44; 20],
            }],
            maturity: word_from_u64(4_000_000_000),
            rcf_threshold: word_from_u64(1000),
            enter_gate: [0u8; 20],
            liquidator_gate: [0u8; 20],
        },
        buy: true,
        maker,
        start: word_from_u64(0),
        expiry: word_from_u64(4_000_000_000),
        tick: word_from_u64(tick),
        group: word_from_u64(1),
        callback: [0u8; 20],
        callback_data: Vec::new(),
        receiver_if_maker_is_seller: [0u8; 20],
        ratifier: [0xbb; 20],
        reduce_only: false,
        max_units: 1_000_000,
        max_assets: 0,
        continuous_fee_cap: word_from_u64(0),
    }
}

/// Two signed fills from one maker's 2-leaf tree, verifiable like bare takes.
fn signed_fills() -> (Vec<OfferFill>, Address) {
    let signer = LocalSigner::from_bytes(&[0x42; 32]).unwrap();
    let offers = [
        offer_for(signer.address(), 3372),
        offer_for(signer.address(), 3376),
    ];
    let chain_id = word_from_u64(31337);
    let tree = OfferTree::build(offers.iter().map(hash_offer).collect()).unwrap();
    let sig = signer
        .sign_digest(&tree_digest(
            tree.root(),
            tree.height(),
            chain_id,
            &offers[0].ratifier,
        ))
        .unwrap();

    let fills = offers
        .iter()
        .enumerate()
        .map(|(i, offer)| {
            let raw = encode_ratifier_data(&sig, &tree.root(), i, &tree.proof(i));
            OfferFill {
                offer: offer.clone(),
                ratifier_data: decode_ratifier_data(&raw).unwrap(),
                ratifier_data_raw: raw,
                units: U256::from(100_000u64 * (i as u64 + 1)),
            }
        })
        .collect();
    (fills, signer.address())
}

fn buy_bundle(fills: Vec<OfferFill>) -> BundleCall {
    BundleCall {
        kind: BundleKind::BuyWithAssetsTarget,
        target: U256::from(500_000u64),
        limit: U256::from(501_570u64),
        taker: [0x77; 20],
        reduce_only: false,
        side: BundleSide::Buy {
            loan_token_permit: TokenPermit {
                kind: 0,
                data: Vec::new(),
            },
            collateral_withdrawals: vec![CollateralWithdrawal {
                collateral_index: U256::ZERO,
                assets: U256::from(42u64),
            }],
            collateral_receiver: [0x88; 20],
        },
        fills,
        referral_fee_pct: U256::ZERO,
        referral_fee_recipient: [0u8; 20],
        max_continuous_fee: U256::MAX,
        deadline: U256::from(4_000_000_000u64),
    }
}

fn sell_bundle(fills: Vec<OfferFill>) -> BundleCall {
    BundleCall {
        kind: BundleKind::SellWithUnitsTarget,
        target: U256::from(250_000u64),
        limit: U256::from(240_000u64),
        taker: [0x77; 20],
        reduce_only: true,
        side: BundleSide::Sell {
            receiver: [0x99; 20],
            collateral_supplies: vec![CollateralSupply {
                collateral_index: U256::ZERO,
                assets: U256::from(7u64),
                permit: TokenPermit {
                    kind: 2,
                    data: vec![0xde, 0xad, 0xbe, 0xef],
                },
            }],
        },
        fills,
        referral_fee_pct: U256::from(10u64),
        referral_fee_recipient: [0xaa; 20],
        max_continuous_fee: U256::from(3_000_000_000u64),
        deadline: U256::from(4_000_000_000u64),
    }
}

#[test]
fn buy_bundle_roundtrips() {
    let (fills, _) = signed_fills();
    let bundle = buy_bundle(fills);
    let calldata = encode_bundle_calldata(&bundle);
    assert_eq!(calldata[..4], BUNDLE_BUY_ASSETS_SELECTOR);
    let decoded = decode_bundle_calldata(&calldata).unwrap();
    assert_eq!(decoded, bundle);
}

#[test]
fn sell_bundle_roundtrips() {
    let (fills, _) = signed_fills();
    let bundle = sell_bundle(fills);
    let calldata = encode_bundle_calldata(&bundle);
    assert_eq!(calldata[..4], BUNDLE_SELL_UNITS_SELECTOR);
    let decoded = decode_bundle_calldata(&calldata).unwrap();
    assert_eq!(decoded, bundle);
}

#[test]
fn decoded_fills_verify_like_bare_takes() {
    let (fills, maker) = signed_fills();
    let calldata = encode_bundle_calldata(&buy_bundle(fills));
    let decoded = decode_bundle_calldata(&calldata).unwrap();
    assert_eq!(decoded.fills.len(), 2);
    for fill in &decoded.fills {
        let rd = &fill.ratifier_data;
        assert!(verify_leaf(
            &rd.root,
            &hash_offer(&fill.offer),
            rd.leaf_index,
            &rd.proof
        ));
        assert!(verify(
            &fill.offer,
            &rd.root,
            rd.leaf_index,
            &rd.proof,
            &rd.sig,
            word_from_u64(31337),
            &fill.offer.ratifier,
            &maker,
        ));
    }
}

#[test]
fn trailing_metadata_bytes_are_tolerated() {
    let (fills, _) = signed_fills();
    let bundle = buy_bundle(fills);
    let mut calldata = encode_bundle_calldata(&bundle);
    // Apps append referral/metadata tags after the ABI region; they must not break decoding.
    calldata.extend_from_slice(&[0x6a, 0x63, 0xc0, 0x8d, 0x12, 0x12, 0x12, 0x12]);
    assert_eq!(decode_bundle_calldata(&calldata).unwrap(), bundle);
}

#[test]
fn selector_kind_mapping_roundtrips() {
    for kind in [
        BundleKind::BuyWithUnitsTarget,
        BundleKind::SellWithUnitsTarget,
        BundleKind::BuyWithAssetsTarget,
        BundleKind::SellWithAssetsTarget,
    ] {
        assert_eq!(BundleKind::from_selector(kind.selector()), Some(kind));
    }
    assert_eq!(BundleKind::from_selector(TAKE_SELECTOR), None);
}

#[test]
fn non_bundle_selector_is_rejected() {
    let err = decode_bundle_calldata(&TAKE_SELECTOR).unwrap_err();
    assert_eq!(err, DecodeError::BadSelector(TAKE_SELECTOR));
}

#[test]
fn truncated_production_payload_errors_cleanly() {
    // A real BuyWithAssetsTarget payload whose tail was lost in copy-paste: it declares six
    // fills and a collateralWithdrawals offset past its own end. Decoding must report
    // TooShort (never panic, never return a partial bundle).
    let hex_str = include_str!("data/truncated_bundle.hex");
    let bytes = hex::decode(hex_str.trim().trim_start_matches("0x")).unwrap();
    match decode_bundle_calldata(&bytes) {
        Err(DecodeError::TooShort { needed, have }) => {
            assert!(needed > have, "needed {needed} vs have {have}");
        }
        other => panic!("expected TooShort, got {other:?}"),
    }
}

#[test]
fn truncation_anywhere_never_panics() {
    let (fills, _) = signed_fills();
    let calldata = encode_bundle_calldata(&buy_bundle(fills));
    for len in 0..calldata.len() {
        let _ = decode_bundle_calldata(&calldata[..len]);
    }
}
