//! Tests for policy validation (`validate_offer` and consumption helpers).

use nocturne::*;

fn u256(x: u64) -> Word {
    let mut w = [0u8; 32];
    w[24..].copy_from_slice(&x.to_be_bytes());
    w
}

const MIDNIGHT: Address = [0x11; 20];

/// A well-formed offer that passes every check under `good_ctx()`.
fn good_offer() -> Offer {
    let market = Market {
        chain_id: u256(1),
        midnight: MIDNIGHT,
        loan_token: [0x22; 20],
        collateral_params: vec![CollateralParams {
            token: [0x33; 20],
            lltv: u256(860_000_000_000_000_000),
            liquidation_cursor: u256(1),
            oracle: [0x44; 20],
        }],
        maturity: u256(1_800_000_000),
        rcf_threshold: u256(1000),
        enter_gate: [0u8; 20],
        liquidator_gate: [0u8; 20],
    };
    Offer {
        market,
        buy: true,
        maker: [0x55; 20],
        start: u256(1_000),
        expiry: u256(2_000_000_000),
        tick: u256(8), // multiple of DEFAULT_TICK_SPACING (4), <= MAX_TICK
        group: u256(0),
        callback: [0u8; 20],
        callback_data: Vec::new(),
        receiver_if_maker_is_seller: [0u8; 20],
        ratifier: [0xbb; 20],
        reduce_only: false,
        max_units: 1_000_000,
        max_assets: 0,
        continuous_fee_cap: u256(100),
    }
}

fn good_ctx() -> ValidateCtx {
    ValidateCtx {
        chain_id: Some(1),
        midnight: Some(MIDNIGHT),
        now: Some(1_700_000_000),
        market: Some(MarketSnapshot {
            tick_spacing: DEFAULT_TICK_SPACING,
            loss_factor_maxed: false,
            continuous_fee: 100,
        }),
    }
}

#[test]
fn good_offer_passes_all_checks() {
    assert_eq!(validate_offer(&good_offer(), &good_ctx()), vec![]);
    assert!(is_valid(&good_offer(), &good_ctx()));
}

#[test]
fn stateless_subset_runs_with_default_ctx() {
    // With no context, only stateless checks run; the good offer is stateless-clean.
    assert!(is_valid(&good_offer(), &ValidateCtx::default()));
}

#[test]
fn invalid_caps_both_zero_and_both_set() {
    let mut o = good_offer();
    o.max_units = 0;
    o.max_assets = 0;
    assert!(validate_offer(&o, &ValidateCtx::default()).contains(&OfferError::InvalidOfferCaps));

    let mut o = good_offer();
    o.max_units = 1;
    o.max_assets = 1;
    assert!(validate_offer(&o, &ValidateCtx::default()).contains(&OfferError::InvalidOfferCaps));
}

#[test]
fn tick_out_of_range() {
    let mut o = good_offer();
    o.tick = u256(MAX_TICK + 1);
    let errs = validate_offer(&o, &ValidateCtx::default());
    assert!(errs.contains(&OfferError::TickOutOfRange));
}

#[test]
fn tick_at_max_is_in_range() {
    let mut o = good_offer();
    o.tick = u256(MAX_TICK); // MAX_TICK = 6744, a multiple of 4
    assert!(!validate_offer(&o, &ValidateCtx::default()).contains(&OfferError::TickOutOfRange));
}

#[test]
fn start_after_expiry() {
    let mut o = good_offer();
    o.start = u256(10);
    o.expiry = u256(9);
    assert!(validate_offer(&o, &ValidateCtx::default()).contains(&OfferError::StartAfterExpiry));
}

#[test]
fn buy_offer_must_have_zero_maker_receiver() {
    let mut o = good_offer();
    o.buy = true;
    o.receiver_if_maker_is_seller = [0x99; 20];
    assert!(
        validate_offer(&o, &ValidateCtx::default()).contains(&OfferError::UnusedReceiverMustBeZero)
    );

    // A sell offer may set it.
    let mut o = good_offer();
    o.buy = false;
    o.receiver_if_maker_is_seller = [0x99; 20];
    assert!(!validate_offer(&o, &ValidateCtx::default())
        .contains(&OfferError::UnusedReceiverMustBeZero));
}

#[test]
fn sell_offer_must_have_nonzero_maker_receiver() {
    // `take` accepts a zero receiver on a sell offer (it only checks the unused side), but the
    // maker's proceeds would go to address(0), so it's a stateless reject.
    let mut o = good_offer();
    o.buy = false;
    assert!(validate_offer(&o, &ValidateCtx::default()).contains(&OfferError::SellerReceiverZero));

    // A nonzero receiver makes the sell offer fully valid.
    let mut o = good_offer();
    o.buy = false;
    o.receiver_if_maker_is_seller = [0x99; 20];
    assert_eq!(validate_offer(&o, &good_ctx()), vec![]);

    // The rule is sell-side only: a buy offer keeps the zero receiver (and must, per
    // `buy_offer_must_have_zero_maker_receiver`).
    let o = good_offer();
    assert!(!validate_offer(&o, &ValidateCtx::default()).contains(&OfferError::SellerReceiverZero));
}

#[test]
fn zero_ratifier_is_rejected() {
    // A zero ratifier can never be authorized on-chain, so it's a stateless reject.
    let mut o = good_offer();
    o.ratifier = [0u8; 20];
    assert!(validate_offer(&o, &ValidateCtx::default()).contains(&OfferError::RatifierUnauthorized));

    // A nonzero ratifier clears the stateless check (on-chain authorization still applies).
    let mut o = good_offer();
    o.ratifier = [0xcc; 20];
    assert!(
        !validate_offer(&o, &ValidateCtx::default()).contains(&OfferError::RatifierUnauthorized)
    );
}

#[test]
fn collateral_structure() {
    // Empty.
    let mut o = good_offer();
    o.market.collateral_params.clear();
    assert!(validate_offer(&o, &ValidateCtx::default()).contains(&OfferError::NoCollateralParams));

    // Not strictly ascending (duplicate token).
    let mut o = good_offer();
    let cp = o.market.collateral_params[0].clone();
    o.market.collateral_params.push(cp);
    assert!(validate_offer(&o, &ValidateCtx::default())
        .contains(&OfferError::CollateralParamsNotSorted));

    // Properly sorted two-collateral market is fine.
    let mut o = good_offer();
    let mut cp2 = o.market.collateral_params[0].clone();
    cp2.token = [0x66; 20]; // > 0x33
    o.market.collateral_params.push(cp2);
    assert!(!validate_offer(&o, &ValidateCtx::default())
        .contains(&OfferError::CollateralParamsNotSorted));
}

#[test]
fn first_collateral_token_must_be_nonzero() {
    // `touchMarket` starts `previousCollateralToken` at address(0) and requires strict ascent
    // from there, so a zero first token fails even though every adjacent pair is sorted.
    let mut o = good_offer();
    o.market.collateral_params[0].token = [0u8; 20];
    let mut cp2 = o.market.collateral_params[0].clone();
    cp2.token = [0x33; 20]; // > 0x00, so the windows(2) pair alone is fine
    o.market.collateral_params.push(cp2);
    assert!(validate_offer(&o, &ValidateCtx::default())
        .contains(&OfferError::CollateralParamsNotSorted));

    // A normal nonzero-first sorted list passes (the good offer's single 0x33 collateral).
    assert!(!validate_offer(&good_offer(), &ValidateCtx::default())
        .contains(&OfferError::CollateralParamsNotSorted));
}

#[test]
fn max_lif_too_high() {
    // lltv = 0.5e18, cursor = 0.999e18: maxLif = 1e36 / (1e18 - 0.999e18*0.5e18/1e18)
    // = 1e36 / 0.5005e18 ~= 1.998e18 <= 2*WAD, but lltv * maxLif ~= 0.999001e36 > 0.999e36.
    let mut o = good_offer();
    o.market.collateral_params[0].lltv = u256(500_000_000_000_000_000);
    o.market.collateral_params[0].liquidation_cursor = u256(999_000_000_000_000_000);
    let errs = validate_offer(&o, &ValidateCtx::default());
    assert!(errs.contains(&OfferError::MaxLifTooHigh));
    assert!(!errs.contains(&OfferError::InvalidMaxLif));
}

#[test]
fn invalid_max_lif() {
    // lltv = 0.2e18, cursor = 0.7e18: maxLif = 1e36 / (1e18 - 0.7e18*0.8e18/1e18)
    // = 1e36 / 0.44e18 ~= 2.27e18 > 2*WAD. The product check still passes (0.4545e36 <= 0.999e36).
    let mut o = good_offer();
    o.market.collateral_params[0].lltv = u256(200_000_000_000_000_000);
    o.market.collateral_params[0].liquidation_cursor = u256(700_000_000_000_000_000);
    let errs = validate_offer(&o, &ValidateCtx::default());
    assert!(errs.contains(&OfferError::InvalidMaxLif));
    assert!(!errs.contains(&OfferError::MaxLifTooHigh));
}

#[test]
fn max_lif_reverting_computation_is_invalid() {
    // lltv = 0, cursor = WAD: the denominator WAD - cursor*(WAD-lltv)/WAD is exactly 0, so the
    // on-chain maxLif divides by zero.
    let mut o = good_offer();
    o.market.collateral_params[0].lltv = u256(0);
    o.market.collateral_params[0].liquidation_cursor = u256(1_000_000_000_000_000_000);
    assert!(validate_offer(&o, &ValidateCtx::default()).contains(&OfferError::InvalidMaxLif));

    // lltv > WAD: WAD - lltv underflows on-chain.
    let mut o = good_offer();
    o.market.collateral_params[0].lltv = u256(1_000_000_000_000_000_001);
    assert!(validate_offer(&o, &ValidateCtx::default()).contains(&OfferError::InvalidMaxLif));
}

#[test]
fn typical_lltv_cursor_passes_max_lif_checks() {
    // lltv = 0.8e18, cursor = 0.5e18: maxLif = 1e36 / 0.9e18 ~= 1.111e18 <= 2*WAD and
    // lltv * maxLif ~= 0.889e36 <= 0.999e36 - fully valid.
    let mut o = good_offer();
    o.market.collateral_params[0].lltv = u256(800_000_000_000_000_000);
    o.market.collateral_params[0].liquidation_cursor = u256(500_000_000_000_000_000);
    assert_eq!(validate_offer(&o, &good_ctx()), vec![]);
}

#[test]
fn lltv_of_wad_skips_product_check() {
    // lltv == WAD: maxLif = WAD for any cursor (WAD - lltv = 0), and the lltv * maxLif clause is
    // short-circuited by `lltv == WAD`, so even an aggressive cursor is fine.
    let mut o = good_offer();
    o.market.collateral_params[0].lltv = u256(1_000_000_000_000_000_000);
    o.market.collateral_params[0].liquidation_cursor = u256(999_000_000_000_000_000);
    assert_eq!(validate_offer(&o, &good_ctx()), vec![]);
}

#[test]
fn identity_checks() {
    let o = good_offer();
    let mut ctx = good_ctx();
    ctx.chain_id = Some(999);
    assert!(validate_offer(&o, &ctx).contains(&OfferError::InvalidChainId));

    let mut ctx = good_ctx();
    ctx.midnight = Some([0xee; 20]);
    assert!(validate_offer(&o, &ctx).contains(&OfferError::InvalidMidnight));
}

#[test]
fn time_checks() {
    let o = good_offer(); // start=1_000, expiry=2_000_000_000

    let mut ctx = good_ctx();
    ctx.now = Some(500); // before start
    assert!(validate_offer(&o, &ctx).contains(&OfferError::OfferNotStarted));

    let mut ctx = good_ctx();
    ctx.now = Some(2_000_000_001); // after expiry
    assert!(validate_offer(&o, &ctx).contains(&OfferError::OfferExpired));
}

#[test]
fn maturity_too_far() {
    let mut o = good_offer();
    o.market.maturity = u256(4_000_000_000); // ~far future but check against now
    let mut ctx = good_ctx();
    ctx.now = Some(1_000); // maturity is way more than 100y past now? no - 4e9-1e3 < 100y (3.15e9)? 100y ~= 3.156e9
                           // 4_000_000_000 - 1_000 > 3_153_600_000 -> too far
    assert!(validate_offer(&o, &ctx).contains(&OfferError::MaturityTooFar));

    // Within horizon.
    let mut ctx = good_ctx();
    ctx.now = Some(1_000_000_000);
    assert!(!validate_offer(&o, &ctx).contains(&OfferError::MaturityTooFar));
}

#[test]
fn market_snapshot_checks() {
    let o = good_offer(); // tick = 8, continuous_fee_cap = 100

    // Tick not a multiple of spacing.
    let mut ctx = good_ctx();
    ctx.market = Some(MarketSnapshot {
        tick_spacing: 3,
        loss_factor_maxed: false,
        continuous_fee: 100,
    });
    assert!(validate_offer(&o, &ctx).contains(&OfferError::TickNotAccessible));

    // Loss factor maxed.
    let mut ctx = good_ctx();
    ctx.market = Some(MarketSnapshot {
        tick_spacing: 4,
        loss_factor_maxed: true,
        continuous_fee: 100,
    });
    assert!(validate_offer(&o, &ctx).contains(&OfferError::MarketLossFactorMaxedOut));

    // Continuous fee above the offer's cap.
    let mut ctx = good_ctx();
    ctx.market = Some(MarketSnapshot {
        tick_spacing: 4,
        loss_factor_maxed: false,
        continuous_fee: 101,
    });
    assert!(validate_offer(&o, &ctx).contains(&OfferError::ContinuousFeeAboveOfferCap));

    // Fee exactly at cap is allowed (contract uses <=).
    let mut ctx = good_ctx();
    ctx.market = Some(MarketSnapshot {
        tick_spacing: 4,
        loss_factor_maxed: false,
        continuous_fee: 100,
    });
    assert!(!validate_offer(&o, &ctx).contains(&OfferError::ContinuousFeeAboveOfferCap));
}

#[test]
fn reports_multiple_errors_at_once() {
    let mut o = good_offer();
    o.max_units = 0;
    o.max_assets = 0; // InvalidOfferCaps
    o.tick = u256(MAX_TICK + 100); // TickOutOfRange
    o.start = u256(10);
    o.expiry = u256(5); // StartAfterExpiry
    let errs = validate_offer(&o, &ValidateCtx::default());
    assert!(errs.contains(&OfferError::InvalidOfferCaps));
    assert!(errs.contains(&OfferError::TickOutOfRange));
    assert!(errs.contains(&OfferError::StartAfterExpiry));
}

#[test]
fn consumption_headroom_and_can_consume() {
    // Units-capped offer.
    let mut o = good_offer();
    o.max_units = 1_000;
    o.max_assets = 0;
    assert_eq!(active_cap(&o), Some(Cap::Units(1_000)));
    assert_eq!(consumption_headroom(&o, 400), Some(600));
    assert!(can_consume(&o, 400, 600));
    assert!(!can_consume(&o, 400, 601));
    // Fully consumed never underflows.
    assert_eq!(consumption_headroom(&o, 2_000), Some(0));
    assert!(!can_consume(&o, 2_000, 1));

    // Assets-capped offer.
    let mut o = good_offer();
    o.max_units = 0;
    o.max_assets = 500;
    assert_eq!(active_cap(&o), Some(Cap::Assets(500)));
    assert_eq!(consumption_headroom(&o, 100), Some(400));

    // Invalid caps -> None / cannot consume.
    let mut o = good_offer();
    o.max_units = 1;
    o.max_assets = 1;
    assert_eq!(active_cap(&o), None);
    assert_eq!(consumption_headroom(&o, 0), None);
    assert!(!can_consume(&o, 0, 0));
}
