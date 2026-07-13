//! Ergonomic builders for `Market` and `Offer`.
//!
//! The wire structs store every `uint256` as a raw big-endian `Word`, which is easy to get
//! wrong by hand. These builders take typed inputs (`u64` / `u128` / [`U256`] / `Address`),
//! apply sensible defaults, and pack the `Word`s for you. `try_build` additionally runs
//! [`validate_offer`](crate::validate_offer) so a malformed offer never leaves the builder.

use crate::{
    u256_to_word, validate_offer, word_from_u64, Address, CollateralParams, Market, Offer,
    OfferError, ValidateCtx, Word, U256,
};

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
    pub fn collateral(mut self, token: Address, lltv: U256, liquidation_cursor: U256, oracle: Address) -> Self {
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
/// Defaults: `start = 0`, `expiry = u256::MAX` (never expires until set), no callback, no
/// receiver, `reduce_only = false`, `continuous_fee_cap = 0`. You must set a side, a tick, a
/// ratifier, and exactly one of [`max_units`](Self::max_units) / [`max_assets`](Self::max_assets).
#[derive(Clone, Debug)]
#[must_use = "call `.build()` or `.try_build(..)` to produce the Offer"]
pub struct OfferBuilder {
    market: Market,
    buy: bool,
    maker: Address,
    start: U256,
    expiry: U256,
    tick: U256,
    group: Word,
    callback: Address,
    callback_data: Vec<u8>,
    receiver_if_maker_is_seller: Address,
    ratifier: Address,
    reduce_only: bool,
    max_units: u128,
    max_assets: u128,
    continuous_fee_cap: U256,
}

impl OfferBuilder {
    /// Start an offer in `market` made by `maker`.
    pub fn new(market: Market, maker: Address) -> Self {
        Self {
            market,
            buy: true,
            maker,
            start: U256::ZERO,
            expiry: U256::MAX,
            tick: U256::ZERO,
            group: [0u8; 32],
            callback: [0u8; 20],
            callback_data: Vec::new(),
            receiver_if_maker_is_seller: [0u8; 20],
            ratifier: [0u8; 20],
            reduce_only: false,
            max_units: 0,
            max_assets: 0,
            continuous_fee_cap: U256::ZERO,
        }
    }

    /// `buy = true` (maker is the buyer) or `false` (maker is the seller).
    pub fn side(mut self, buy: bool) -> Self {
        self.buy = buy;
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

    /// Set the price tick.
    pub fn tick(mut self, tick: u64) -> Self {
        self.tick = U256::from(tick);
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

    /// Set the receiver used when the maker is the seller (must stay zero for buy offers).
    pub fn receiver_if_maker_is_seller(mut self, receiver: Address) -> Self {
        self.receiver_if_maker_is_seller = receiver;
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

    /// Produce the `Offer` without validating it.
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

    /// Produce the `Offer`, or the list of [`OfferError`]s if it fails validation against `ctx`.
    pub fn try_build(self, ctx: &ValidateCtx) -> Result<Offer, Vec<OfferError>> {
        let offer = self.build();
        let errs = validate_offer(&offer, ctx);
        if errs.is_empty() {
            Ok(offer)
        } else {
            Err(errs)
        }
    }
}
