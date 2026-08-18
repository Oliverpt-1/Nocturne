use nocturne::{
    canonical_market, hash_market, market_id, word_from_u64, Address, CollateralParams, Market,
    U256,
};

fn address(value: &str) -> Address {
    hex::decode(value.trim_start_matches("0x"))
        .unwrap()
        .try_into()
        .unwrap()
}

fn word(value: U256) -> [u8; 32] {
    value.to_be_bytes()
}

fn fixture_market() -> Market {
    Market {
        chain_id: word_from_u64(8453),
        midnight: address("0xAdedD8ab6dE832766Fedf0FaC4992E5C4D3EA18A"),
        loan_token: address("0x0000000000000000000000000000000000006000"),
        collateral_params: vec![CollateralParams {
            token: address("0x0000000000000000000000000000000000007000"),
            lltv: word(U256::from(770_000_000_000_000_000u64)),
            liquidation_cursor: word(U256::from(250_000_000_000_000_000u64)),
            oracle: address("0x0000000000000000000000000000000000008000"),
        }],
        maturity: word_from_u64(2_000),
        rcf_threshold: word_from_u64(0),
        enter_gate: [0; 20],
        liquidator_gate: [0; 20],
    }
}

#[test]
fn market_hash_and_id_match_protocol_vectors() {
    let market = fixture_market();
    assert_eq!(
        hex::encode(hash_market(&market)),
        "0c2a140996f0e000f896b45764bcd789d291ac30247d4228cc9f4bc6c6eda451"
    );
    assert_eq!(
        hex::encode(market_id(&market)),
        "994ce88b951db7a30742bb05a4dedd42dd42ce6633884ea82d7d525b1a56ed1f"
    );
}

#[test]
fn market_identity_is_independent_of_collateral_input_order() {
    let mut market = fixture_market();
    market.collateral_params.push(CollateralParams {
        token: address("0x0000000000000000000000000000000000001000"),
        lltv: word_from_u64(1),
        liquidation_cursor: word_from_u64(2),
        oracle: address("0x0000000000000000000000000000000000002000"),
    });
    let mut reversed = market.clone();
    reversed.collateral_params.reverse();

    assert_eq!(hash_market(&market), hash_market(&reversed));
    assert_eq!(market_id(&market), market_id(&reversed));

    let canonical = canonical_market(&market);
    assert!(canonical
        .collateral_params
        .windows(2)
        .all(|pair| pair[0].token < pair[1].token));
}
