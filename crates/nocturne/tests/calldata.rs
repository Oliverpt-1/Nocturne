//! Round-trip tests for the calldata decoders (`decode_take_calldata`, `decode_ratifier_data`,
//! `decode_cancel_root_calldata`).
//!
//! The existing take/ratifier/cancel encoders are asserted byte-for-byte against the Midnight
//! contracts in `tests/codec.rs`; the repay-and-withdraw encoder is independently checked against
//! Alloy's Solidity ABI bindings in `tests/actions.rs`. Encoding with those verified encoders and
//! decoding back is therefore a parity check on the decoders: if a decoder disagrees with its
//! inverse, the round-trip breaks.

use nocturne::*;

fn offer_for(maker: Address) -> Offer {
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
        tick: word_from_u64(3372),
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

#[test]
fn ratifier_data_round_trips() {
    let signer = LocalSigner::from_bytes(&[0x42; 32]).unwrap();
    let offer = offer_for(signer.address());
    let ratifier = offer.ratifier;
    let chain_id = word_from_u64(31337);

    // Two-leaf tree so the proof is non-empty (height 1, one sibling).
    let other = offer_for([0x99; 20]);
    let tree = OfferTree::build(vec![hash_offer(&offer), hash_offer(&other)]).unwrap();
    let leaf_index = 0;
    let proof = tree.proof(leaf_index).unwrap();
    let digest = tree_digest(tree.root(), tree.height(), chain_id, &ratifier);
    let sig = signer.sign_digest(&digest).unwrap();

    let encoded = encode_ratifier_data(&sig, &tree.root(), leaf_index, &proof);
    let decoded = decode_ratifier_data(&encoded).unwrap();

    assert_eq!(decoded.sig, sig);
    assert_eq!(decoded.root, tree.root());
    assert_eq!(decoded.leaf_index, leaf_index);
    assert_eq!(decoded.proof, proof);
}

#[test]
fn take_calldata_round_trips() {
    let signer = LocalSigner::from_bytes(&[0x42; 32]).unwrap();
    let offer = offer_for(signer.address());
    let ratifier = offer.ratifier;
    let chain_id = word_from_u64(31337);

    let tree = OfferTree::build(vec![hash_offer(&offer)]).unwrap();
    let proof = tree.proof(0).unwrap();
    let digest = tree_digest(tree.root(), tree.height(), chain_id, &ratifier);
    let sig = signer.sign_digest(&digest).unwrap();
    let ratifier_data = encode_ratifier_data(&sig, &tree.root(), 0, &proof);

    let units = U256::from(250_000u64);
    let taker = [0x77; 20];
    let receiver = [0x88; 20];
    let taker_callback = [0u8; 20];
    let taker_callback_data = b"hello-callback-data".to_vec();

    let calldata = encode_take_calldata(
        &offer,
        &ratifier_data,
        units,
        &taker,
        &receiver,
        &taker_callback,
        &taker_callback_data,
    );

    let decoded = decode_take_calldata(&calldata).unwrap();

    assert_eq!(decoded.offer, offer, "offer mismatch");
    assert_eq!(decoded.units, units);
    assert_eq!(decoded.taker, taker);
    assert_eq!(decoded.receiver_if_taker_is_seller, receiver);
    assert_eq!(decoded.taker_callback, taker_callback);
    assert_eq!(decoded.taker_callback_data, taker_callback_data);
    assert_eq!(decoded.ratifier_data_raw, ratifier_data);
    let RatifierPayload::Ecrecover(rd) = &decoded.ratifier_data else {
        panic!(
            "expected ecrecover ratifier data, got {:?}",
            decoded.ratifier_data
        );
    };
    assert_eq!(rd.sig, sig);
    assert_eq!(rd.root, tree.root());
    assert_eq!(rd.proof, proof);
}

#[test]
fn repay_withdraw_calldata_round_trips() {
    let market = offer_for([0x42; 20]).market;
    let on_behalf = [0x77; 20];
    let collateral_receiver = [0x88; 20];
    let referral_fee_recipient = [0x99; 20];
    let loan_token_permit = TokenPermit {
        kind: 2,
        data: vec![0xde, 0xad, 0xbe, 0xef, 0x01],
    };
    let collateral_withdrawals = vec![
        CollateralWithdrawal {
            collateral_index: U256::ZERO,
            assets: U256::from(250u64),
        },
        CollateralWithdrawal {
            collateral_index: U256::ZERO,
            assets: U256::from(500u64),
        },
    ];
    let repay_assets = U256::from(1_000u64);
    let referral_fee_pct = U256::from(25_000_000_000_000_000u64);
    let deadline = U256::from(4_000_000_000u64);

    let calldata = encode_repay_withdraw_collateral_calldata(
        &market,
        repay_assets,
        &on_behalf,
        &loan_token_permit,
        &collateral_withdrawals,
        &collateral_receiver,
        referral_fee_pct,
        &referral_fee_recipient,
        deadline,
    );
    let decoded = decode_repay_withdraw_collateral_calldata(&calldata).unwrap();

    assert_eq!(
        decoded,
        RepayWithdrawCall {
            market,
            repay_assets,
            on_behalf,
            loan_token_permit,
            collateral_withdrawals,
            collateral_receiver,
            referral_fee_pct,
            referral_fee_recipient,
            deadline,
        }
    );
}

#[test]
fn setter_ratifier_data_round_trips() {
    let root = keccak(b"root");
    let proof = vec![keccak(b"a"), keccak(b"b"), keccak(b"c")];
    let encoded = encode_setter_ratifier_data(&root, 5, &proof);
    let decoded = decode_setter_ratifier_data(&encoded).unwrap();
    assert_eq!(decoded.root, root);
    assert_eq!(decoded.leaf_index, 5);
    assert_eq!(decoded.proof, proof);
}

#[test]
fn any_ratifier_data_discriminates_both_layouts() {
    let signer = LocalSigner::from_bytes(&[0x7c; 32]).unwrap();
    let root = keccak(b"root");
    let proof = vec![keccak(b"a")];
    let sig = signer.sign_digest(&keccak(b"digest")).unwrap();

    // EcrecoverRatifier layout: first word is a zero-padded uint8 v.
    let ec = encode_ratifier_data(&sig, &root, 1, &proof);
    match decode_any_ratifier_data(&ec).unwrap() {
        RatifierPayload::Ecrecover(rd) => {
            assert_eq!(rd.sig, sig);
            assert_eq!(rd.root, root);
        }
        other => panic!("expected ecrecover, got {other:?}"),
    }

    // SetterRatifier layout: first word is the root itself (a keccak hash).
    let setter = encode_setter_ratifier_data(&root, 1, &proof);
    match decode_any_ratifier_data(&setter).unwrap() {
        RatifierPayload::Setter(rd) => {
            assert_eq!(rd.root, root);
            assert_eq!(rd.leaf_index, 1);
            assert_eq!(rd.proof, proof);
        }
        other => panic!("expected setter, got {other:?}"),
    }
}

#[test]
fn take_with_setter_ratifier_data_decodes() {
    let offer = offer_for([0x42; 20]);
    let tree = OfferTree::build(vec![hash_offer(&offer)]).unwrap();
    let ratifier_data = encode_setter_ratifier_data(&tree.root(), 0, &tree.proof(0).unwrap());
    let calldata = encode_take_calldata(
        &offer,
        &ratifier_data,
        U256::from(1u64),
        &[0x77; 20],
        &[0x88; 20],
        &[0u8; 20],
        &[],
    );
    let d = decode_take_calldata(&calldata).unwrap();
    let rd = &d.ratifier_data;
    assert_eq!(rd.sig(), None, "setter data carries no signature");
    assert!(verify_leaf(
        rd.root(),
        &hash_offer(&d.offer),
        rd.leaf_index(),
        rd.proof()
    ));
}

/// The whole point of the tool: from decoded calldata alone, reproduce the signed root and
/// confirm the recovered signer is the offer's maker.
#[test]
fn decoded_calldata_verifies_signature_and_root() {
    let signer = LocalSigner::from_bytes(&[0x7c; 32]).unwrap();
    let offer = offer_for(signer.address());
    let ratifier = offer.ratifier;
    let chain_id = word_from_u64(31337);

    let tree = OfferTree::build(vec![hash_offer(&offer)]).unwrap();
    let sig = signer
        .sign_digest(&tree_digest(
            tree.root(),
            tree.height(),
            chain_id,
            &ratifier,
        ))
        .unwrap();
    let ratifier_data = encode_ratifier_data(&sig, &tree.root(), 0, &tree.proof(0).unwrap());
    let calldata = encode_take_calldata(
        &offer,
        &ratifier_data,
        U256::from(1u64),
        &[0x77; 20],
        &[0x88; 20],
        &[0u8; 20],
        &[],
    );

    let d = decode_take_calldata(&calldata).unwrap();
    let RatifierPayload::Ecrecover(rd) = &d.ratifier_data else {
        panic!("expected ecrecover ratifier data");
    };

    // 1. The leaf hashes to a member of the signed root.
    let leaf = hash_offer(&d.offer);
    assert!(
        verify_leaf(&rd.root, &leaf, rd.leaf_index, &rd.proof),
        "leaf not under signed root"
    );
    // 2. The recovered signer is the maker, over exactly this digest.
    let digest = tree_digest(rd.root, rd.proof.len(), chain_id, &ratifier);
    assert_eq!(recover(&digest, &rd.sig).as_ref(), Some(&d.offer.maker));
    // 3. The library's one-shot verify agrees.
    assert!(verify(
        &d.offer,
        &rd.root,
        rd.leaf_index,
        &rd.proof,
        &rd.sig,
        chain_id,
        &ratifier,
        &d.offer.maker,
    ));
}

#[test]
fn cancel_root_round_trips() {
    let maker = [0x33; 20];
    let root = word_from_u64(0xdead_beef);
    let calldata = encode_cancel_root_calldata(&maker, &root);
    let (m, r) = decode_cancel_root_calldata(&calldata).unwrap();
    assert_eq!(m, maker);
    assert_eq!(r, root);
}

#[test]
fn wrong_selector_is_rejected() {
    let cancel = encode_cancel_root_calldata(&[0x33; 20], &word_from_u64(1));
    match decode_take_calldata(&cancel) {
        Err(DecodeError::BadSelector(s)) => assert_eq!(s, CANCEL_ROOT_SELECTOR),
        other => panic!("expected BadSelector, got {other:?}"),
    }
}

#[test]
fn truncated_calldata_errors_not_panics() {
    // Valid selector, but no args.
    let mut bytes = TAKE_SELECTOR.to_vec();
    bytes.extend_from_slice(&[0u8; 16]);
    assert!(decode_take_calldata(&bytes).is_err());
    // Shorter than a selector.
    assert!(decode_take_calldata(&[0x6a, 0x14]).is_err());
}

#[test]
fn repay_withdraw_malformed_calldata_is_rejected() {
    let market = offer_for([0x42; 20]).market;
    let calldata = encode_repay_withdraw_collateral_calldata(
        &market,
        U256::from(1u64),
        &[0x77; 20],
        &TokenPermit {
            kind: 0,
            data: Vec::new(),
        },
        &[],
        &[0x88; 20],
        U256::ZERO,
        &[0u8; 20],
        U256::MAX,
    );

    let truncated = calldata[..4 + 9 * 32].to_vec();
    assert!(matches!(
        decode_repay_withdraw_collateral_calldata(&truncated),
        Err(DecodeError::TooShort { .. })
    ));

    // A copied transaction can be truncated at any byte boundary; none should panic.
    for len in 0..calldata.len() {
        let _ = decode_repay_withdraw_collateral_calldata(&calldata[..len]);
    }

    let mut overflowing_offset = calldata.clone();
    overflowing_offset[4..36].copy_from_slice(&[0xff; 32]);
    assert_eq!(
        decode_repay_withdraw_collateral_calldata(&overflowing_offset),
        Err(DecodeError::IntegerOverflow)
    );

    let cancel = encode_cancel_root_calldata(&[0x33; 20], &word_from_u64(1));
    assert_eq!(
        decode_repay_withdraw_collateral_calldata(&cancel),
        Err(DecodeError::BadSelector(CANCEL_ROOT_SELECTOR))
    );
}

#[test]
fn set_is_root_ratified_round_trips() {
    let root = keccak(b"root");
    let calldata = encode_set_is_root_ratified_calldata(&[0x42; 20], &root, true);
    assert_eq!(calldata[..4], SET_IS_ROOT_RATIFIED_SELECTOR);
    let d = decode_set_is_root_ratified_calldata(&calldata).unwrap();
    assert_eq!(
        d,
        RatifyCall {
            maker: [0x42; 20],
            root,
            ratified: true,
        }
    );
    // The unratify direction too.
    let revoke = encode_set_is_root_ratified_calldata(&[0x42; 20], &root, false);
    assert!(
        !decode_set_is_root_ratified_calldata(&revoke)
            .unwrap()
            .ratified
    );
}

#[test]
fn production_ratify_calldata_decodes() {
    // A real SetterRatifier.setIsRootRatified transaction from Base (tx 0x0901ae10...,
    // block 48891514), sent by the maker itself and carrying the app's trailing metadata
    // tag - the root matches fill[0] of the production bundle fixture.
    let hex_str = include_str!("data/setter_ratify_prod.hex");
    let bytes = hex::decode(hex_str.trim().trim_start_matches("0x")).unwrap();
    let d = decode_set_is_root_ratified_calldata(&bytes).unwrap();
    let maker: Address = [
        0xd4, 0x18, 0x22, 0x4a, 0xe3, 0xc5, 0x10, 0xb6, 0x45, 0x11, 0x2f, 0xd9, 0x27, 0x5c, 0xcf,
        0xd5, 0x0f, 0x99, 0x6e, 0xe4,
    ];
    assert_eq!(d.maker, maker);
    assert!(d.ratified);

    // Cross-fixture anchor: the ratified root is exactly the root claimed by fill[0] of the
    // production taker bundle.
    let bundle_hex = include_str!("data/setter_bundle_full.hex");
    let bundle =
        decode_bundle_calldata(&hex::decode(bundle_hex.trim().trim_start_matches("0x")).unwrap())
            .unwrap();
    assert_eq!(*bundle.fills[0].ratifier_data.root(), d.root);
}
