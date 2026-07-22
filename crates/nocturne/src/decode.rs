//! Decode Midnight offers and on-chain state into typed Rust.
//!
//! [`decode_offer`] is the inverse of Solidity `abi.encode(offer)` for the dynamic `Offer`
//! tuple (it carries a dynamic `Market`, a dynamic `CollateralParams[]`, and dynamic
//! `callbackData`). The state decoders ([`decode_market_state`], [`decode_position`],
//! [`decode_consumed`]) parse the raw `bytes` returned by an `eth_call` against Midnight's
//! `marketState`, `position`, and consumed-units getters, where each field is ABI-encoded as
//! its own 32-byte word in declaration order.
//!
//! Malformed input never panics: every path returns a [`DecodeError`].

use crate::{
    word_to_u256, Address, CollateralParams, Market, MarketSnapshot, Offer, Position, Sig,
    SimMarket, Word, U256,
};

/// Failure decoding ABI-encoded offer or state bytes.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum DecodeError {
    /// The input ended before a required word or byte range.
    #[error("input too short: needed {needed} bytes, have {have}")]
    TooShort {
        /// Byte length the decode required.
        needed: usize,
        /// Byte length actually available.
        have: usize,
    },
    /// A dynamic-field offset was unaddressable (overflowed or pointed past the buffer).
    #[error("bad offset: {0}")]
    BadOffset(usize),
    /// A length prefix was unaddressable (overflowed the buffer).
    #[error("bad length: {0}")]
    BadLength(usize),
    /// A word held a value too large for its target integer type.
    #[error("integer overflow: value does not fit its target type")]
    IntegerOverflow,
    /// A `bool` word was neither 0 nor 1 (or had dirty high bytes).
    #[error("invalid bool encoding")]
    InvalidBool,
    /// The calldata's leading 4-byte selector did not match the expected function.
    #[error("unexpected function selector: {0:02x?}")]
    BadSelector([u8; 4]),
}

// ---- word-level readers ------------------------------------------------------

/// Read the 32-byte word at absolute byte offset `off`.
fn word_at(bytes: &[u8], off: usize) -> Result<Word, DecodeError> {
    let end = off.checked_add(32).ok_or(DecodeError::BadOffset(off))?;
    if end > bytes.len() {
        return Err(DecodeError::TooShort {
            needed: end,
            have: bytes.len(),
        });
    }
    let mut w = [0u8; 32];
    w.copy_from_slice(&bytes[off..end]);
    Ok(w)
}

/// The `k`-th head word of a tuple whose head starts at absolute offset `base`.
fn head_word(bytes: &[u8], base: usize, k: usize) -> Result<Word, DecodeError> {
    let rel = k.checked_mul(32).ok_or(DecodeError::BadOffset(k))?;
    let off = base.checked_add(rel).ok_or(DecodeError::BadOffset(base))?;
    word_at(bytes, off)
}

/// Interpret a word as an unsigned integer that must fit in `usize` (for offsets/lengths).
fn word_to_usize(w: &Word) -> Result<usize, DecodeError> {
    if w[..24].iter().any(|&b| b != 0) {
        return Err(DecodeError::IntegerOverflow);
    }
    let mut b = [0u8; 8];
    b.copy_from_slice(&w[24..]);
    usize::try_from(u64::from_be_bytes(b)).map_err(|_| DecodeError::IntegerOverflow)
}

/// The last 20 bytes of a word as an address (Solidity right-aligns addresses).
fn word_to_address(w: &Word) -> Address {
    let mut a = [0u8; 20];
    a.copy_from_slice(&w[12..]);
    a
}

/// A word encoding a `bool`: high 31 bytes must be zero, last byte 0 or 1.
fn word_to_bool(w: &Word) -> Result<bool, DecodeError> {
    if w[..31].iter().any(|&b| b != 0) {
        return Err(DecodeError::InvalidBool);
    }
    match w[31] {
        0 => Ok(false),
        1 => Ok(true),
        _ => Err(DecodeError::InvalidBool),
    }
}

/// A word encoding a `uint128`: high 16 bytes must be zero.
fn word_to_u128(w: &Word) -> Result<u128, DecodeError> {
    crate::word_to_u128(w).ok_or(DecodeError::IntegerOverflow)
}

/// A word encoding a small unsigned integer held in its last `n` bytes (`n <= 8`); the
/// remaining high bytes must be zero.
fn word_to_small(w: &Word, n: usize) -> Result<u64, DecodeError> {
    if w[..32 - n].iter().any(|&b| b != 0) {
        return Err(DecodeError::IntegerOverflow);
    }
    let mut b = [0u8; 8];
    b[8 - n..].copy_from_slice(&w[32 - n..]);
    Ok(u64::from_be_bytes(b))
}

/// Absolute offset of a dynamic field: `base + offset_word` with overflow checks.
fn resolve(base: usize, offset: usize) -> Result<usize, DecodeError> {
    base.checked_add(offset)
        .ok_or(DecodeError::BadOffset(offset))
}

// ---- offer decoding ----------------------------------------------------------

/// Decode Solidity `abi.encode(offer)` back into an [`Offer`].
///
/// `abi.encode` of a single dynamic tuple emits one head word (an offset, `0x20`) followed by
/// the tuple body; this reads that offset and decodes the tuple it points at.
pub fn decode_offer(bytes: &[u8]) -> Result<Offer, DecodeError> {
    let tuple_off = word_to_usize(&word_at(bytes, 0)?)?;
    decode_offer_tuple(bytes, tuple_off)
}

/// Decode an `Offer` tuple whose head begins at absolute offset `base`. Dynamic-field offsets
/// inside the tuple are relative to `base`.
fn decode_offer_tuple(bytes: &[u8], base: usize) -> Result<Offer, DecodeError> {
    let market_off = word_to_usize(&head_word(bytes, base, 0)?)?;
    let buy = word_to_bool(&head_word(bytes, base, 1)?)?;
    let maker = word_to_address(&head_word(bytes, base, 2)?);
    let start = head_word(bytes, base, 3)?;
    let expiry = head_word(bytes, base, 4)?;
    let tick = head_word(bytes, base, 5)?;
    let group = head_word(bytes, base, 6)?;
    let callback = word_to_address(&head_word(bytes, base, 7)?);
    let callback_data_off = word_to_usize(&head_word(bytes, base, 8)?)?;
    let receiver_if_maker_is_seller = word_to_address(&head_word(bytes, base, 9)?);
    let ratifier = word_to_address(&head_word(bytes, base, 10)?);
    let reduce_only = word_to_bool(&head_word(bytes, base, 11)?)?;
    let max_units = word_to_u128(&head_word(bytes, base, 12)?)?;
    let max_assets = word_to_u128(&head_word(bytes, base, 13)?)?;
    let continuous_fee_cap = head_word(bytes, base, 14)?;

    let market = decode_market_tuple(bytes, resolve(base, market_off)?)?;
    let callback_data = decode_bytes(bytes, resolve(base, callback_data_off)?)?;

    Ok(Offer {
        market,
        buy,
        maker,
        start,
        expiry,
        tick,
        group,
        callback,
        callback_data,
        receiver_if_maker_is_seller,
        ratifier,
        reduce_only,
        max_units,
        max_assets,
        continuous_fee_cap,
    })
}

/// Decode a `Market` tuple whose head begins at absolute offset `base`.
fn decode_market_tuple(bytes: &[u8], base: usize) -> Result<Market, DecodeError> {
    let chain_id = head_word(bytes, base, 0)?;
    let midnight = word_to_address(&head_word(bytes, base, 1)?);
    let loan_token = word_to_address(&head_word(bytes, base, 2)?);
    let cp_off = word_to_usize(&head_word(bytes, base, 3)?)?;
    let maturity = head_word(bytes, base, 4)?;
    let rcf_threshold = head_word(bytes, base, 5)?;
    let enter_gate = word_to_address(&head_word(bytes, base, 6)?);
    let liquidator_gate = word_to_address(&head_word(bytes, base, 7)?);

    let collateral_params = decode_collateral_params(bytes, resolve(base, cp_off)?)?;

    Ok(Market {
        chain_id,
        midnight,
        loan_token,
        collateral_params,
        maturity,
        rcf_threshold,
        enter_gate,
        liquidator_gate,
    })
}

/// Decode a `CollateralParams[]` whose length word is at absolute offset `base`.
///
/// `CollateralParams` is fully static (4 words), so the array is a length word followed by
/// the elements inline with no per-element offsets.
fn decode_collateral_params(
    bytes: &[u8],
    base: usize,
) -> Result<Vec<CollateralParams>, DecodeError> {
    let len = word_to_usize(&word_at(bytes, base)?)?;
    let elems_base = resolve(base, 32)?;
    // Guard against an absurd length before allocating.
    let span = len.checked_mul(128).ok_or(DecodeError::BadLength(len))?;
    let end = elems_base
        .checked_add(span)
        .ok_or(DecodeError::BadLength(len))?;
    if end > bytes.len() {
        return Err(DecodeError::TooShort {
            needed: end,
            have: bytes.len(),
        });
    }

    let mut out = Vec::with_capacity(len);
    for i in 0..len {
        let cp_base = resolve(
            elems_base,
            i.checked_mul(128).ok_or(DecodeError::BadLength(len))?,
        )?;
        out.push(CollateralParams {
            token: word_to_address(&head_word(bytes, cp_base, 0)?),
            lltv: head_word(bytes, cp_base, 1)?,
            liquidation_cursor: head_word(bytes, cp_base, 2)?,
            oracle: word_to_address(&head_word(bytes, cp_base, 3)?),
        });
    }
    Ok(out)
}

/// Decode ABI `bytes` whose length word is at absolute offset `base`.
fn decode_bytes(bytes: &[u8], base: usize) -> Result<Vec<u8>, DecodeError> {
    let len = word_to_usize(&word_at(bytes, base)?)?;
    let start = resolve(base, 32)?;
    let end = start.checked_add(len).ok_or(DecodeError::BadLength(len))?;
    if end > bytes.len() {
        return Err(DecodeError::TooShort {
            needed: end,
            have: bytes.len(),
        });
    }
    Ok(bytes[start..end].to_vec())
}

/// Decode an ABI `bytes32[]` (dynamic array of static words) whose length word is at absolute
/// offset `base`: a length word followed by that many inline words.
fn decode_word_array(bytes: &[u8], base: usize) -> Result<Vec<Word>, DecodeError> {
    let len = word_to_usize(&word_at(bytes, base)?)?;
    let elems_base = resolve(base, 32)?;
    let span = len.checked_mul(32).ok_or(DecodeError::BadLength(len))?;
    let end = elems_base
        .checked_add(span)
        .ok_or(DecodeError::BadLength(len))?;
    if end > bytes.len() {
        return Err(DecodeError::TooShort {
            needed: end,
            have: bytes.len(),
        });
    }
    let mut out = Vec::with_capacity(len);
    for i in 0..len {
        let off = resolve(
            elems_base,
            i.checked_mul(32).ok_or(DecodeError::BadLength(len))?,
        )?;
        out.push(word_at(bytes, off)?);
    }
    Ok(out)
}

// ---- on-chain state decoding -------------------------------------------------

/// Decoded return of Midnight's `marketState(id)` getter.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MarketStateView {
    /// Total open units in the market.
    pub total_units: u128,
    /// Current loss factor (`u128::MAX` means maxed out / market bankrupt).
    pub loss_factor: u128,
    /// Withdrawable balance.
    pub withdrawable: u128,
    /// Accrued continuous-fee credit.
    pub continuous_fee_credit: u128,
    /// Settlement-fee breakpoints `settlementFeeCbp0..6`.
    pub settlement_fee_cbp: [u16; 7],
    /// Continuous fee (on-chain `uint32`).
    pub continuous_fee: u32,
    /// Tick spacing (0 means the market isn't created).
    pub tick_spacing: u8,
}

impl MarketStateView {
    /// Whether the loss factor is maxed out.
    fn loss_factor_maxed(&self) -> bool {
        self.loss_factor == u128::MAX
    }

    /// Project into the simulator's market view.
    pub fn to_sim_market(&self) -> SimMarket {
        SimMarket {
            tick_spacing: self.tick_spacing,
            continuous_fee: self.continuous_fee as u128,
            settlement_fee_cbp: self.settlement_fee_cbp,
            loss_factor_maxed: self.loss_factor_maxed(),
        }
    }

    /// Project into the validator's market snapshot.
    pub fn to_market_snapshot(&self) -> MarketSnapshot {
        MarketSnapshot {
            tick_spacing: self.tick_spacing,
            loss_factor_maxed: self.loss_factor_maxed(),
            continuous_fee: self.continuous_fee as u128,
        }
    }
}

/// Decode the raw `eth_call` return of `marketState(id)`.
///
/// Field order and widths mirror the getter: `totalUnits(u128)`, `lossFactor(u128)`,
/// `withdrawable(u128)`, `continuousFeeCredit(u128)`, `settlementFeeCbp0..6` (7×u16),
/// `continuousFee(u32)`, `tickSpacing(u8)` - each padded to its own 32-byte word.
pub fn decode_market_state(bytes: &[u8]) -> Result<MarketStateView, DecodeError> {
    let total_units = word_to_u128(&head_word(bytes, 0, 0)?)?;
    let loss_factor = word_to_u128(&head_word(bytes, 0, 1)?)?;
    let withdrawable = word_to_u128(&head_word(bytes, 0, 2)?)?;
    let continuous_fee_credit = word_to_u128(&head_word(bytes, 0, 3)?)?;

    let mut settlement_fee_cbp = [0u16; 7];
    for (i, slot) in settlement_fee_cbp.iter_mut().enumerate() {
        *slot = word_to_small(&head_word(bytes, 0, 4 + i)?, 2)? as u16;
    }

    let continuous_fee = word_to_small(&head_word(bytes, 0, 11)?, 4)? as u32;
    let tick_spacing = word_to_small(&head_word(bytes, 0, 12)?, 1)? as u8;

    Ok(MarketStateView {
        total_units,
        loss_factor,
        withdrawable,
        continuous_fee_credit,
        settlement_fee_cbp,
        continuous_fee,
        tick_spacing,
    })
}

/// Decoded return of Midnight's `position(id, user)` getter (the fixed `collateral[128]` array
/// is not part of the public-mapping getter's return).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PositionView {
    /// Outstanding lender credit.
    pub credit: u128,
    /// Accrued continuous fee owed.
    pub pending_fee: u128,
    /// Loss factor snapshot at last accrual.
    pub last_loss_factor: u128,
    /// Timestamp of last accrual.
    pub last_accrual: u128,
    /// Outstanding borrower debt.
    pub debt: u128,
    /// Bitmap of collateral tokens in use.
    pub collateral_bitmap: u128,
}

impl PositionView {
    /// Project into the simulator's position view.
    pub fn to_sim_position(&self) -> Position {
        Position {
            credit: self.credit,
            debt: self.debt,
            pending_fee: self.pending_fee,
        }
    }
}

/// Decode the raw `eth_call` return of `position(id, user)`.
///
/// Field order mirrors the getter: `credit`, `pendingFee`, `lastLossFactor`, `lastAccrual`,
/// `debt`, `collateralBitmap` - each a `uint128` in its own 32-byte word.
pub fn decode_position(bytes: &[u8]) -> Result<PositionView, DecodeError> {
    Ok(PositionView {
        credit: word_to_u128(&head_word(bytes, 0, 0)?)?,
        pending_fee: word_to_u128(&head_word(bytes, 0, 1)?)?,
        last_loss_factor: word_to_u128(&head_word(bytes, 0, 2)?)?,
        last_accrual: word_to_u128(&head_word(bytes, 0, 3)?)?,
        debt: word_to_u128(&head_word(bytes, 0, 4)?)?,
        collateral_bitmap: word_to_u128(&head_word(bytes, 0, 5)?)?,
    })
}

/// Decode a single `uint128` word (e.g. a consumed-units getter return).
pub fn decode_consumed(bytes: &[u8]) -> Result<u128, DecodeError> {
    word_to_u128(&word_at(bytes, 0)?)
}

// ---- calldata decoding -------------------------------------------------------

/// The `EcrecoverRatifier` ratifier data, decoded - the inverse of
/// [`encode_ratifier_data`](crate::encode_ratifier_data).
///
/// This is what a taker submits as the `bytes ratifierData` argument to `take`, and what the
/// maker's signature commits to: `sig` signs the EIP-712 digest over (`root`, tree height),
/// where the tree height is `proof.len()`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RatifierData {
    /// The maker's signature over the tree digest.
    pub sig: Sig,
    /// The signed Merkle root.
    pub root: Word,
    /// The offer's leaf index in the tree.
    pub leaf_index: usize,
    /// The Merkle proof from the leaf to `root`; its length is the tree height.
    pub proof: Vec<Word>,
}

/// Decode `abi.encode(Signature{uint8 v, bytes32 r, bytes32 s}, bytes32 root, uint256 leafIndex,
/// bytes32[] proof)` - the inverse of [`encode_ratifier_data`](crate::encode_ratifier_data).
pub fn decode_ratifier_data(bytes: &[u8]) -> Result<RatifierData, DecodeError> {
    // `Signature` is a static 3-word tuple inlined in the head: v (padded), r, s.
    let v = word_to_small(&word_at(bytes, 0)?, 1)? as u8;
    let r = word_at(bytes, 32)?;
    let s = word_at(bytes, 64)?;
    let root = word_at(bytes, 96)?;
    let leaf_index = word_to_usize(&word_at(bytes, 128)?)?;
    // `proof` is a dynamic array: head word 5 is its offset from the start of this blob.
    let proof_off = word_to_usize(&word_at(bytes, 160)?)?;
    let proof = decode_word_array(bytes, proof_off)?;
    Ok(RatifierData {
        sig: Sig { r, s, v },
        root,
        leaf_index,
        proof,
    })
}

/// A decoded `Midnight.take` call - the inverse of
/// [`encode_take_calldata`](crate::encode_take_calldata).
///
/// Everything a taker's transaction carries, split back into typed fields. `ratifier_data` is
/// the already-decoded [`RatifierData`]; `ratifier_data_raw` keeps the original bytes so callers
/// can re-verify the exact blob that was signed over / submitted.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TakeCall {
    /// The offer being taken.
    pub offer: Offer,
    /// The raw `ratifierData` bytes as they appear in the calldata.
    pub ratifier_data_raw: Vec<u8>,
    /// The decoded `ratifierData` (signature, root, leaf index, proof).
    pub ratifier_data: RatifierData,
    /// Units the taker is consuming.
    pub units: U256,
    /// The taker address.
    pub taker: Address,
    /// Receiver of assets when the taker is the seller (buy offers).
    pub receiver_if_taker_is_seller: Address,
    /// Optional taker callback target (zero if unused).
    pub taker_callback: Address,
    /// Optional taker callback data.
    pub taker_callback_data: Vec<u8>,
}

/// Decode raw `Midnight.take(...)` calldata (4-byte selector + ABI args) into a [`TakeCall`] -
/// the inverse of [`encode_take_calldata`](crate::encode_take_calldata).
///
/// Errors [`DecodeError::BadSelector`] if the leading selector is not
/// [`TAKE_SELECTOR`](crate::TAKE_SELECTOR).
pub fn decode_take_calldata(bytes: &[u8]) -> Result<TakeCall, DecodeError> {
    let args = strip_selector(bytes, crate::TAKE_SELECTOR)?;
    // Head: [offer offset, ratifierData offset, units, taker, receiver, callback, callbackData off]
    let offer_off = word_to_usize(&head_word(args, 0, 0)?)?;
    let ratifier_off = word_to_usize(&head_word(args, 0, 1)?)?;
    let units = word_to_u256(&head_word(args, 0, 2)?);
    let taker = word_to_address(&head_word(args, 0, 3)?);
    let receiver_if_taker_is_seller = word_to_address(&head_word(args, 0, 4)?);
    let taker_callback = word_to_address(&head_word(args, 0, 5)?);
    let callback_data_off = word_to_usize(&head_word(args, 0, 6)?)?;

    let offer = decode_offer_tuple(args, offer_off)?;
    let ratifier_data_raw = decode_bytes(args, ratifier_off)?;
    let ratifier_data = decode_ratifier_data(&ratifier_data_raw)?;
    let taker_callback_data = decode_bytes(args, callback_data_off)?;

    Ok(TakeCall {
        offer,
        ratifier_data_raw,
        ratifier_data,
        units,
        taker,
        receiver_if_taker_is_seller,
        taker_callback,
        taker_callback_data,
    })
}

/// Decode raw `EcrecoverRatifier.cancelRoot(address maker, bytes32 root)` calldata - the inverse
/// of [`encode_cancel_root_calldata`](crate::encode_cancel_root_calldata).
pub fn decode_cancel_root_calldata(bytes: &[u8]) -> Result<(Address, Word), DecodeError> {
    let args = strip_selector(bytes, crate::CANCEL_ROOT_SELECTOR)?;
    let maker = word_to_address(&head_word(args, 0, 0)?);
    let root = head_word(args, 0, 1)?;
    Ok((maker, root))
}

/// Verify the leading 4-byte selector and return the ABI-args slice that follows it.
fn strip_selector(bytes: &[u8], expected: [u8; 4]) -> Result<&[u8], DecodeError> {
    if bytes.len() < 4 {
        return Err(DecodeError::TooShort {
            needed: 4,
            have: bytes.len(),
        });
    }
    let selector = [bytes[0], bytes[1], bytes[2], bytes[3]];
    if selector != expected {
        return Err(DecodeError::BadSelector(selector));
    }
    Ok(&bytes[4..])
}
