//! Policy validation — "will the router accept this offer?"
//!
//! Mirrors the offer-relevant revert conditions in `Midnight.take` (and the market checks in
//! `touchMarket`) so a maker can reject a malformed offer *before* signing and posting a grid,
//! instead of discovering it when a taker's `take` reverts on the critical path.
//!
//! Checks split by what they need:
//! - **Stateless** (from the offer alone): caps, tick range, receiver rule, market structure,
//!   `start <= expiry`.
//! - **Context** (from a [`ValidateCtx`] the maker fills in as far as it can): chain id, midnight
//!   address, current time, and a live-market snapshot (tick spacing, loss factor, continuous fee).
//!
//! A check whose input is absent is simply skipped, so `validate_offer(offer, &ValidateCtx::default())`
//! runs the stateless subset. Consumption headroom (which depends on take amounts) is exposed
//! separately via [`consumption_headroom`] / [`can_consume`].

use crate::{word_from_u128 as w128, word_to_u128, Address, Offer};

/// Highest tick `TickLib.tickToPrice` accepts (`MAX_TICK` in TickLib.sol).
pub const MAX_TICK: u64 = 6744;
/// Max collateral params in a market (`MAX_COLLATERALS` in ConstantsLib.sol).
pub const MAX_COLLATERALS: usize = 128;
/// Default tick spacing a market is created with (`DEFAULT_TICK_SPACING` in ConstantsLib.sol).
pub const DEFAULT_TICK_SPACING: u8 = 4;
/// `touchMarket` rejects a maturity more than 100 years out.
pub const MAX_MATURITY_HORIZON_SECS: u64 = 100 * 365 * 24 * 60 * 60;

/// A reason an offer would be rejected. Names mirror `IMidnight` errors where one exists.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OfferError {
    /// Exactly one of `maxAssets` / `maxUnits` must be nonzero.
    InvalidOfferCaps,
    /// `tick` exceeds `MAX_TICK`, so `tickToPrice` would revert.
    TickOutOfRange,
    /// `start > expiry`: the offer is never simultaneously started and unexpired.
    StartAfterExpiry,
    /// `buy` offer must leave `receiverIfMakerIsSeller` zero (the maker is the buyer).
    UnusedReceiverMustBeZero,
    /// Market has no collateral params.
    NoCollateralParams,
    /// Market has more than `MAX_COLLATERALS` collateral params.
    TooManyCollateralParams,
    /// Collateral params are not strictly sorted ascending by token address.
    CollateralParamsNotSorted,
    /// `market.chainId` does not match the expected chain id.
    InvalidChainId,
    /// `market.midnight` does not match the expected Midnight address.
    InvalidMidnight,
    /// Maturity is more than 100 years past `now`.
    MaturityTooFar,
    /// `now < start`: not yet active.
    OfferNotStarted,
    /// `now > expiry`: already expired.
    OfferExpired,
    /// `tick` is not a multiple of the market's tick spacing.
    TickNotAccessible,
    /// The market's loss factor is maxed out (no new takes).
    MarketLossFactorMaxedOut,
    /// The market's continuous fee exceeds the offer's `continuousFeeCap`.
    ContinuousFeeAboveOfferCap,
    /// Group consumption would exceed `maxUnits`.
    ConsumedUnits,
    /// Group consumption would exceed `maxAssets`.
    ConsumedAssets,
    /// Taker equals maker.
    SelfTake,
    /// A take after maturity would increase the seller's debt.
    CannotIncreaseDebtPostMaturity,
    /// A `reduceOnly` offer would increase the maker's credit (buy) or debt (sell).
    MakerCreditOrDebtIncreased,
}

/// A snapshot of live market state, if the maker has one to validate against.
#[derive(Clone, Copy, Debug)]
pub struct MarketSnapshot {
    /// Current tick spacing. For a not-yet-created market pass [`DEFAULT_TICK_SPACING`].
    pub tick_spacing: u8,
    /// Whether the market's loss factor is maxed out.
    pub loss_factor_maxed: bool,
    /// The market's current continuous fee (same units as `continuousFeeCap`).
    pub continuous_fee: u128,
}

/// Optional environment for validation. Absent fields skip their checks.
#[derive(Clone, Copy, Debug, Default)]
pub struct ValidateCtx {
    /// Expected chain id (`block.chainid`).
    pub chain_id: Option<u64>,
    /// Expected Midnight instance address.
    pub midnight: Option<Address>,
    /// Current time (`block.timestamp`), for start/expiry/maturity checks.
    pub now: Option<u64>,
    /// Live market snapshot, for tick spacing / loss factor / fee checks.
    pub market: Option<MarketSnapshot>,
}

/// Which cap governs the offer, and its value.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Cap {
    /// Consumption is capped in units (`maxUnits`).
    Units(u128),
    /// Consumption is capped in assets — buyer assets if `buy`, else seller assets (`maxAssets`).
    Assets(u128),
}

/// The active cap for an offer, or `None` if the caps are invalid (`InvalidOfferCaps`).
pub fn active_cap(offer: &Offer) -> Option<Cap> {
    match (offer.max_assets == 0, offer.max_units == 0) {
        (false, true) => Some(Cap::Assets(offer.max_assets)),
        (true, false) => Some(Cap::Units(offer.max_units)),
        _ => None, // both zero or both nonzero
    }
}

/// Remaining consumable amount for the offer's group given `consumed_so_far`.
///
/// The unit is units or assets per [`active_cap`]. `None` if the caps are invalid. Returns 0 once
/// the group is fully consumed (never underflows).
pub fn consumption_headroom(offer: &Offer, consumed_so_far: u128) -> Option<u128> {
    let cap = match active_cap(offer)? {
        Cap::Units(v) | Cap::Assets(v) => v,
    };
    Some(cap.saturating_sub(consumed_so_far))
}

/// Whether taking `amount` more (in the offer's cap unit) stays within the group cap — the
/// off-chain mirror of the `ConsumedUnits` / `ConsumedAssets` checks.
pub fn can_consume(offer: &Offer, consumed_so_far: u128, amount: u128) -> bool {
    match consumption_headroom(offer, consumed_so_far) {
        Some(h) => amount <= h,
        None => false,
    }
}

/// Validate an offer, returning **every** problem found (empty vec = acceptable). Collecting all
/// errors lets a maker fix a whole grid in one pass rather than one revert at a time.
pub fn validate_offer(offer: &Offer, ctx: &ValidateCtx) -> Vec<OfferError> {
    let mut errs = Vec::new();

    // ---- stateless ----
    if active_cap(offer).is_none() {
        errs.push(OfferError::InvalidOfferCaps);
    }
    if offer.tick > w128(MAX_TICK as u128) {
        errs.push(OfferError::TickOutOfRange);
    }
    if offer.start > offer.expiry {
        errs.push(OfferError::StartAfterExpiry);
    }
    if offer.buy && offer.receiver_if_maker_is_seller != [0u8; 20] {
        errs.push(OfferError::UnusedReceiverMustBeZero);
    }

    let cps = &offer.market.collateral_params;
    if cps.is_empty() {
        errs.push(OfferError::NoCollateralParams);
    }
    if cps.len() > MAX_COLLATERALS {
        errs.push(OfferError::TooManyCollateralParams);
    }
    if cps.windows(2).any(|w| w[1].token <= w[0].token) {
        errs.push(OfferError::CollateralParamsNotSorted);
    }

    // ---- context: identity ----
    if let Some(chain_id) = ctx.chain_id {
        if offer.market.chain_id != w128(chain_id as u128) {
            errs.push(OfferError::InvalidChainId);
        }
    }
    if let Some(midnight) = ctx.midnight {
        if offer.market.midnight != midnight {
            errs.push(OfferError::InvalidMidnight);
        }
    }

    // ---- context: time ----
    if let Some(now) = ctx.now {
        let now_w = w128(now as u128);
        if now_w < offer.start {
            errs.push(OfferError::OfferNotStarted);
        }
        if now_w > offer.expiry {
            errs.push(OfferError::OfferExpired);
        }
        let horizon = w128(now as u128 + MAX_MATURITY_HORIZON_SECS as u128);
        if offer.market.maturity > horizon {
            errs.push(OfferError::MaturityTooFar);
        }
    }

    // ---- context: market snapshot ----
    if let Some(m) = ctx.market {
        if m.loss_factor_maxed {
            errs.push(OfferError::MarketLossFactorMaxedOut);
        }
        if w128(m.continuous_fee) > offer.continuous_fee_cap {
            errs.push(OfferError::ContinuousFeeAboveOfferCap);
        }
        // Only meaningful when the tick itself is in range and spacing is set.
        if m.tick_spacing > 0 {
            if let Some(tick) = word_to_u128(&offer.tick) {
                if tick % m.tick_spacing as u128 != 0 {
                    errs.push(OfferError::TickNotAccessible);
                }
            }
        }
    }

    errs
}

/// Convenience: `true` iff `validate_offer` finds no problems.
pub fn is_valid(offer: &Offer, ctx: &ValidateCtx) -> bool {
    validate_offer(offer, ctx).is_empty()
}
