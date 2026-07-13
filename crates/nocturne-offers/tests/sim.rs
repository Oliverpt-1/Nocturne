//! Behavior tests for the take simulator: settlement fee interpolation, take amounts, and
//! full simulate_take deltas / revert reasons.
//!
//! `tick_to_price` parity vs the real TickLib lives in `sim_parity.rs`. The settlement-fee and
//! amount math here is checked against values computed by hand from the exact `Midnight.take`
//! formulas.

use nocturne_offers::*;

const MIDNIGHT: Address = [0x11; 20];
const MAKER: Address = [0x55; 20];
const RATIFIER: Address = [0xbb; 20];
const CBP: u128 = 1_000_000_000_000; // 1e12
const DAY: u64 = 86_400;

fn wad(x: u64) -> U256 {
    U256::from(x) * U256::from(1_000_000_000_000_000_000u64)
}

fn market() -> Market {
    MarketBuilder::new(1, MIDNIGHT, [0x22; 20])
        .collateral([0x33; 20], wad_frac(), U256::from(1u64), [0x44; 20])
        .maturity(2_000_000_000)
        .build()
}

fn wad_frac() -> U256 {
    U256::from(860_000_000_000_000_000u64)
}

// ---- settlement_fee ----

#[test]
fn settlement_fee_beyond_360d_is_cbp6() {
    let cbps = [1, 2, 3, 4, 5, 6, 100];
    let fee = settlement_fee(cbps, U256::from(400 * DAY));
    assert_eq!(fee, U256::from(100u128 * CBP));
}

#[test]
fn settlement_fee_at_breakpoints_is_exact() {
    let cbps = [10, 20, 30, 40, 50, 60, 70];
    // ttm == 0 (post-maturity) -> 0d breakpoint (cbp0).
    assert_eq!(settlement_fee(cbps, U256::ZERO), U256::from(10u128 * CBP));
    // ttm == 1 day -> cbp1.
    assert_eq!(settlement_fee(cbps, U256::from(DAY)), U256::from(20u128 * CBP));
    // ttm == 7 days -> cbp2.
    assert_eq!(settlement_fee(cbps, U256::from(7 * DAY)), U256::from(30u128 * CBP));
    // ttm == 360 days -> cbp6.
    assert_eq!(settlement_fee(cbps, U256::from(360 * DAY)), U256::from(70u128 * CBP));
}

#[test]
fn settlement_fee_interpolates_linearly() {
    // Halfway between 0d (cbp0=0) and 1d (cbp1=100) -> 50 * CBP.
    let cbps = [0, 100, 0, 0, 0, 0, 0];
    let fee = settlement_fee(cbps, U256::from(DAY / 2));
    assert_eq!(fee, U256::from(50u128 * CBP));
}

// ---- take_amounts ----

#[test]
fn take_amounts_zero_fee_buy() {
    // tick 3372 -> price 0.5 WAD; zero settlement fee.
    let offer = OfferBuilder::new(market(), MAKER).buy().tick(3372).ratifier(RATIFIER).max_units(u128::MAX).build();
    let a = take_amounts(&offer, wad(1000), 1_000_000_000, [0; 7]).unwrap();
    assert_eq!(a.offer_price, U256::from(500_000_000_000_000_000u64));
    assert_eq!(a.buyer_price, a.seller_price); // fee = 0
    assert_eq!(a.buyer_assets, U256::from(500u64) * wad(1)); // 1000 units * 0.5
    assert_eq!(a.settlement_fee_assets, U256::ZERO);
}

#[test]
fn take_amounts_with_fee_buy_splits_correctly() {
    // price 0.5 WAD, fee = 100*CBP = 1e14 WAD (cbp6, ttm >= 360d).
    let offer = OfferBuilder::new(market(), MAKER).buy().tick(3372).ratifier(RATIFIER).max_units(u128::MAX).build();
    let cbps = [0, 0, 0, 0, 0, 0, 100];
    // maturity 2e9, now 0 -> ttm = 2e9 s > 360 days, so fee = cbp6.
    let units = U256::from(1_000_000u64);
    let a = take_amounts(&offer, units, 0, cbps).unwrap();
    // buy: buyer_price = offer_price (0.5e18), seller_price = 0.5e18 - 1e14.
    assert_eq!(a.buyer_price, U256::from(500_000_000_000_000_000u64));
    assert_eq!(a.seller_price, U256::from(499_900_000_000_000_000u64));
    assert_eq!(a.buyer_assets, U256::from(500_000u64));
    assert_eq!(a.seller_assets, U256::from(499_900u64));
    assert_eq!(a.settlement_fee_assets, U256::from(100u64));
}

#[test]
fn take_amounts_sell_rounds_up() {
    // sell offer: buyer_price = offer_price + fee, assets rounded up.
    let offer = OfferBuilder::new(market(), MAKER).sell().tick(3372).ratifier(RATIFIER).max_units(u128::MAX).build();
    let a = take_amounts(&offer, U256::from(3u64), 1_000_000_000, [0; 7]).unwrap();
    // price 0.5e18, units 3 -> 3*0.5 = 1.5 -> rounds up to 2.
    assert_eq!(a.buyer_assets, U256::from(2u64));
    assert_eq!(a.seller_assets, U256::from(2u64));
}

// ---- simulate_take ----

fn base_ctx() -> SimCtx {
    SimCtx {
        now: 1_000_000_000,
        market: SimMarket {
            tick_spacing: DEFAULT_TICK_SPACING,
            continuous_fee: 0,
            settlement_fee_cbp: [0; 7],
            loss_factor_maxed: false,
        },
        consumed: 0,
        maker_position: Position::default(),
        taker_position: Position::default(),
        taker_is_maker: false,
    }
}

fn buy_offer() -> Offer {
    OfferBuilder::new(market(), MAKER)
        .buy()
        .tick(8)
        .start(0)
        .expiry(2_000_000_000)
        .ratifier(RATIFIER)
        .max_units(10_000)
        .build()
}

#[test]
fn simulate_happy_path_deltas() {
    let offer = buy_offer();
    let out = simulate_take(&offer, U256::from(1_000u64), &base_ctx()).unwrap();
    assert!(out.reverts.is_empty(), "unexpected reverts: {:?}", out.reverts);
    // buy: maker is buyer (debt 0 -> credit increases by units); taker is seller (credit 0 -> all debt).
    assert_eq!(out.buyer_credit_increase, U256::from(1_000u64));
    assert_eq!(out.seller_credit_decrease, U256::ZERO);
    assert_eq!(out.seller_debt_increase, U256::from(1_000u64));
    assert_eq!(out.new_consumed, U256::from(1_000u64));
}

#[test]
fn simulate_seller_credit_offsets_debt() {
    let offer = buy_offer();
    let mut ctx = base_ctx();
    ctx.taker_position = Position { credit: 300, debt: 0, pending_fee: 0 }; // taker is seller
    let out = simulate_take(&offer, U256::from(1_000u64), &ctx).unwrap();
    // seller has 300 credit -> 300 decreases, remaining 700 becomes debt.
    assert_eq!(out.seller_credit_decrease, U256::from(300u64));
    assert_eq!(out.seller_debt_increase, U256::from(700u64));
}

#[test]
fn simulate_flags_self_take() {
    let mut ctx = base_ctx();
    ctx.taker_is_maker = true;
    let out = simulate_take(&buy_offer(), U256::from(1u64), &ctx).unwrap();
    assert!(out.reverts.contains(&OfferError::SelfTake));
}

#[test]
fn simulate_flags_consumed_units() {
    let offer = buy_offer(); // max_units = 10_000
    let mut ctx = base_ctx();
    ctx.consumed = 9_500;
    let out = simulate_take(&offer, U256::from(1_000u64), &ctx).unwrap();
    assert!(out.reverts.contains(&OfferError::ConsumedUnits));
    assert_eq!(out.new_consumed, U256::from(10_500u64));
}

#[test]
fn simulate_flags_reduce_only_increase() {
    let offer = OfferBuilder::new(market(), MAKER)
        .buy()
        .tick(8)
        .start(0)
        .expiry(2_000_000_000)
        .ratifier(RATIFIER)
        .reduce_only(true)
        .max_units(10_000)
        .build();
    // maker is buyer with no debt -> credit increases -> reduceOnly violated.
    let out = simulate_take(&offer, U256::from(1_000u64), &base_ctx()).unwrap();
    assert!(out.reverts.contains(&OfferError::MakerCreditOrDebtIncreased));
}

#[test]
fn simulate_flags_post_maturity_debt() {
    // now after maturity, seller (taker) has no credit -> debt would increase.
    let offer = OfferBuilder::new(market(), MAKER) // maturity 2e9
        .buy()
        .tick(8)
        .start(0)
        .expiry(3_000_000_000)
        .ratifier(RATIFIER)
        .max_units(10_000)
        .build();
    let mut ctx = base_ctx();
    ctx.now = 2_500_000_000; // > maturity
    let out = simulate_take(&offer, U256::from(1_000u64), &ctx).unwrap();
    assert!(out.reverts.contains(&OfferError::CannotIncreaseDebtPostMaturity));
}

#[test]
fn simulate_flags_tick_and_fee_and_loss() {
    let offer = OfferBuilder::new(market(), MAKER)
        .buy()
        .tick(5) // 5 % 4 != 0
        .start(0)
        .expiry(2_000_000_000)
        .ratifier(RATIFIER)
        .max_units(10_000)
        .continuous_fee_cap(U256::from(50u64))
        .build();
    let mut ctx = base_ctx();
    ctx.market.continuous_fee = 100; // > cap 50
    ctx.market.loss_factor_maxed = true;
    let out = simulate_take(&offer, U256::from(1u64), &ctx).unwrap();
    assert!(out.reverts.contains(&OfferError::TickNotAccessible));
    assert!(out.reverts.contains(&OfferError::ContinuousFeeAboveOfferCap));
    assert!(out.reverts.contains(&OfferError::MarketLossFactorMaxedOut));
}

#[test]
fn simulate_continuous_fee_accrues_to_buyer() {
    let offer = buy_offer();
    let mut ctx = base_ctx();
    ctx.market.continuous_fee = 1_000_000_000; // arbitrary rate
    ctx.now = 1_000_000_000; // maturity 2e9 -> ttm = 1e9
    let out = simulate_take(&offer, U256::from(1_000u64), &ctx).unwrap();
    // buyer_pending_fee_increase = mulDivDown(creditIncrease=1000, fee*ttm, WAD)
    // = 1000 * (1e9 * 1e9) / 1e18 = 1000 * 1e18 / 1e18 = 1000.
    assert_eq!(out.buyer_pending_fee_increase, U256::from(1_000u64));
}
