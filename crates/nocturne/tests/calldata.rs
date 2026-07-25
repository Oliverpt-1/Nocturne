//! Round-trip tests for the calldata decoders (`decode_take_calldata`, `decode_ratifier_data`,
//! `decode_cancel_root_calldata`).
//!
//! The encoders (`encode_take_calldata` / `encode_ratifier_data` / `encode_cancel_root_calldata`)
//! are already asserted byte-for-byte against the Midnight contracts in `tests/codec.rs`, so
//! encoding with them and decoding back is a parity check on the decoders: if a decoder disagrees
//! with its verified inverse, the round-trip breaks.

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
    let proof = tree.proof(leaf_index);
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
    let proof = tree.proof(0);
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
    let ratifier_data = encode_setter_ratifier_data(&tree.root(), 0, &tree.proof(0));
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
    let ratifier_data = encode_ratifier_data(&sig, &tree.root(), 0, &tree.proof(0));
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
