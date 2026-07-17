//! assets<->units sizing and consumable-units helpers.
//!
//! The inverse of the forward take math in [`crate::take_amounts`]: given a target *asset* amount
//! (notional), how many units must a taker lift to move exactly that many assets? Plus
//! [`consumable_units`], how many units of an offer remain takeable given prior consumption and the
//! offer's cap. This lets a maker/taker size in notional terms and compute remaining capacity
//! off-chain, with no chain reads.
//!
//! Mirrors `src/periphery/TakeAmountsLib.sol` (`buyerAssetsToUnits` / `sellerAssetsToUnits`) and
//! `src/periphery/ConsumableUnitsLib.sol` (`consumableUnits`) at contract rev f47568c9. The price /
//! settlement-fee / seller-price / buyer-price derivation and the buy/sell rounding directions are
//! kept consistent with the forward path in `sim.rs` so the two are exact inverses on clean values.
//!
//! ## Errors
//! These functions return [`SizingError`] rather than [`SimError`]. The contract's
//! `require(buyerPrice <= WAD, PriceGreaterThanOne())` has no counterpart in `SimError`, and
//! `SimError` lives in `sim.rs` which this module does not own (so a variant cannot be added there).
//! `SizingError` therefore wraps every [`SimError`] transparently and adds the sizing-specific
//! cases ([`SizingError::PriceGreaterThanOne`] and a defensive [`SizingError::ZeroPrice`] guard so
//! a degenerate zero price yields an error instead of a division-by-zero panic; on-chain that path
//! reverts too).

use crate::{settlement_fee, tick_to_price, word_to_u128, word_to_u256, Offer, SimError, U256};

const WAD: u128 = 1_000_000_000_000_000_000; // 1e18

/// Errors from the assets<->units sizing helpers.
///
/// Wraps [`SimError`] (tick / settlement-fee failures shared with the forward path) and adds the
/// two cases specific to the inverse math. See the module docs for why this is a distinct type.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum SizingError {
    /// A tick / settlement-fee failure shared with the forward take math.
    #[error(transparent)]
    Sim(#[from] SimError),
    /// `buyerPrice > WAD`: not every target asset amount is reachable at this price, so the
    /// contract's `require(buyerPrice <= WAD, PriceGreaterThanOne())` reverts.
    #[error("buyer price exceeds WAD; not all buyer assets are reachable")]
    PriceGreaterThanOne,
    /// The relevant price is zero, so no finite unit count moves a positive asset amount. On-chain
    /// the corresponding `mulDiv` by a zero denominator reverts; guarded here to avoid a panic.
    #[error("price is zero; units are unbounded")]
    ZeroPrice,
}

#[inline]
fn wad() -> U256 {
    U256::from(WAD)
}

/// `x * y / d`, rounded down (`FixedPointMathLib.mulDivDown`). Caller guarantees `d != 0`.
#[inline]
fn mul_div_down(x: U256, y: U256, d: U256) -> U256 {
    x * y / d
}

/// `(x * y + d - 1) / d`, rounded up (`FixedPointMathLib.mulDivUp`). Caller guarantees `d != 0`.
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

#[inline]
fn zero_floor_sub_u128(x: u128, y: u128) -> u128 {
    x.saturating_sub(y)
}

fn tick_u64(offer: &Offer) -> Result<u64, SimError> {
    let t = word_to_u128(&offer.tick).ok_or(SimError::TickNotU64)?;
    u64::try_from(t).map_err(|_| SimError::TickNotU64)
}

/// The offer's per-unit prices at time `now`, matching `sim.rs`'s derivation exactly.
///
/// Returns `(seller_price, buyer_price)`. For a buy offer `seller_price = offer_price - fee` (the
/// on-chain subtraction that reverts on underflow) and `buyer_price = offer_price`; for a sell
/// offer `seller_price = offer_price` and `buyer_price = offer_price + fee`.
fn prices(offer: &Offer, now: u64, cbps: [u16; 7]) -> Result<(U256, U256), SizingError> {
    let offer_price = tick_to_price(tick_u64(offer)?)?;
    let maturity = word_to_u256(&offer.market.maturity);
    let ttm = zero_floor_sub(maturity, U256::from(now));
    let fee = settlement_fee(cbps, ttm);

    let seller_price = if offer.buy {
        offer_price
            .checked_sub(fee)
            .ok_or(SimError::SettlementFeeExceedsPrice)?
    } else {
        offer_price
    };
    let buyer_price = seller_price + fee;
    Ok((seller_price, buyer_price))
}

/// Units a taker must lift so the **buyer** pays exactly `target_buyer_assets`.
///
/// Mirrors `TakeAmountsLib.buyerAssetsToUnits`. Inverse of the forward buyer-assets math: a buy
/// offer rounds up (`mulDivUp`), a sell offer rounds down (`mulDivDown`). Errors with
/// [`SizingError::PriceGreaterThanOne`] when `buyer_price > WAD`.
pub fn buyer_assets_to_units(
    offer: &Offer,
    target_buyer_assets: U256,
    now: u64,
    cbps: [u16; 7],
) -> Result<U256, SizingError> {
    let (_seller_price, buyer_price) = prices(offer, now, cbps)?;
    if buyer_price > wad() {
        return Err(SizingError::PriceGreaterThanOne);
    }
    if buyer_price.is_zero() {
        return Err(SizingError::ZeroPrice);
    }
    Ok(if offer.buy {
        mul_div_up(target_buyer_assets, wad(), buyer_price)
    } else {
        mul_div_down(target_buyer_assets, wad(), buyer_price)
    })
}

/// Units a taker must lift so the **seller** receives exactly `target_seller_assets`.
///
/// Mirrors `TakeAmountsLib.sellerAssetsToUnits`. Inverse of the forward seller-assets math: a buy
/// offer rounds up (`mulDivUp`), a sell offer rounds down (`mulDivDown`). Unlike the buyer helper,
/// this does not require `buyer_price <= WAD` (the contract doesn't either).
pub fn seller_assets_to_units(
    offer: &Offer,
    target_seller_assets: U256,
    now: u64,
    cbps: [u16; 7],
) -> Result<U256, SizingError> {
    let (seller_price, _buyer_price) = prices(offer, now, cbps)?;
    if seller_price.is_zero() {
        return Err(SizingError::ZeroPrice);
    }
    Ok(if offer.buy {
        mul_div_up(target_seller_assets, wad(), seller_price)
    } else {
        mul_div_down(target_seller_assets, wad(), seller_price)
    })
}

/// Units still takeable on `offer` given `consumed` so far — enough to fully consume it.
///
/// Mirrors `ConsumableUnitsLib.consumableUnits`. A units-capped offer (`max_units > 0`) returns
/// `zeroFloorSub(max_units, consumed)`; otherwise it is assets-capped and the remaining
/// `zeroFloorSub(max_assets, consumed)` assets are converted to units via the buyer helper (buy
/// offer) or seller helper (sell offer). `consumed` is `consumed[maker][group]` on-chain.
pub fn consumable_units(
    offer: &Offer,
    consumed: u128,
    now: u64,
    cbps: [u16; 7],
) -> Result<U256, SizingError> {
    if offer.max_units > 0 {
        return Ok(U256::from(zero_floor_sub_u128(offer.max_units, consumed)));
    }
    let remaining_assets = U256::from(zero_floor_sub_u128(offer.max_assets, consumed));
    if offer.buy {
        buyer_assets_to_units(offer, remaining_assets, now, cbps)
    } else {
        seller_assets_to_units(offer, remaining_assets, now, cbps)
    }
}
