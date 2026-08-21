use nocturne::*;

fn word(value: u128) -> Word {
    let mut result = [0u8; 32];
    result[16..].copy_from_slice(&value.to_be_bytes());
    result
}

fn offer() -> Offer {
    Offer {
        market: Market {
            chain_id: word(8_453),
            midnight: [0x10; 20],
            loan_token: [0x20; 20],
            collateral_params: vec![CollateralParams {
                token: [0x30; 20],
                lltv: word(770_000_000_000_000_000),
                liquidation_cursor: word(250_000_000_000_000_000),
                oracle: [0x40; 20],
            }],
            // 1970-01-01 15:00:00 UTC. Small timestamps keep boundary tests readable.
            maturity: word(54_000),
            rcf_threshold: word(0),
            enter_gate: [0u8; 20],
            liquidator_gate: [0u8; 20],
        },
        buy: true,
        maker: [0x50; 20],
        start: word(1_000),
        expiry: word(3_600),
        tick: word(5_000),
        group: [0x60; 32],
        callback: [0u8; 20],
        callback_data: vec![0xaa, 0xbb],
        receiver_if_maker_is_seller: [0u8; 20],
        ratifier: [0x70; 20],
        reduce_only: false,
        max_units: 100,
        max_assets: 0,
        continuous_fee_cap: word(123),
    }
}

fn item() -> PayloadItem {
    PayloadItem {
        offer: offer(),
        ratifier_data: vec![0xde, 0xad, 0xbe, 0xef],
    }
}

#[test]
fn payload_round_trips_every_offer_field() {
    let items = vec![item(), item()];
    let payload = Payload::encode(&items).unwrap();

    assert_eq!(payload[0], PAYLOAD_VERSION);
    assert_eq!(Payload::decode(&payload, None).unwrap(), items);
}

#[test]
fn bounded_attribution_suffix_is_ignored() {
    let expected = vec![item()];
    let mut payload = Payload::encode(&expected).unwrap();
    payload.extend([0xa5; MAX_ATTRIBUTION_SUFFIX_BYTES]);
    assert_eq!(Payload::decode(&payload, None).unwrap(), expected);

    payload.push(0xa5);
    assert!(matches!(
        Payload::decode(&payload, None),
        Err(PayloadError::AttributionTooLarge(257))
    ));
}

#[test]
fn malformed_frames_and_item_limits_fail_cleanly() {
    assert!(matches!(
        Payload::decode(&[], None),
        Err(PayloadError::HeaderTooShort)
    ));
    assert!(matches!(
        Payload::decode(&[2, 0, 0, 0, 0], None),
        Err(PayloadError::InvalidVersion(2))
    ));
    assert!(matches!(
        Payload::decode(&[1, 0, 0, 0, 1], None),
        Err(PayloadError::Truncated)
    ));

    let payload = Payload::encode(&[item(), item()]).unwrap();
    assert!(matches!(
        Payload::decode(&payload, Some(1)),
        Err(PayloadError::TooManyItems(1))
    ));
    assert!(matches!(
        Payload::decode(&payload, Some(0)),
        Err(PayloadError::TooManyItems(0))
    ));
}

#[test]
fn empty_and_noncanonical_offers_are_rejected() {
    assert!(matches!(Payload::encode(&[]), Err(PayloadError::Empty)));

    let mut unsorted = item();
    unsorted
        .offer
        .market
        .collateral_params
        .push(CollateralParams {
            token: [0x2f; 20],
            lltv: word(700_000_000_000_000_000),
            liquidation_cursor: word(1),
            oracle: [0x41; 20],
        });
    assert!(matches!(
        Payload::encode(&[unsorted]),
        Err(PayloadError::InvalidOffer(
            "collaterals must be sorted and unique"
        ))
    ));

    let mut invalid_cap = item();
    invalid_cap.offer.max_assets = 1;
    assert!(matches!(
        Payload::encode(&[invalid_cap]),
        Err(PayloadError::InvalidOffer(
            "exactly one offer cap must be non-zero"
        ))
    ));
}

#[test]
fn exact_zero_padding_offer_is_supported() {
    let padding = Offer {
        market: Market {
            chain_id: [0; 32],
            midnight: [0; 20],
            loan_token: [0; 20],
            collateral_params: vec![],
            maturity: [0; 32],
            rcf_threshold: [0; 32],
            enter_gate: [0; 20],
            liquidator_gate: [0; 20],
        },
        buy: false,
        maker: [0; 20],
        start: [0; 32],
        expiry: [0; 32],
        tick: [0; 32],
        group: [0; 32],
        callback: [0; 20],
        callback_data: vec![],
        receiver_if_maker_is_seller: [0; 20],
        ratifier: [0; 20],
        reduce_only: false,
        max_units: 0,
        max_assets: 0,
        continuous_fee_cap: [0; 32],
    };
    let items = vec![
        item(),
        PayloadItem {
            offer: padding,
            ratifier_data: vec![],
        },
    ];
    let payload = Payload::encode(&items).unwrap();
    assert_eq!(Payload::decode(&payload, None).unwrap(), items);
}
