//! Local take simulation - "if a taker lifts this offer for N units, what executes?"
//!
//! Ports the price / fee / amount / position math from `Midnight.take` (and `TickLib`,
//! `settlementFee`) so a maker can see the exact assets that would move and the position deltas
//! that would result, off-chain, without a node round-trip. This is the analog of a local EVM
//! simulator for the one call that matters on the hot path.
//!
//! Scope: the deterministic economic outcome - tick→price, settlement fee, buyer/seller assets,
//! consumption, and credit/debt/fee deltas - plus the take-time revert reasons that are locally
//! computable. Out of scope (needs live chain reads / external calls): gate checks, borrower
//! health, ratifier/authorization, and position slashing + fee accrual. Positions passed in are
//! assumed already up to date (i.e. post-`_updatePosition`).

use crate::{word_to_u128, word_to_u256, Offer, OfferError, MAX_TICK, U256};

const WAD: u128 = 1_000_000_000_000_000_000; // 1e18
const CBP: u128 = 1_000_000_000_000; // 1e12
const SEC_PER_DAY: u64 = 86_400;
// TickLib constants.
const LN_ONE_PLUS_DELTA: i128 = 4_987_541_511_039_073; // floor(ln(1.005) * 1e18)
const PRICE_ROUNDING_STEP: u128 = 100_000_000_000; // 1e11
const LN2: i128 = 693_147_180_559_945_309; // floor(ln(2) * 1e18)
const WEXP_OFFSET: i128 = 322_611_214_989_459_870; // 0.32261121498945987e18

/// Errors that prevent computing an outcome at all (as opposed to a would-revert reason).
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum SimError {
    /// The offer's tick exceeds `MAX_TICK`, so it has no price.
    #[error("tick {0} exceeds MAX_TICK ({MAX_TICK})")]
    TickOutOfRange(u128),
    /// The offer's tick word does not fit in a `u64`.
    #[error("tick does not fit in u64")]
    TickNotU64,
    /// The settlement fee exceeds the offer price (the on-chain subtraction would underflow).
    #[error("settlement fee exceeds offer price")]
    SettlementFeeExceedsPrice,
    /// A price passed to [`price_to_tick`] exceeds `1e18` (WAD), so it maps to no tick. Mirrors
    /// `TickLib.priceToTick`'s `require(price <= 1e18, PriceGreaterThanOne())`.
    #[error("price exceeds 1e18 (WAD)")]
    PriceGreaterThanOne,
    /// The time-to-maturity is zero, so a term rate cannot be annualized.
    #[error("time-to-maturity is zero")]
    ZeroTimeToMaturity,
    /// The tick's price is zero, so the term rate `1/P - 1` is undefined.
    #[error("price is zero; term rate is undefined")]
    ZeroPrice,
}

/// Seconds in a (non-leap) year: `365 * 24 * 3600`. Used to annualize term rates in APR math.
pub const SECONDS_PER_YEAR: u64 = 31_536_000;

#[inline]
fn wad() -> U256 {
    U256::from(WAD)
}

#[inline]
fn e36() -> U256 {
    // 1e36
    U256::from(WAD) * U256::from(WAD)
}

/// `x / d` rounded to nearest, ties down (`TickLib.divHalfDownUnchecked`).
#[inline]
fn div_half_down(x: U256, d: U256) -> U256 {
    (x + (d - U256::from(1u64)) / U256::from(2u64)) / d
}

#[inline]
fn mul_div_down(x: U256, y: U256, d: U256) -> U256 {
    x * y / d
}

#[inline]
fn mul_div_up(x: U256, y: U256, d: U256) -> U256 {
    (x * y + (d - U256::from(1u64))) / d
}

#[inline]
fn zero_floor_sub(x: U256, y: U256) -> U256 {
    if x > y {
        x - y
    } else {
        U256::ZERO
    }
}

/// `TickLib.wExp` - WAD-scaled exponential. The polynomial fits in `i128`; only the final shift
/// and reciprocal need 256 bits.
fn wexp(x: i128) -> U256 {
    if x < 0 {
        return e36() / wexp(-x);
    }
    let q = (x + WEXP_OFFSET) / LN2; // >= 0
    let r = x - q * LN2;
    let second = r * r / (2 * WAD as i128);
    let third = second * r / (3 * WAD as i128);
    let exp_r = WAD as i128 + r + second + third; // positive
    U256::from(exp_r as u128) << (q as usize)
}

/// `TickLib.tickToPrice` - the WAD price for a tick, rounded to `PRICE_ROUNDING_STEP`.
pub fn tick_to_price(tick: u64) -> Result<U256, SimError> {
    if tick > MAX_TICK {
        return Err(SimError::TickOutOfRange(tick as u128));
    }
    let arg = LN_ONE_PLUS_DELTA * ((MAX_TICK as i128 / 2) - tick as i128);
    let inner = wad() + wexp(arg);
    let step = U256::from(PRICE_ROUNDING_STEP);
    let p = div_half_down(e36(), inner);
    Ok(div_half_down(p, step) * step)
}

/// `TickLib.priceToTick` - among the ticks that are multiples of `spacing`, the lowest one whose
/// price is greater than or equal to `price`.
///
/// Binary-searches [`tick_to_price`] for the lowest tick with `tick_to_price(tick) >= price`
/// (prices are monotonically increasing in tick), then rounds that tick **up** to the next
/// multiple of `spacing`: `(low + spacing - 1) / spacing * spacing`. `spacing` should divide
/// `MAX_TICK` (the default is [`DEFAULT_TICK_SPACING`](crate::DEFAULT_TICK_SPACING)).
///
/// Errors with [`SimError::PriceGreaterThanOne`] when `price > 1e18` (WAD), exactly as the
/// contract's `require(price <= 1e18, PriceGreaterThanOne())`.
pub fn price_to_tick(price: U256, spacing: u64) -> Result<u64, SimError> {
    if price > wad() {
        return Err(SimError::PriceGreaterThanOne);
    }
    let mut low: u64 = 0;
    let mut high: u64 = MAX_TICK;
    while low != high {
        let mid = (low + high) / 2;
        if tick_to_price(mid)? < price {
            low = mid + 1;
        } else {
            high = mid;
        }
    }
    Ok((low + spacing - 1) / spacing * spacing)
}

/// The **simple annualized APR** of a tick, as a **percentage** (e.g. `7.2` means 7.2%).
///
/// The whitepaper defines the simple rate over the remaining term as `r = 1/P - 1`, where
/// `P = tick_to_price(tick)` is the discounted price in WAD (`0..1e18`). This annualizes `r`
/// **linearly** (simple, not compound) by the time-to-maturity:
///
/// ```text
/// price_frac = tick_to_price(tick) / 1e18
/// term_rate  = 1 / price_frac - 1
/// apr_pct    = term_rate * (SECONDS_PER_YEAR / ttm_secs) * 100
/// ```
///
/// Because price increases with tick, APR **decreases** with tick. Errors on `ttm_secs == 0`
/// ([`SimError::ZeroTimeToMaturity`]) or a zero price ([`SimError::ZeroPrice`]).
pub fn tick_to_apr(tick: u64, ttm_secs: u64) -> Result<f64, SimError> {
    if ttm_secs == 0 {
        return Err(SimError::ZeroTimeToMaturity);
    }
    let price = tick_to_price(tick)?;
    if price.is_zero() {
        return Err(SimError::ZeroPrice);
    }
    let price_frac = u128::try_from(price).expect("price <= 1e18 fits in u128") as f64 / 1e18;
    let term_rate = 1.0 / price_frac - 1.0;
    let apr_pct = term_rate * (SECONDS_PER_YEAR as f64 / ttm_secs as f64) * 100.0;
    Ok(apr_pct)
}

/// Inverse of [`tick_to_apr`]: the lowest **accessible** tick that yields at most `apr_pct`.
///
/// Converts the simple annualized APR (percent) back to a WAD price and defers to
/// [`price_to_tick`]:
///
/// ```text
/// term_rate = (apr_pct / 100) * (ttm_secs / SECONDS_PER_YEAR)
/// price_frac = 1 / (1 + term_rate)   // clamped to <= 1.0
/// price_wad  = price_frac * 1e18
/// tick       = price_to_tick(price_wad, spacing)
/// ```
///
/// # Round-trip is not exact
/// [`price_to_tick`] snaps **up** to the nearest accessible tick (price `>=` the target, aligned
/// to `spacing`), so `apr_to_tick(tick_to_apr(t, ttm), ttm, spacing)` need not equal `t`. It is,
/// however, guaranteed to land within one `spacing` step of `t`, and the resulting tick's APR is
/// `<= apr_pct` (the snap only ever raises the price / lowers the rate).
///
/// A negative `apr_pct` (implied price above par) clamps `price_frac` to `1.0`, mapping to the
/// tick nearest par. Errors propagate from [`price_to_tick`].
pub fn apr_to_tick(apr_pct: f64, ttm_secs: u64, spacing: u64) -> Result<u64, SimError> {
    let term_rate = (apr_pct / 100.0) * (ttm_secs as f64 / SECONDS_PER_YEAR as f64);
    let mut price_frac = 1.0 / (1.0 + term_rate);
    if price_frac > 1.0 {
        price_frac = 1.0;
    }
    let price_wad = U256::from((price_frac * 1e18) as u128);
    price_to_tick(price_wad, spacing)
}

/// `Midnight.settlementFee` - piecewise-linear interpolation over the 7 breakpoints (0/1/7/30/
/// 90/180/360 days), returned in WAD. `cbps` are the market's `settlementFeeCbp0..6`.
pub fn settlement_fee(cbps: [u16; 7], time_to_maturity: U256) -> U256 {
    let day = |d: u64| U256::from(d * SEC_PER_DAY);
    let cbp = |i: usize| U256::from(cbps[i] as u128) * U256::from(CBP);

    if time_to_maturity >= day(360) {
        return cbp(6);
    }
    let ttm = time_to_maturity; // < 360 days, fits everything below
    let (start, end, fee_lower, fee_upper) = if ttm < day(1) {
        (day(0), day(1), cbp(0), cbp(1))
    } else if ttm < day(7) {
        (day(1), day(7), cbp(1), cbp(2))
    } else if ttm < day(30) {
        (day(7), day(30), cbp(2), cbp(3))
    } else if ttm < day(90) {
        (day(30), day(90), cbp(3), cbp(4))
    } else if ttm < day(180) {
        (day(90), day(180), cbp(4), cbp(5))
    } else {
        (day(180), day(360), cbp(5), cbp(6))
    };
    (fee_lower * (end - ttm) + fee_upper * (ttm - start)) / (end - start)
}

/// The prices and asset amounts a take moves. All in WAD-scaled token units.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TakeAmounts {
    /// `tickToPrice(offer.tick)`.
    pub offer_price: U256,
    /// Price the buyer pays per unit.
    pub buyer_price: U256,
    /// Price the seller receives per unit.
    pub seller_price: U256,
    /// Assets the buyer pays.
    pub buyer_assets: U256,
    /// Assets the seller receives.
    pub seller_assets: U256,
    /// Protocol settlement fee taken (`buyer_assets - seller_assets`).
    pub settlement_fee_assets: U256,
}

fn tick_u64(offer: &Offer) -> Result<u64, SimError> {
    let t = word_to_u128(&offer.tick).ok_or(SimError::TickNotU64)?;
    u64::try_from(t).map_err(|_| SimError::TickNotU64)
}

/// Compute prices and asset amounts for taking `units` of `offer` at time `now`, using the
/// market's settlement-fee breakpoints. No position state required.
pub fn take_amounts(
    offer: &Offer,
    units: U256,
    now: u64,
    cbps: [u16; 7],
) -> Result<TakeAmounts, SimError> {
    let offer_price = tick_to_price(tick_u64(offer)?)?;
    let maturity = word_to_u256(&offer.market.maturity);
    let ttm = zero_floor_sub(maturity, U256::from(now));
    let fee = settlement_fee(cbps, ttm);

    let (seller_price, buyer_price) = if offer.buy {
        let sp = offer_price
            .checked_sub(fee)
            .ok_or(SimError::SettlementFeeExceedsPrice)?;
        (sp, offer_price) // buyer_price = seller_price + fee = offer_price
    } else {
        (offer_price, offer_price + fee)
    };

    let (buyer_assets, seller_assets) = if offer.buy {
        (
            mul_div_down(units, buyer_price, wad()),
            mul_div_down(units, seller_price, wad()),
        )
    } else {
        (
            mul_div_up(units, buyer_price, wad()),
            mul_div_up(units, seller_price, wad()),
        )
    };

    Ok(TakeAmounts {
        offer_price,
        buyer_price,
        seller_price,
        buyer_assets,
        seller_assets,
        settlement_fee_assets: buyer_assets - seller_assets,
    })
}

/// A user's effective position, assumed already updated (post-slash / post-accrual).
#[derive(Clone, Copy, Debug, Default)]
pub struct Position {
    /// Outstanding lender credit.
    pub credit: u128,
    /// Outstanding borrower debt.
    pub debt: u128,
    /// Accrued continuous fee owed.
    pub pending_fee: u128,
}

/// Market state needed to simulate a take.
#[derive(Clone, Copy, Debug)]
pub struct SimMarket {
    /// Current tick spacing (0 means the market isn't created).
    pub tick_spacing: u8,
    /// Market continuous fee (same scaling as the on-chain `uint32`).
    pub continuous_fee: u128,
    /// Settlement-fee breakpoints `settlementFeeCbp0..6`.
    pub settlement_fee_cbp: [u16; 7],
    /// Whether the loss factor is maxed out.
    pub loss_factor_maxed: bool,
}

/// Everything the simulator needs beyond the offer and take size.
#[derive(Clone, Copy, Debug)]
pub struct SimCtx {
    /// Current time (`block.timestamp`).
    pub now: u64,
    /// The market snapshot.
    pub market: SimMarket,
    /// `consumed[maker][group]` so far.
    pub consumed: u128,
    /// The maker's current position.
    pub maker_position: Position,
    /// The taker's current position.
    pub taker_position: Position,
    /// Whether taker == maker (drives `SelfTake`).
    pub taker_is_maker: bool,
}

/// The full result of simulating a take.
#[derive(Clone, Debug)]
pub struct TakeOutcome {
    /// Prices and asset amounts moved.
    pub amounts: TakeAmounts,
    /// Increase in the buyer's credit.
    pub buyer_credit_increase: U256,
    /// Decrease in the seller's credit.
    pub seller_credit_decrease: U256,
    /// Increase in the seller's debt.
    pub seller_debt_increase: U256,
    /// Increase in the buyer's pending continuous fee.
    pub buyer_pending_fee_increase: U256,
    /// Decrease in the seller's pending continuous fee.
    pub seller_pending_fee_decrease: U256,
    /// New `consumed[maker][group]` after the take.
    pub new_consumed: U256,
    /// Would-revert reasons; empty means the take succeeds (subject to the out-of-scope checks).
    pub reverts: Vec<OfferError>,
}

/// Simulate a take of `units` of `offer`. Returns the economic outcome and any locally
/// computable revert reasons. See the module docs for what's out of scope.
pub fn simulate_take(offer: &Offer, units: U256, ctx: &SimCtx) -> Result<TakeOutcome, SimError> {
    let amounts = take_amounts(offer, units, ctx.now, ctx.market.settlement_fee_cbp)?;
    let mut reverts = Vec::new();

    // Caps: exactly one of maxAssets / maxUnits.
    let assets_capped = offer.max_assets != 0;
    let units_capped = offer.max_units != 0;
    if assets_capped == units_capped {
        reverts.push(OfferError::InvalidOfferCaps);
    }
    if ctx.market.loss_factor_maxed {
        reverts.push(OfferError::MarketLossFactorMaxedOut);
    }
    if U256::from(ctx.market.continuous_fee) > word_to_u256(&offer.continuous_fee_cap) {
        reverts.push(OfferError::ContinuousFeeAboveOfferCap);
    }
    // The unused receiver side must be zero (Midnight.take). Only the offer-side half (a buy
    // offer's receiverIfMakerIsSeller) is computable here; the sell-side half constrains the
    // taker's own receiver argument, which the simulator does not take.
    if offer.buy && offer.receiver_if_maker_is_seller != [0u8; 20] {
        reverts.push(OfferError::UnusedReceiverMustBeZero);
    }
    if let Ok(tick) = tick_u64(offer) {
        if ctx.market.tick_spacing > 0 && tick % ctx.market.tick_spacing as u64 != 0 {
            reverts.push(OfferError::TickNotAccessible);
        }
    }
    let now = U256::from(ctx.now);
    if now < word_to_u256(&offer.start) {
        reverts.push(OfferError::OfferNotStarted);
    }
    if now > word_to_u256(&offer.expiry) {
        reverts.push(OfferError::OfferExpired);
    }
    if ctx.taker_is_maker {
        reverts.push(OfferError::SelfTake);
    }

    // Consumption.
    let consumed = U256::from(ctx.consumed);
    let new_consumed = if assets_capped {
        let add = if offer.buy {
            amounts.buyer_assets
        } else {
            amounts.seller_assets
        };
        let nc = consumed + add;
        if nc > U256::from(offer.max_assets) {
            reverts.push(OfferError::ConsumedAssets);
        }
        nc
    } else if units_capped {
        let nc = consumed + units;
        if nc > U256::from(offer.max_units) {
            reverts.push(OfferError::ConsumedUnits);
        }
        nc
    } else {
        consumed // invalid caps already flagged
    };

    // buyer = buy ? maker : taker ; seller = buy ? taker : maker.
    let (buyer_pos, seller_pos) = if offer.buy {
        (ctx.maker_position, ctx.taker_position)
    } else {
        (ctx.taker_position, ctx.maker_position)
    };

    let buyer_credit_increase = zero_floor_sub(units, U256::from(buyer_pos.debt));
    let seller_credit_decrease = core::cmp::min(units, U256::from(seller_pos.credit));
    let seller_debt_increase = units - seller_credit_decrease;

    let ttm = zero_floor_sub(word_to_u256(&offer.market.maturity), now);
    let buyer_pending_fee_increase = mul_div_down(
        buyer_credit_increase,
        U256::from(ctx.market.continuous_fee) * ttm,
        wad(),
    );
    let seller_pending_fee_decrease = if seller_pos.credit > 0 {
        mul_div_up(
            U256::from(seller_pos.pending_fee),
            seller_credit_decrease,
            U256::from(seller_pos.credit),
        )
    } else {
        U256::ZERO
    };

    // Post-maturity debt increase.
    if now > word_to_u256(&offer.market.maturity) && seller_debt_increase != U256::ZERO {
        reverts.push(OfferError::CannotIncreaseDebtPostMaturity);
    }
    // reduceOnly: maker's own credit (buy) / debt (sell) must not increase.
    if offer.reduce_only {
        let maker_increased = if offer.buy {
            buyer_credit_increase != U256::ZERO
        } else {
            seller_debt_increase != U256::ZERO
        };
        if maker_increased {
            reverts.push(OfferError::MakerCreditOrDebtIncreased);
        }
    }

    Ok(TakeOutcome {
        amounts,
        buyer_credit_increase,
        seller_credit_decrease,
        seller_debt_increase,
        buyer_pending_fee_increase,
        seller_pending_fee_decrease,
        new_consumed,
        reverts,
    })
}
