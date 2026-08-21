//! Differential vectors generated directly from @morpho-org/midnight-sdk 1.3.0 at
//! eb27628b88300665405e0bd0bde395b834cae969.

use nocturne::*;

fn market() -> Market {
    MarketBuilder::new(8453, [0x11; 20], [0x22; 20])
        .collateral(
            [0x33; 20],
            U256::from(770_000_000_000_000_000u128),
            U256::from(250_000_000_000_000_000u128),
            [0x44; 20],
        )
        .maturity(1_798_815_600)
        .rcf_threshold(U256::from(1_000u16))
        .build_checked()
        .unwrap()
}

fn offers() -> Vec<Offer> {
    [5_000u64, 5_004]
        .map(|tick| {
            OfferBuilder::new(market(), [0x55; 20])
                .buy()
                .tick(tick)
                .expiry(1_798_815_500)
                .ratifier([0xbb; 20])
                .max_assets(1_000_000)
                .build_checked()
                .unwrap()
        })
        .into()
}

#[test]
fn latest_typescript_hash_tree_digest_and_payload_are_compatible() {
    let offers = offers();
    assert_eq!(word_to_u256(&offers[0].continuous_fee_cap), U256::ZERO);

    let group = OfferGroup::create(offers).unwrap();
    assert_eq!(
        hex::encode(group.id),
        "6c7e3514e1309a6ce57d8145de6bb6088cb2824079be1c0113974a643ed69a9b"
    );
    let descriptor = OfferTree::from_entries([group]).unwrap();
    assert_eq!(
        descriptor.offers[..2]
            .iter()
            .map(|offer| hex::encode(hash_offer(offer)))
            .collect::<Vec<_>>(),
        [
            "70461adefbfc9b5da3545f64b8f24c67f2236f92e45972253550795f7ae1a1ee",
            "61f00b769b5dc7801bf128319cd0986d95d134129aa2af410782931084310fca",
        ]
    );
    assert_eq!(
        hex::encode(descriptor.tree.root()),
        "70e8b2240bf0425b585f624eb07cee1cad7e2e2075f6031cc6c94fd08c6b66d9"
    );
    assert_eq!(
        descriptor.tree.proof(0).unwrap(),
        vec![hash_offer(&descriptor.offers[1])]
    );
    assert_eq!(
        descriptor.tree.proof(1).unwrap(),
        vec![hash_offer(&descriptor.offers[0])]
    );
    assert_eq!(
        hex::encode(tree_digest(
            descriptor.tree.root(),
            descriptor.tree.height(),
            word_from_u64(8453),
            &[0xbb; 20],
        )),
        "735cb8720453dc8584799bca6f67aa7a61c49af5c1585b248f3f472afc7006cc"
    );

    // This complete gzip payload was emitted by the current TypeScript Payload.encode. Rust and
    // browser gzip headers differ, so interoperability is asserted by decoding canonical items.
    let typescript_payload = hex::decode(concat!(
        "01000000d01f8b08000000000000136360c00b14f04b3330119077c02fcd9240997ee607f8e51909",
        "c83330227342b10002faf1826cf3fd3cf8550877e4d4998a3c349895f3b4b6d1f55ef6368e9e4d4",
        "d0e95fb641885a77ba5d85d9b359b12fb81e1d3408aeadd580065f61300fc4e0e9419a0c88acc13",
        "c40290e595b000fce633e29505c66f017efdcc2ff0cb530c501c688c05e0d2c8b57edaebb9bcb80d",
        "66aed05df3f226aa980b164099f31998844c28d2ef805f7ac8970f3da3e5032560b47c40e60cc5f2",
        "61f559fc0a00c65f617dc0080000",
    ))
    .unwrap();
    let decoded = Payload::decode(&typescript_payload, None).unwrap();
    assert_eq!(decoded.len(), 2);
    assert_eq!(decoded[0].offer, descriptor.offers[0]);
    assert_eq!(decoded[0].ratifier_data, [0x12, 0x34]);
    assert_eq!(decoded[1].offer, descriptor.offers[1]);
    assert_eq!(decoded[1].ratifier_data, [0xab, 0xcd]);

    let rust_payload = Payload::encode(&[
        PayloadItem {
            offer: descriptor.offers[0].clone(),
            ratifier_data: vec![0x12, 0x34],
        },
        PayloadItem {
            offer: descriptor.offers[1].clone(),
            ratifier_data: vec![0xab, 0xcd],
        },
    ])
    .unwrap();
    if std::env::var_os("NOCTURNE_PRINT_PARITY_PAYLOAD").is_some() {
        println!("RUST_PAYLOAD=0x{}", hex::encode(&rust_payload));
    }
    assert_eq!(Payload::decode(&rust_payload, None).unwrap(), decoded);
}
