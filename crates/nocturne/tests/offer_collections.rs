use nocturne::{
    empty_offer, hash_offer, offer_group_id, word_from_u64, CollateralParams, GroupError, Market,
    Offer, OfferGroup, OfferTree, U256,
};

fn addr(last: u8) -> [u8; 20] {
    let mut address = [0; 20];
    address[19] = last;
    address
}

fn fixture_market() -> Market {
    Market {
        chain_id: word_from_u64(8453),
        midnight: addr(0x10),
        loan_token: addr(0x20),
        collateral_params: vec![
            CollateralParams {
                token: addr(0x60),
                lltv: U256::from(770_000_000_000_000_000u64).to_be_bytes(),
                liquidation_cursor: U256::from(250_000_000_000_000_000u64).to_be_bytes(),
                oracle: addr(0x70),
            },
            CollateralParams {
                token: addr(0x80),
                lltv: U256::from(860_000_000_000_000_000u64).to_be_bytes(),
                liquidation_cursor: U256::from(500_000_000_000_000_000u64).to_be_bytes(),
                oracle: addr(0x90),
            },
        ],
        maturity: word_from_u64(2_000_000_000),
        rcf_threshold: word_from_u64(1_000),
        enter_gate: [0; 20],
        liquidator_gate: addr(0xa0),
    }
}

fn fixture_offers() -> Vec<Offer> {
    [1_000u64, 2_000, 3_000]
        .into_iter()
        .enumerate()
        .map(|(index, tick)| Offer {
            market: fixture_market(),
            buy: true,
            maker: addr(0x30),
            start: word_from_u64(100 + index as u64 * 100),
            expiry: word_from_u64(10_000 + index as u64 * 100),
            tick: word_from_u64(tick),
            group: [0; 32],
            callback: if index == 1 { addr(0xb0) } else { [0; 20] },
            callback_data: if index == 1 {
                vec![1, 2, 3, 4, 5]
            } else {
                Vec::new()
            },
            receiver_if_maker_is_seller: [0; 20],
            ratifier: addr(0x40),
            reduce_only: index == 2,
            max_units: 0,
            max_assets: 1_000_000,
            continuous_fee_cap: word_from_u64(300_000_000),
        })
        .collect()
}

#[test]
fn grouped_and_padded_tree_matches_protocol_vectors() {
    let offers = fixture_offers();
    assert_eq!(
        hex::encode(offer_group_id(&offers).unwrap()),
        "aa4f2293102020a46ec999ae7a8d8cc8d5ede4fcecf1479e648fb45b93cb4cc7"
    );

    let group = OfferGroup::create(offers).unwrap();
    assert!(group.offers.iter().all(|offer| offer.group == group.id));
    let descriptor = OfferTree::from_entries([group]).unwrap();

    assert_eq!(descriptor.offers.len(), 4);
    assert_eq!(descriptor.offers[3], empty_offer());
    assert_eq!(descriptor.tree.height(), 2);
    assert_eq!(
        hex::encode(descriptor.tree.root()),
        "e3fc4fdcec1fd83b9d893c7f71640e0ae6ae96989f52b1ec7813793316e343b3"
    );
    assert_eq!(
        hex::encode(hash_offer(&descriptor.offers[3])),
        "59931827d7e4bea2472434c9cefa6baeb450425181a8e4bc73ebe59f9bcf248e"
    );
}

#[test]
fn group_id_does_not_depend_on_offer_order_or_stale_group_values() {
    let offers = fixture_offers();
    let expected = offer_group_id(&offers).unwrap();
    let mut reversed = offers.clone();
    reversed.reverse();
    for offer in &mut reversed {
        offer.group = [0xff; 32];
    }
    assert_eq!(offer_group_id(&reversed).unwrap(), expected);
}

#[test]
fn group_rejects_mismatched_consumption_caps() {
    let mut offers = fixture_offers();
    offers[1].max_assets += 1;
    assert_eq!(
        OfferGroup::create(offers).unwrap_err(),
        GroupError::CapMismatch
    );
}

#[test]
fn standalone_offers_receive_distinct_singleton_groups() {
    let offers = fixture_offers();
    let descriptor = OfferTree::from_entries(offers.clone()).unwrap();
    assert_eq!(descriptor.offers.len(), 4);
    for (input, grouped) in offers.iter().zip(&descriptor.offers) {
        assert_eq!(
            grouped.group,
            offer_group_id(std::slice::from_ref(input)).unwrap()
        );
    }
}
