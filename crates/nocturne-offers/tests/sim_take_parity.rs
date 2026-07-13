//! Contract-anchored parity for the take math.
//!
//! The expected values below come from `fixtures/GenTake.t.sol`, which runs a **real**
//! `Midnight.take` through the full contract (rev f47568c9e45a9b70830b82a130b47393dcafec33) and
//! reads back the resulting amounts and position deltas. This promotes `settlement_fee` /
//! `take_amounts` / `simulate_take` from hand-verified to contract-anchored — the same standard
//! as the typehash, signing-digest, and tick-price parity tests.
//!
//! The scenario: a fresh buy offer (maker = buyer, taker = seller), tick 3372 (price 0.5 WAD),
//! ttm = 45 days (interpolates between the 30d/90d fee breakpoints), the on-chain max settlement
//! fee curve, and a nonzero continuous fee — so every branch (price, fee interpolation, buy-side
//! rounding, pending-fee accrual, consumption, deltas) is exercised at once.

use nocturne_offers::*;

// ---- constants emitted by fixtures/GenTake.t.sol ----
const NOW: u64 = 1_000_000;
const MATURITY: u64 = 4_888_000; // now + 45 days
const UNITS: u128 = 1_000_000;
const TICK: u64 = 3372;
const CONTINUOUS_FEE: u128 = 100_000_000;
const CBPS: [u16; 7] = [14, 14, 98, 417, 1250, 2500, 5000];

const EXPECT_SETTLEMENT_FEE: u64 = 625_250_000_000_000;
const EXPECT_BUYER_ASSETS: u64 = 500_000;
const EXPECT_SELLER_ASSETS: u64 = 499_374;
const EXPECT_BUYER_PENDING_FEE: u64 = 388;
const EXPECT_NEW_CONSUMED: u64 = 1_000_000;

// Addresses/group don't affect the amount math, so any well-formed market works.
fn offer() -> Offer {
    let market = MarketBuilder::new(1, [0x11; 20], [0x22; 20])
        .collateral([0x33; 20], U256::from(770_000_000_000_000_000u64), U256::from(300_000_000_000_000_000u64), [0x44; 20])
        .maturity(MATURITY)
        .build();
    OfferBuilder::new(market, [0x55; 20])
        .buy()
        .tick(TICK)
        .expiry(NOW + 200)
        .ratifier([0xbb; 20])
        .max_units(u128::MAX)
        .continuous_fee_cap(U256::MAX)
        .build()
}

#[test]
fn settlement_fee_matches_contract() {
    let fee = settlement_fee(CBPS, U256::from(MATURITY - NOW));
    assert_eq!(fee, U256::from(EXPECT_SETTLEMENT_FEE));
}

#[test]
fn take_amounts_match_contract() {
    let a = take_amounts(&offer(), U256::from(UNITS), NOW, CBPS).unwrap();
    assert_eq!(a.offer_price, U256::from(500_000_000_000_000_000u64), "offer price");
    assert_eq!(a.buyer_assets, U256::from(EXPECT_BUYER_ASSETS), "buyer assets");
    assert_eq!(a.seller_assets, U256::from(EXPECT_SELLER_ASSETS), "seller assets");
    assert_eq!(
        a.settlement_fee_assets,
        U256::from(EXPECT_BUYER_ASSETS - EXPECT_SELLER_ASSETS),
        "protocol fee assets"
    );
}

#[test]
fn simulate_take_matches_contract() {
    let ctx = SimCtx {
        now: NOW,
        market: SimMarket {
            tick_spacing: DEFAULT_TICK_SPACING,
            continuous_fee: CONTINUOUS_FEE,
            settlement_fee_cbp: CBPS,
            loss_factor_maxed: false,
        },
        consumed: 0,
        maker_position: Position::default(),  // buyer, fresh
        taker_position: Position::default(),  // seller, fresh
        taker_is_maker: false,
    };
    let out = simulate_take(&offer(), U256::from(UNITS), &ctx).unwrap();

    assert!(out.reverts.is_empty(), "take should succeed, got {:?}", out.reverts);
    assert_eq!(out.amounts.buyer_assets, U256::from(EXPECT_BUYER_ASSETS));
    assert_eq!(out.amounts.seller_assets, U256::from(EXPECT_SELLER_ASSETS));
    assert_eq!(out.buyer_credit_increase, U256::from(UNITS));
    assert_eq!(out.seller_debt_increase, U256::from(UNITS));
    assert_eq!(out.seller_credit_decrease, U256::ZERO);
    assert_eq!(out.buyer_pending_fee_increase, U256::from(EXPECT_BUYER_PENDING_FEE));
    assert_eq!(out.seller_pending_fee_decrease, U256::ZERO);
    assert_eq!(out.new_consumed, U256::from(EXPECT_NEW_CONSUMED));
}
