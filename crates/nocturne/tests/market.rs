use nocturne::{
    canonical_market, encode_market_params, hash_market, market_id, word_from_u64, Address,
    CollateralParams, Market, MarketBuildError, MarketBuilder, MAX_COLLATERALS, U256,
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

fn token(last: u8) -> Address {
    let mut value = [0u8; 20];
    value[19] = last;
    value
}

fn builder_market(collateral_tokens: &[u8]) -> Market {
    let mut builder = MarketBuilder::new(8453, token(0x10), token(0x20))
        .maturity(2_000_000_000)
        .rcf_threshold(U256::from(1_000u64));
    for &last in collateral_tokens {
        builder = builder.collateral(
            token(last),
            U256::from(770_000_000_000_000_000u64) + U256::from(last),
            U256::from(250_000_000_000_000_000u64),
            token(last + 1),
        );
    }
    builder.build()
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

#[test]
fn market_builder_canonicalizes_collateral_everywhere() {
    let sorted = builder_market(&[0x30, 0x50, 0x70]);
    let unsorted = builder_market(&[0x70, 0x30, 0x50]);

    assert_eq!(unsorted, sorted);
    assert_eq!(hash_market(&unsorted), hash_market(&sorted));
    assert_eq!(market_id(&unsorted), market_id(&sorted));
    assert_eq!(
        encode_market_params(&unsorted),
        encode_market_params(&sorted)
    );
}

#[test]
fn raw_market_build_also_sorts_collateral() {
    let market = MarketBuilder::new(8453, token(0x10), token(0x20))
        .collateral(token(0x70), U256::from(1u64), U256::from(1u64), token(0x71))
        .collateral(token(0x30), U256::from(1u64), U256::from(1u64), token(0x31))
        .build();
    assert_eq!(
        market
            .collateral_params
            .iter()
            .map(|collateral| collateral.token)
            .collect::<Vec<_>>(),
        vec![token(0x30), token(0x70)]
    );
}

#[test]
fn checked_market_build_rejects_invalid_collateral_structure() {
    assert_eq!(
        MarketBuilder::new(8453, token(0x10), token(0x20))
            .build_checked()
            .unwrap_err(),
        MarketBuildError::NoCollateralParams
    );

    assert_eq!(
        MarketBuilder::new(8453, token(0x10), token(0x20))
            .collateral([0u8; 20], U256::from(1u64), U256::from(1u64), token(1))
            .build_checked()
            .unwrap_err(),
        MarketBuildError::ZeroCollateralToken
    );

    assert_eq!(
        MarketBuilder::new(8453, token(0x10), token(0x20))
            .collateral(token(0x30), U256::from(1u64), U256::from(1u64), token(0x31))
            .collateral(token(0x30), U256::from(2u64), U256::from(2u64), token(0x32))
            .build_checked()
            .unwrap_err(),
        MarketBuildError::DuplicateCollateralToken(token(0x30))
    );

    let oversized = (1..=MAX_COLLATERALS + 1).fold(
        MarketBuilder::new(8453, token(0x10), token(0x20)),
        |builder, index| {
            let mut collateral = [0u8; 20];
            collateral[18..].copy_from_slice(&(index as u16).to_be_bytes());
            builder.collateral(collateral, U256::from(1u64), U256::from(1u64), token(0x40))
        },
    );
    assert_eq!(
        oversized.build_checked().unwrap_err(),
        MarketBuildError::TooManyCollateralParams(MAX_COLLATERALS + 1)
    );
}

#[test]
fn checked_market_build_rejects_invalid_collateral_risk_parameters() {
    let wad = U256::from(1_000_000_000_000_000_000u128);
    let collateral = token(0x30);

    assert_eq!(
        MarketBuilder::new(8453, token(0x10), token(0x20))
            .collateral(collateral, wad + U256::from(1u8), U256::ZERO, token(0x31))
            .build_checked()
            .unwrap_err(),
        MarketBuildError::InvalidLltv(collateral)
    );
    assert_eq!(
        MarketBuilder::new(8453, token(0x10), token(0x20))
            .collateral(collateral, wad, wad, token(0x31))
            .build_checked()
            .unwrap_err(),
        MarketBuildError::InvalidLiquidationCursor(collateral)
    );
    assert_eq!(
        MarketBuilder::new(8453, token(0x10), token(0x20))
            .collateral(collateral, U256::ZERO, wad - U256::from(1u8), token(0x31),)
            .build_checked()
            .unwrap_err(),
        MarketBuildError::InvalidMaxLif(collateral)
    );
    assert_eq!(
        MarketBuilder::new(8453, token(0x10), token(0x20))
            .collateral(
                collateral,
                U256::from(950_000_000_000_000_000u128),
                U256::from(990_000_000_000_000_000u128),
                token(0x31),
            )
            .build_checked()
            .unwrap_err(),
        MarketBuildError::MaxLifTooHigh(collateral)
    );
}
