//! Tests for the assets<->units sizing helpers.
//!
//! Two guarantees:
//!   1. Contract-anchored parity. The `EXPECT_*` constants come from `fixtures/GenSizing.t.sol`,
//!      which calls the **real** `TakeAmountsLib.buyerAssetsToUnits` / `sellerAssetsToUnits` and
//!      `ConsumableUnitsLib.consumableUnits` against a live market (rev
//!      f47568c9e45a9b70830b82a130b47393dcafec33). This promotes the inverse sizing math from
//!      hand-verified to contract-anchored, the same standard as `sim_take_parity.rs`.
//!   2. Round-trip consistency with the forward path (`take_amounts`): sizing to a target asset
//!      amount and taking that many units moves the intended assets (exact on clean values, within
//!      the documented rounding otherwise).

use nocturne::*;

// ---- scenario, matching fixtures/GenSizing.t.sol ----
const NOW: u64 = 1_000_000;
const MATURITY: u64 = 4_888_000; // now + 45 days
const TICK: u64 = 3372; // price 0.5 WAD
const CBPS: [u16; 7] = [14, 14, 98, 417, 1250, 2500, 5000];
const ZERO_CBPS: [u16; 7] = [0; 7]; // fee-free: clean, exact round-trips

const TARGET: u64 = 500_000;
const MAX_ASSETS: u128 = 1_000_000;
const MAX_UNITS_CAP: u128 = 4_000_000;

// ---- values emitted by fixtures/GenSizing.t.sol (settlement fee 625_250_000_000_000) ----
const EXPECT_BUY_BUYER_UNITS: u64 = 1_000_000;
const EXPECT_BUY_SELLER_UNITS: u64 = 1_001_253;
const EXPECT_SELL_BUYER_UNITS: u64 = 998_751;
const EXPECT_SELL_SELLER_UNITS: u64 = 1_000_000;
const EXPECT_CONS_UNITS_CAPPED: u64 = 4_000_000;
const EXPECT_CONS_ASSETS_BUY: u64 = 2_000_000;
const EXPECT_CONS_ASSETS_SELL: u64 = 2_000_000;

// Addresses/group don't affect the amount math, so any well-formed market works.
fn offer(buy: bool, tick: u64, max_units: u128, max_assets: u128) -> Offer {
    let market = MarketBuilder::new(1, [0x11; 20], [0x22; 20])
        .collateral(
            [0x33; 20],
            U256::from(770_000_000_000_000_000u64),
            U256::from(300_000_000_000_000_000u64),
            [0x44; 20],
        )
        .maturity(MATURITY)
        .build();
    let b = OfferBuilder::new(market, [0x55; 20])
        .tick(tick)
        .expiry(NOW + 200)
        .ratifier([0xbb; 20])
        .continuous_fee_cap(U256::MAX);
    // The builder's cap setters are mutually exclusive, so set only the one in use.
    let b = if max_units > 0 {
        b.max_units(max_units)
    } else {
        b.max_assets(max_assets)
    };
    let b = if buy { b.buy() } else { b.sell() };
    b.build()
}

// ---------------------------------------------------------------------------
// Contract-anchored parity (fixtures/GenSizing.t.sol)
// ---------------------------------------------------------------------------

#[test]
fn assets_to_units_match_contract() {
    let buy = offer(true, TICK, u128::MAX, 0);
    let sell = offer(false, TICK, u128::MAX, 0);
    let t = U256::from(TARGET);

    assert_eq!(
        buyer_assets_to_units(&buy, t, NOW, CBPS).unwrap(),
        U256::from(EXPECT_BUY_BUYER_UNITS),
        "buy buyer_assets_to_units"
    );
    assert_eq!(
        seller_assets_to_units(&buy, t, NOW, CBPS).unwrap(),
        U256::from(EXPECT_BUY_SELLER_UNITS),
        "buy seller_assets_to_units"
    );
    assert_eq!(
        buyer_assets_to_units(&sell, t, NOW, CBPS).unwrap(),
        U256::from(EXPECT_SELL_BUYER_UNITS),
        "sell buyer_assets_to_units"
    );
    assert_eq!(
        seller_assets_to_units(&sell, t, NOW, CBPS).unwrap(),
        U256::from(EXPECT_SELL_SELLER_UNITS),
        "sell seller_assets_to_units"
    );
}

#[test]
fn consumable_units_match_contract() {
    // Units-capped (consumed = 0): the full cap remains.
    let units_capped = offer(true, TICK, MAX_UNITS_CAP, 0);
    assert_eq!(
        consumable_units(&units_capped, 0, NOW, CBPS).unwrap(),
        U256::from(EXPECT_CONS_UNITS_CAPPED),
        "units-capped"
    );
    // Assets-capped buy (consumed = 0): buyer_assets_to_units(maxAssets).
    let assets_buy = offer(true, TICK, 0, MAX_ASSETS);
    assert_eq!(
        consumable_units(&assets_buy, 0, NOW, CBPS).unwrap(),
        U256::from(EXPECT_CONS_ASSETS_BUY),
        "assets-capped buy"
    );
    // Assets-capped sell (consumed = 0): seller_assets_to_units(maxAssets).
    let assets_sell = offer(false, TICK, 0, MAX_ASSETS);
    assert_eq!(
        consumable_units(&assets_sell, 0, NOW, CBPS).unwrap(),
        U256::from(EXPECT_CONS_ASSETS_SELL),
        "assets-capped sell"
    );
}

#[test]
fn sdk_consumable_units_match_market_quote_vectors() {
    let now = 1_900_000_000;
    let cbps = [1, 2, 3, 4, 5, 6, 7];
    let market_fee = U256::from(200_000_000u64);
    let mut buy = offer(true, 3_000, 0, 1_000_000);
    buy.market.maturity = word_from_u64(2_000_000_000);
    buy.continuous_fee_cap = word_from_u64(300_000_000);

    assert_eq!(
        get_consumable_units(&buy, 0, now, cbps, market_fee).unwrap(),
        U256::from(7_393_236u64)
    );
    assert_eq!(
        get_consumable_units(&buy, 500_000, now, cbps, market_fee).unwrap(),
        U256::from(3_696_621u64)
    );

    let mut sell = buy.clone();
    sell.buy = false;
    sell.receiver_if_maker_is_seller = sell.maker;
    assert_eq!(
        get_consumable_units(&sell, 500_000, now, cbps, market_fee).unwrap(),
        U256::from(3_696_614u64)
    );

    let mut units = buy.clone();
    units.max_units = 777_777;
    units.max_assets = 0;
    assert_eq!(
        get_consumable_units(&units, 123_456, now, cbps, market_fee).unwrap(),
        U256::from(654_321u64)
    );

    buy.continuous_fee_cap = word_from_u64(199_999_999);
    assert_eq!(
        get_consumable_units(&buy, 0, now, cbps, market_fee).unwrap(),
        U256::ZERO
    );
}

// ---------------------------------------------------------------------------
// consumable_units: consumption arithmetic (zeroFloorSub, no underflow)
// ---------------------------------------------------------------------------

#[test]
fn consumable_units_subtracts_consumption() {
    // Units-capped: remaining = max_units - consumed.
    let units_capped = offer(true, TICK, MAX_UNITS_CAP, 0);
    assert_eq!(
        consumable_units(&units_capped, 1_000_000, NOW, CBPS).unwrap(),
        U256::from(MAX_UNITS_CAP - 1_000_000),
    );

    // Assets-capped: remaining assets = max_assets - consumed, then converted to units.
    let consumed: u128 = 400_000;
    let assets_buy = offer(true, TICK, 0, MAX_ASSETS);
    let expected_buy =
        buyer_assets_to_units(&assets_buy, U256::from(MAX_ASSETS - consumed), NOW, CBPS).unwrap();
    assert_eq!(
        consumable_units(&assets_buy, consumed, NOW, CBPS).unwrap(),
        expected_buy
    );

    let assets_sell = offer(false, TICK, 0, MAX_ASSETS);
    let expected_sell =
        seller_assets_to_units(&assets_sell, U256::from(MAX_ASSETS - consumed), NOW, CBPS).unwrap();
    assert_eq!(
        consumable_units(&assets_sell, consumed, NOW, CBPS).unwrap(),
        expected_sell
    );
}

#[test]
fn consumable_units_fully_consumed_is_zero() {
    // Over-consumption must floor at zero, never underflow.
    let units_capped = offer(true, TICK, MAX_UNITS_CAP, 0);
    assert_eq!(
        consumable_units(&units_capped, MAX_UNITS_CAP + 1, NOW, CBPS).unwrap(),
        U256::ZERO
    );

    let assets_buy = offer(true, TICK, 0, MAX_ASSETS);
    assert_eq!(
        consumable_units(&assets_buy, MAX_ASSETS + 1, NOW, CBPS).unwrap(),
        U256::ZERO
    );

    let assets_sell = offer(false, TICK, 0, MAX_ASSETS);
    assert_eq!(
        consumable_units(&assets_sell, MAX_ASSETS, NOW, CBPS).unwrap(),
        U256::ZERO
    );
}

// ---------------------------------------------------------------------------
// Round-trip against the forward take math (take_amounts)
// ---------------------------------------------------------------------------

// Fee-free, tick 3372 (price exactly 0.5 WAD), even unit count: the inverse is exact.
#[test]
fn round_trip_exact_when_fee_free() {
    let units = U256::from(1_000_000u64);
    for buy in [true, false] {
        let o = offer(buy, TICK, u128::MAX, 0);
        let a = take_amounts(&o, units, NOW, ZERO_CBPS).unwrap();
        assert_eq!(
            buyer_assets_to_units(&o, a.buyer_assets, NOW, ZERO_CBPS).unwrap(),
            units,
            "buyer round-trip (buy={buy})"
        );
        assert_eq!(
            seller_assets_to_units(&o, a.seller_assets, NOW, ZERO_CBPS).unwrap(),
            units,
            "seller round-trip (buy={buy})"
        );
    }
}

// With a settlement fee the inverse recovers the same unit count up to the documented rounding.
#[test]
fn round_trip_within_rounding_with_fee() {
    let units = U256::from(1_000_000u64);
    let one = U256::from(1u64);
    let close = |a: U256, b: U256| if a > b { a - b <= one } else { b - a <= one };

    for buy in [true, false] {
        let o = offer(buy, TICK, u128::MAX, 0);
        let a = take_amounts(&o, units, NOW, CBPS).unwrap();
        let ru_buyer = buyer_assets_to_units(&o, a.buyer_assets, NOW, CBPS).unwrap();
        let ru_seller = seller_assets_to_units(&o, a.seller_assets, NOW, CBPS).unwrap();
        assert!(
            close(ru_buyer, units),
            "buyer round-trip (buy={buy}): got {ru_buyer}"
        );
        assert!(
            close(ru_seller, units),
            "seller round-trip (buy={buy}): got {ru_seller}"
        );
    }
}

// Sizing to a target and taking that many units moves the intended assets (mirrors the contract's
// own e2e TakeAmountsTest: buyerAssets == targetBuyerAssets on clean values).
#[test]
fn sizing_to_target_hits_target_assets() {
    let t = U256::from(TARGET);
    // Buy offer, buyer side is fee-independent (buyer_price == offer_price) -> exact.
    let buy = offer(true, TICK, u128::MAX, 0);
    let units = buyer_assets_to_units(&buy, t, NOW, CBPS).unwrap();
    assert_eq!(
        take_amounts(&buy, units, NOW, CBPS).unwrap().buyer_assets,
        t
    );

    // Sell offer, seller side is fee-independent (seller_price == offer_price) -> exact.
    let sell = offer(false, TICK, u128::MAX, 0);
    let units = seller_assets_to_units(&sell, t, NOW, CBPS).unwrap();
    assert_eq!(
        take_amounts(&sell, units, NOW, CBPS).unwrap().seller_assets,
        t
    );
}

// ---------------------------------------------------------------------------
// Error paths - must return Err, never panic
// ---------------------------------------------------------------------------

#[test]
fn buyer_price_above_wad_errors() {
    // Sell offer at the top tick (price ~WAD) + a settlement fee pushes buyer_price > WAD.
    let sell = offer(false, MAX_TICK, u128::MAX, 0);
    let err = buyer_assets_to_units(&sell, U256::from(TARGET), NOW, CBPS).unwrap_err();
    assert_eq!(err, SizingError::PriceGreaterThanOne);
}

#[test]
fn out_of_range_tick_errors() {
    let bad = offer(true, MAX_TICK + 1, u128::MAX, 0);
    assert_eq!(
        buyer_assets_to_units(&bad, U256::from(TARGET), NOW, CBPS).unwrap_err(),
        SizingError::Sim(SimError::TickOutOfRange((MAX_TICK + 1) as u128)),
    );
    assert!(seller_assets_to_units(&bad, U256::from(TARGET), NOW, CBPS).is_err());
    assert!(consumable_units(&offer(true, MAX_TICK + 1, 0, MAX_ASSETS), 0, NOW, CBPS).is_err());
}

#[test]
fn settlement_fee_exceeding_price_errors() {
    // Buy offer at tick 0 (tiny price) with a nonzero fee: offer_price - fee underflows on-chain.
    let buy = offer(true, 0, u128::MAX, 0);
    assert_eq!(
        buyer_assets_to_units(&buy, U256::from(TARGET), NOW, CBPS).unwrap_err(),
        SizingError::Sim(SimError::SettlementFeeExceedsPrice),
    );
    assert_eq!(
        seller_assets_to_units(&buy, U256::from(TARGET), NOW, CBPS).unwrap_err(),
        SizingError::Sim(SimError::SettlementFeeExceedsPrice),
    );
}
