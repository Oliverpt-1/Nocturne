//! Ergonomic builders for `Market` and `Offer`.
//!
//! The wire structs store every `uint256` as a raw big-endian `Word`, which is easy to get
//! wrong by hand. These builders take typed inputs (`u64` / `u128` / [`U256`] / `Address`),
//! apply sensible defaults, and pack the `Word`s for you. `try_build` additionally runs
//! [`validate_offer`](crate::validate_offer) so a malformed offer never leaves the builder.

use crate::{
    apr_to_tick, buyer_assets_to_units, seller_assets_to_units, u256_to_word, validate_offer,
    word_from_u64, word_to_u256, Address, CollateralParams, Market, Offer, OfferError, SimError,
    SizingError, ValidateCtx, Word, DEFAULT_TICK_SPACING, MAX_CONTINUOUS_FEE, U256,
};

/// A builder-usage error surfaced at [`build_checked`](OfferBuilder::build_checked) time: either a
/// conversion failure captured by the ergonomic [`OfferBuilder`] setters ([`apr`](OfferBuilder::apr) /
/// [`assets`](OfferBuilder::assets)) - so those setters never panic - or a mandatory field (side /
/// tick) that was never set.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum BuildError {
    /// An APR→tick conversion failed (see [`apr_to_tick`](crate::apr_to_tick)).
    #[error(transparent)]
    Sim(#[from] SimError),
    /// An assets→units conversion failed (see [`buyer_assets_to_units`](crate::buyer_assets_to_units)).
    #[error(transparent)]
    Sizing(#[from] SizingError),
    /// The computed unit count does not fit in the `u128` `maxUnits` field.
    #[error("computed units exceed u128")]
    UnitsOverflow,
    /// The offer side was never set (via [`side`](OfferBuilder::side) or one of its shorthands).
    #[error("offer side never set")]
    SideNotSet,
    /// The tick was never set (via [`tick`](OfferBuilder::tick) or [`apr`](OfferBuilder::apr)).
    #[error("offer tick never set")]
    TickNotSet,
    /// The required offer expiry was never set.
    #[error("offer expiry never set")]
    ExpiryNotSet,
}

/// Builder for a [`Market`]. Collateral params must be added in ascending token order (the only
/// ordering the protocol accepts); `try_build` on the offer will flag it otherwise.
#[derive(Clone, Debug)]
#[must_use = "call `.build()` to produce the Market"]
pub struct MarketBuilder {
    chain_id: U256,
    midnight: Address,
    loan_token: Address,
    collateral_params: Vec<CollateralParams>,
    maturity: U256,
    rcf_threshold: U256,
    enter_gate: Address,
    liquidator_gate: Address,
}

impl MarketBuilder {
    /// Start a market on `chain_id`, deployed at `midnight`, denominated in `loan_token`.
    pub fn new(chain_id: u64, midnight: Address, loan_token: Address) -> Self {
        Self {
            chain_id: U256::from(chain_id),
            midnight,
            loan_token,
            collateral_params: Vec::new(),
            maturity: U256::ZERO,
            rcf_threshold: U256::ZERO,
            enter_gate: [0u8; 20],
            liquidator_gate: [0u8; 20],
        }
    }

    /// Append a collateral option. Add these in ascending `token` order.
    pub fn collateral(
        mut self,
        token: Address,
        lltv: U256,
        liquidation_cursor: U256,
        oracle: Address,
    ) -> Self {
        self.collateral_params.push(CollateralParams {
            token,
            lltv: u256_to_word(lltv),
            liquidation_cursor: u256_to_word(liquidation_cursor),
            oracle,
        });
        self
    }

    /// Set the market maturity (unix seconds).
    pub fn maturity(mut self, unix_secs: u64) -> Self {
        self.maturity = U256::from(unix_secs);
        self
    }

    /// Set the recovery-close-factor threshold.
    pub fn rcf_threshold(mut self, v: U256) -> Self {
        self.rcf_threshold = v;
        self
    }

    /// Set the enter gate (default: none / zero address).
    pub fn enter_gate(mut self, gate: Address) -> Self {
        self.enter_gate = gate;
        self
    }

    /// Set the liquidator gate (default: none / zero address).
    pub fn liquidator_gate(mut self, gate: Address) -> Self {
        self.liquidator_gate = gate;
        self
    }

    /// Produce the `Market`.
    pub fn build(self) -> Market {
        Market {
            chain_id: u256_to_word(self.chain_id),
            midnight: self.midnight,
            loan_token: self.loan_token,
            collateral_params: self.collateral_params,
            maturity: u256_to_word(self.maturity),
            rcf_threshold: u256_to_word(self.rcf_threshold),
            enter_gate: self.enter_gate,
            liquidator_gate: self.liquidator_gate,
        }
    }
}

/// Builder for an [`Offer`].
///
/// Defaults: `start = 0`, no callback, `reduce_only = false`, and
/// `continuous_fee_cap = MAX_CONTINUOUS_FEE`. You must set a side, tick, expiry, ratifier, and
/// exactly one of [`max_units`](Self::max_units) / [`max_assets`](Self::max_assets).
/// [`try_build`](Self::try_build) and [`build_checked`](Self::build_checked) reject an offer whose
/// side or tick was never set: relying on the defaults would sign a buy offer (in `take` the maker
/// is then the buyer whose approved loan tokens are pulled) at tick 0, whose price rounds to zero
/// (the `PRICE_ROUNDING_STEP` snap in TickLib.sol), giving the maker's units away for nothing.
/// Only the raw [`build`](Self::build) escape hatch skips this enforcement. A sell offer
/// must also set a nonzero [`receiver_if_maker_is_seller`](Self::receiver_if_maker_is_seller):
/// the zero default is only valid for buy offers - on a sell offer `take` would send the maker's
/// loan-token proceeds to `address(0)`.
#[derive(Clone, Debug)]
#[must_use = "call `.build()` or `.try_build(..)` to produce the Offer"]
pub struct OfferBuilder {
    market: Market,
    buy: bool,
    side_set: bool,
    maker: Address,
    start: U256,
    expiry: U256,
    expiry_set: bool,
    tick: U256,
    tick_set: bool,
    group: Word,
    callback: Address,
    callback_data: Vec<u8>,
    receiver_if_maker_is_seller: Address,
    receiver_set: bool,
    ratifier: Address,
    reduce_only: bool,
    max_units: u128,
    max_assets: u128,
    continuous_fee_cap: U256,
    error: Option<BuildError>,
}

impl OfferBuilder {
    /// Start an offer in `market` made by `maker`.
    pub fn new(market: Market, maker: Address) -> Self {
        Self {
            market,
            buy: true,
            side_set: false,
            maker,
            start: U256::ZERO,
            expiry: U256::ZERO,
            expiry_set: false,
            tick: U256::ZERO,
            tick_set: false,
            group: [0u8; 32],
            callback: [0u8; 20],
            callback_data: Vec::new(),
            receiver_if_maker_is_seller: [0u8; 20],
            receiver_set: false,
            ratifier: [0u8; 20],
            reduce_only: false,
            max_units: 0,
            max_assets: 0,
            continuous_fee_cap: U256::from(MAX_CONTINUOUS_FEE),
            error: None,
        }
    }

    /// Record the first conversion error so [`build_checked`](Self::build_checked) can surface it.
    fn record_err(&mut self, e: BuildError) {
        if self.error.is_none() {
            self.error = Some(e);
        }
    }

    /// `buy = true` (maker is the buyer) or `false` (maker is the seller).
    pub fn side(mut self, buy: bool) -> Self {
        self.buy = buy;
        self.side_set = true;
        if !self.receiver_set {
            self.receiver_if_maker_is_seller = if buy { [0u8; 20] } else { self.maker };
        }
        self
    }

    /// Shorthand for `side(true)`.
    pub fn buy(self) -> Self {
        self.side(true)
    }

    /// Shorthand for `side(false)`.
    pub fn sell(self) -> Self {
        self.side(false)
    }

    /// Shorthand for `side(true)` - a lender buys the loan (maker is the buyer).
    pub fn lend(self) -> Self {
        self.side(true)
    }

    /// Shorthand for `side(false)` - a borrower sells the loan (maker is the seller).
    pub fn borrow(self) -> Self {
        self.side(false)
    }

    /// Set the price tick.
    pub fn tick(mut self, tick: u64) -> Self {
        self.tick = U256::from(tick);
        self.tick_set = true;
        self
    }

    /// Set the start time (unix seconds).
    pub fn start(mut self, unix_secs: u64) -> Self {
        self.start = U256::from(unix_secs);
        self
    }

    /// Set the expiry time (unix seconds).
    pub fn expiry(mut self, unix_secs: u64) -> Self {
        self.expiry = U256::from(unix_secs);
        self.expiry_set = true;
        self
    }

    /// Set the group (raw 32-byte one-cancels-others key).
    pub fn group(mut self, group: Word) -> Self {
        self.group = group;
        self
    }

    /// Set the group from a small integer.
    pub fn group_u64(mut self, group: u64) -> Self {
        self.group = word_from_u64(group);
        self
    }

    /// Set the ratifier that will authorize this offer's signature.
    pub fn ratifier(mut self, ratifier: Address) -> Self {
        self.ratifier = ratifier;
        self
    }

    /// Set the offer callback and its data.
    pub fn callback(mut self, callback: Address, data: Vec<u8>) -> Self {
        self.callback = callback;
        self.callback_data = data;
        self
    }

    /// Set the receiver of the maker's proceeds when the maker is the seller (must stay zero for
    /// buy offers, must be nonzero for sell offers - `address(0)` would receive the proceeds).
    pub fn receiver_if_maker_is_seller(mut self, receiver: Address) -> Self {
        self.receiver_if_maker_is_seller = receiver;
        self.receiver_set = true;
        self
    }

    /// Mark the offer reduce-only (maker's credit/debt may not increase).
    pub fn reduce_only(mut self, reduce_only: bool) -> Self {
        self.reduce_only = reduce_only;
        self
    }

    /// Cap consumption in units. Clears any assets cap (the protocol allows exactly one).
    pub fn max_units(mut self, max_units: u128) -> Self {
        self.max_units = max_units;
        self.max_assets = 0;
        self
    }

    /// Cap consumption in assets. Clears any units cap (the protocol allows exactly one).
    pub fn max_assets(mut self, max_assets: u128) -> Self {
        self.max_assets = max_assets;
        self.max_units = 0;
        self
    }

    /// Set the maximum market continuous fee the maker will tolerate.
    pub fn continuous_fee_cap(mut self, cap: U256) -> Self {
        self.continuous_fee_cap = cap;
        self
    }

    /// Set the tick from a target **simple annualized APR** (percent, e.g. `7.2` = 7.2%).
    ///
    /// Computes the time-to-maturity as `market.maturity - now` and converts via
    /// [`apr_to_tick`](crate::apr_to_tick) at [`DEFAULT_TICK_SPACING`], snapping to the lowest
    /// accessible tick whose price is `>=` the target (so the realized APR is `<=` `apr_pct`).
    ///
    /// Never panics: any conversion error is stored and surfaced by
    /// [`build_checked`](Self::build_checked). Because of this, offers built with `.apr()` must be
    /// finalized with [`build_checked`](Self::build_checked) (or validated via `try_build`), not the
    /// infallible [`build`](Self::build).
    pub fn apr(mut self, apr_pct: f64, now: u64) -> Self {
        let maturity = word_to_u256(&self.market.maturity);
        let ttm = maturity.saturating_sub(U256::from(now));
        let ttm_secs = u64::try_from(ttm).unwrap_or(u64::MAX);
        match apr_to_tick(apr_pct, ttm_secs, DEFAULT_TICK_SPACING as u64) {
            Ok(tick) => {
                self.tick = U256::from(tick);
                self.tick_set = true;
            }
            Err(e) => self.record_err(e.into()),
        }
        self
    }

    /// Set [`max_units`](Self::max_units) from a target **asset** (notional) amount.
    ///
    /// Sizes the maker's side: for a buy offer the maker is the buyer
    /// ([`buyer_assets_to_units`](crate::buyer_assets_to_units)); for a sell offer the maker is the
    /// seller ([`seller_assets_to_units`](crate::seller_assets_to_units)). `cbps` are the market's
    /// `settlementFeeCbp0..6`. Set the tick (via [`tick`](Self::tick) or [`apr`](Self::apr)) first,
    /// since sizing depends on the price.
    ///
    /// Never panics: any conversion error is stored and surfaced by
    /// [`build_checked`](Self::build_checked), so offers built with `.assets()` must be finalized
    /// with [`build_checked`](Self::build_checked) (or `try_build`), not [`build`](Self::build).
    pub fn assets(mut self, target_assets: u128, now: u64, cbps: [u16; 7]) -> Self {
        let offer = self.clone().build();
        let target = U256::from(target_assets);
        let res = if self.buy {
            buyer_assets_to_units(&offer, target, now, cbps)
        } else {
            seller_assets_to_units(&offer, target, now, cbps)
        };
        match res {
            Ok(units) => match u128::try_from(units) {
                Ok(u) => {
                    self.max_units = u;
                    self.max_assets = 0;
                }
                Err(_) => self.record_err(BuildError::UnitsOverflow),
            },
            Err(e) => self.record_err(e.into()),
        }
        self
    }

    /// Produce the `Offer`, or the first conversion error stored by [`apr`](Self::apr) /
    /// [`assets`](Self::assets), or [`SideNotSet`](BuildError::SideNotSet) /
    /// [`TickNotSet`](BuildError::TickNotSet) if a mandatory field was left at its default. Unlike
    /// [`try_build`](Self::try_build) this does not run offer validation.
    pub fn build_checked(self) -> Result<Offer, BuildError> {
        if let Some(e) = self.error {
            return Err(e);
        }
        if !self.side_set {
            return Err(BuildError::SideNotSet);
        }
        if !self.tick_set {
            return Err(BuildError::TickNotSet);
        }
        if !self.expiry_set {
            return Err(BuildError::ExpiryNotSet);
        }
        Ok(self.build())
    }

    /// Produce the `Offer` without validating it.
    ///
    /// Note: this ignores any error stored by [`apr`](Self::apr) / [`assets`](Self::assets) and
    /// does not enforce that a side or tick was ever set (an unset side builds as buy, an unset
    /// tick as 0); use [`build_checked`](Self::build_checked) (or [`try_build`](Self::try_build))
    /// for those guarantees.
    pub fn build(self) -> Offer {
        Offer {
            market: self.market,
            buy: self.buy,
            maker: self.maker,
            start: u256_to_word(self.start),
            expiry: u256_to_word(self.expiry),
            tick: u256_to_word(self.tick),
            group: self.group,
            callback: self.callback,
            callback_data: self.callback_data,
            receiver_if_maker_is_seller: self.receiver_if_maker_is_seller,
            ratifier: self.ratifier,
            reduce_only: self.reduce_only,
            max_units: self.max_units,
            max_assets: self.max_assets,
            continuous_fee_cap: u256_to_word(self.continuous_fee_cap),
        }
    }

    /// Produce the `Offer`, or the list of [`OfferError`]s if the side or tick was never set or if
    /// it fails validation against `ctx`.
    pub fn try_build(self, ctx: &ValidateCtx) -> Result<Offer, Vec<OfferError>> {
        let mut errs = Vec::new();
        // The wire `Offer` cannot represent "unset", so these builder-only checks run here rather
        // than in `validate_offer`.
        if !self.side_set {
            errs.push(OfferError::SideNotSet);
        }
        if !self.tick_set {
            errs.push(OfferError::TickNotSet);
        }
        if !self.expiry_set {
            errs.push(OfferError::ExpiryNotSet);
        }
        let offer = self.build();
        errs.extend(validate_offer(&offer, ctx));
        if errs.is_empty() {
            Ok(offer)
        } else {
            Err(errs)
        }
    }
}
