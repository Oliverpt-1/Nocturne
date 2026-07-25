//! ABI-encode the `take` / `cancelRoot` calls and the `EcrecoverRatifier` ratifier data.
//!
//! Byte-for-byte mirror of Solidity `abi.encode` / `abi.encodeCall`: 32-byte words, with
//! dynamic values (`bytes`, arrays, tuples containing dynamic members) laid out as an offset
//! in the head and their contents appended to the tail. Parity is asserted in `tests/codec.rs`
//! against constants produced by `fixtures/GenCodec.t.sol` run against the Midnight contracts.

use crate::{Address, Offer, Sig, Word, U256};

// ---- 4-byte function selectors, baked from `fixtures/GenCodec.t.sol` ----

/// `Midnight.take(Offer,bytes,uint256,address,address,address,bytes)` selector.
pub const TAKE_SELECTOR: [u8; 4] = [0x6a, 0x14, 0xc9, 0xef];
/// `EcrecoverRatifier.cancelRoot(address,bytes32)` selector.
pub const CANCEL_ROOT_SELECTOR: [u8; 4] = [0xbb, 0x1f, 0x12, 0xaa];

// `IMidnightBundlesV1` fill-function selectors, derived with `cast sig` from the vendored
// interface (morpho-apps `IMidnightBundles.sol`); the BuyWithAssetsTarget selector is also
// cross-checked against production calldata and the openchain signature database.

/// `midnightBundlesV1BuyWithUnitsTargetAndWithdrawCollateral(...)` selector.
pub const BUNDLE_BUY_UNITS_SELECTOR: [u8; 4] = [0xff, 0xb4, 0x58, 0x4e];
/// `midnightBundlesV1SupplyCollateralAndSellWithUnitsTarget(...)` selector.
pub const BUNDLE_SELL_UNITS_SELECTOR: [u8; 4] = [0xb7, 0x07, 0xc6, 0x08];
/// `midnightBundlesV1BuyWithAssetsTargetAndWithdrawCollateral(...)` selector.
pub const BUNDLE_BUY_ASSETS_SELECTOR: [u8; 4] = [0xa8, 0x5d, 0x52, 0xe5];
/// `midnightBundlesV1SupplyCollateralAndSellWithAssetsTarget(...)` selector.
pub const BUNDLE_SELL_ASSETS_SELECTOR: [u8; 4] = [0x2f, 0xbc, 0x23, 0xf8];

// ---- word packing helpers ----

#[inline]
fn addr_word(a: &Address) -> Word {
    let mut w = [0u8; 32];
    w[12..].copy_from_slice(a);
    w
}

#[inline]
fn bool_word(b: bool) -> Word {
    let mut w = [0u8; 32];
    w[31] = b as u8;
    w
}

#[inline]
fn u128_word(x: u128) -> Word {
    let mut w = [0u8; 32];
    w[16..].copy_from_slice(&x.to_be_bytes());
    w
}

#[inline]
fn usize_word(x: usize) -> Word {
    let mut w = [0u8; 32];
    w[24..].copy_from_slice(&(x as u64).to_be_bytes());
    w
}

// ---- ABI value model ----

/// A minimal ABI value tree, enough to encode the Midnight `take` / ratifier types.
enum Value {
    /// A single pre-padded 32-byte word (uint256, bytes32, address, bool, uint128, ...).
    Word(Word),
    /// Dynamic `bytes` / `string`.
    Bytes(Vec<u8>),
    /// A dynamic-length array; each element carries its own `Value`.
    Array(Vec<Value>),
    /// A tuple / struct; dynamic iff any component is dynamic.
    Tuple(Vec<Value>),
}

impl Value {
    fn is_dynamic(&self) -> bool {
        match self {
            Value::Word(_) => false,
            Value::Bytes(_) | Value::Array(_) => true,
            Value::Tuple(items) => items.iter().any(Value::is_dynamic),
        }
    }

    /// Size in bytes of the inline (head) encoding - only defined for static values.
    fn static_size(&self) -> usize {
        match self {
            Value::Word(_) => 32,
            Value::Tuple(items) => items.iter().map(Value::static_size).sum(),
            Value::Bytes(_) | Value::Array(_) => {
                unreachable!("static_size on a dynamic value")
            }
        }
    }

    /// Full encoding of this value when it stands on its own (in a tail slot).
    fn encode(&self) -> Vec<u8> {
        match self {
            Value::Word(w) => w.to_vec(),
            Value::Bytes(b) => {
                let mut out = usize_word(b.len()).to_vec();
                out.extend_from_slice(b);
                let rem = b.len() % 32;
                if rem != 0 {
                    out.extend(std::iter::repeat(0u8).take(32 - rem));
                }
                out
            }
            Value::Array(items) => {
                let mut out = usize_word(items.len()).to_vec();
                out.extend(encode_sequence(items));
                out
            }
            Value::Tuple(items) => encode_sequence(items),
        }
    }
}

/// Encode a sequence of values with the standard head/tail split (this is exactly what
/// `abi.encode(a, b, c, ...)` does, and also how a dynamic tuple encodes its components).
fn encode_sequence(values: &[Value]) -> Vec<u8> {
    let head_size: usize = values
        .iter()
        .map(|v| if v.is_dynamic() { 32 } else { v.static_size() })
        .sum();

    let mut head = Vec::with_capacity(head_size);
    let mut tail = Vec::new();
    for v in values {
        if v.is_dynamic() {
            let offset = head_size + tail.len();
            head.extend_from_slice(&usize_word(offset));
            tail.extend(v.encode());
        } else {
            head.extend(v.encode());
        }
    }
    head.extend(tail);
    head
}

// ---- Offer / Market as ABI values ----

fn collateral_params_value(cp: &crate::CollateralParams) -> Value {
    Value::Tuple(vec![
        Value::Word(addr_word(&cp.token)),
        Value::Word(cp.lltv),
        Value::Word(cp.liquidation_cursor),
        Value::Word(addr_word(&cp.oracle)),
    ])
}

fn market_value(m: &crate::Market) -> Value {
    Value::Tuple(vec![
        Value::Word(m.chain_id),
        Value::Word(addr_word(&m.midnight)),
        Value::Word(addr_word(&m.loan_token)),
        Value::Array(
            m.collateral_params
                .iter()
                .map(collateral_params_value)
                .collect(),
        ),
        Value::Word(m.maturity),
        Value::Word(m.rcf_threshold),
        Value::Word(addr_word(&m.enter_gate)),
        Value::Word(addr_word(&m.liquidator_gate)),
    ])
}

fn offer_value(o: &Offer) -> Value {
    Value::Tuple(vec![
        market_value(&o.market),
        Value::Word(bool_word(o.buy)),
        Value::Word(addr_word(&o.maker)),
        Value::Word(o.start),
        Value::Word(o.expiry),
        Value::Word(o.tick),
        Value::Word(o.group),
        Value::Word(addr_word(&o.callback)),
        Value::Bytes(o.callback_data.clone()),
        Value::Word(addr_word(&o.receiver_if_maker_is_seller)),
        Value::Word(addr_word(&o.ratifier)),
        Value::Word(bool_word(o.reduce_only)),
        Value::Word(u128_word(o.max_units)),
        Value::Word(u128_word(o.max_assets)),
        Value::Word(o.continuous_fee_cap),
    ])
}

// ---- public encoders ----

/// `abi.encode(Signature{uint8 v, bytes32 r, bytes32 s}, bytes32 root, uint256 leafIndex,
/// bytes32[] proof)` - the `ratifierData` consumed by `EcrecoverRatifier.isRatified`.
///
/// `Signature` is a fully static tuple, so it is inlined as three head words (`v` padded, `r`,
/// `s`); `root` and `leafIndex` are static words; `proof` is a dynamic array (offset in the head,
/// length + elements in the tail).
pub fn encode_ratifier_data(sig: &Sig, root: &Word, leaf_index: usize, proof: &[Word]) -> Vec<u8> {
    let signature = Value::Tuple(vec![
        Value::Word(usize_word(sig.v as usize)),
        Value::Word(sig.r),
        Value::Word(sig.s),
    ]);
    let proof_arr = Value::Array(proof.iter().map(|w| Value::Word(*w)).collect());
    encode_sequence(&[
        signature,
        Value::Word(*root),
        Value::Word(usize_word(leaf_index)),
        proof_arr,
    ])
}

/// `Midnight.take.selector ++ abi.encode(offer, ratifierData, units, taker,
/// receiverIfTakerIsSeller, takerCallback, takerCallbackData)`.
#[allow(clippy::too_many_arguments)]
pub fn encode_take_calldata(
    offer: &Offer,
    ratifier_data: &[u8],
    units: U256,
    taker: &Address,
    receiver_if_taker_is_seller: &Address,
    taker_callback: &Address,
    taker_callback_data: &[u8],
) -> Vec<u8> {
    let args = encode_sequence(&[
        offer_value(offer),
        Value::Bytes(ratifier_data.to_vec()),
        Value::Word(units.to_be_bytes::<32>()),
        Value::Word(addr_word(taker)),
        Value::Word(addr_word(receiver_if_taker_is_seller)),
        Value::Word(addr_word(taker_callback)),
        Value::Bytes(taker_callback_data.to_vec()),
    ]);
    let mut out = Vec::with_capacity(4 + args.len());
    out.extend_from_slice(&TAKE_SELECTOR);
    out.extend(args);
    out
}

/// `IMidnightBundles` fill calldata: the kind's selector ++ its ABI-encoded arguments - the
/// inverse of [`decode_bundle_calldata`](crate::decode_bundle_calldata).
///
/// The wrapper argument order per kind mirrors `IMidnightBundlesV1`: buy variants carry a
/// `TokenPermit`, `OfferFill[]`, `CollateralWithdrawal[]`, and a collateral receiver; sell
/// variants a receiver, `CollateralSupply[]`, then `OfferFill[]`. Both end with
/// (referralFeePct, referralFeeRecipient, maxContinuousFee, deadline).
pub fn encode_bundle_calldata(b: &crate::BundleCall) -> Vec<u8> {
    let fills = Value::Array(
        b.fills
            .iter()
            .map(|f| {
                Value::Tuple(vec![
                    offer_value(&f.offer),
                    Value::Bytes(f.ratifier_data_raw.clone()),
                    Value::Word(f.units.to_be_bytes::<32>()),
                ])
            })
            .collect(),
    );
    let tail = [
        Value::Word(b.referral_fee_pct.to_be_bytes::<32>()),
        Value::Word(addr_word(&b.referral_fee_recipient)),
        Value::Word(b.max_continuous_fee.to_be_bytes::<32>()),
        Value::Word(b.deadline.to_be_bytes::<32>()),
    ];
    let common = [
        Value::Word(b.target.to_be_bytes::<32>()),
        Value::Word(b.limit.to_be_bytes::<32>()),
        Value::Word(addr_word(&b.taker)),
        Value::Word(bool_word(b.reduce_only)),
    ];
    let permit_value = |p: &crate::TokenPermit| {
        Value::Tuple(vec![
            Value::Word(usize_word(p.kind as usize)),
            Value::Bytes(p.data.clone()),
        ])
    };

    let mut args: Vec<Value> = common.into();
    match &b.side {
        crate::BundleSide::Buy {
            loan_token_permit,
            collateral_withdrawals,
            collateral_receiver,
        } => {
            args.push(permit_value(loan_token_permit));
            args.push(fills);
            args.push(Value::Array(
                collateral_withdrawals
                    .iter()
                    .map(|w| {
                        Value::Tuple(vec![
                            Value::Word(w.collateral_index.to_be_bytes::<32>()),
                            Value::Word(w.assets.to_be_bytes::<32>()),
                        ])
                    })
                    .collect(),
            ));
            args.push(Value::Word(addr_word(collateral_receiver)));
        }
        crate::BundleSide::Sell {
            receiver,
            collateral_supplies,
        } => {
            args.push(Value::Word(addr_word(receiver)));
            args.push(Value::Array(
                collateral_supplies
                    .iter()
                    .map(|s| {
                        Value::Tuple(vec![
                            Value::Word(s.collateral_index.to_be_bytes::<32>()),
                            Value::Word(s.assets.to_be_bytes::<32>()),
                            permit_value(&s.permit),
                        ])
                    })
                    .collect(),
            ));
            args.push(fills);
        }
    }
    args.extend(tail);

    let encoded = encode_sequence(&args);
    let mut out = Vec::with_capacity(4 + encoded.len());
    out.extend_from_slice(&b.kind.selector());
    out.extend(encoded);
    out
}

/// `EcrecoverRatifier.cancelRoot.selector ++ abi.encode(maker, root)`.
pub fn encode_cancel_root_calldata(maker: &Address, root: &Word) -> Vec<u8> {
    let args = encode_sequence(&[Value::Word(addr_word(maker)), Value::Word(*root)]);
    let mut out = Vec::with_capacity(4 + args.len());
    out.extend_from_slice(&CANCEL_ROOT_SELECTOR);
    out.extend(args);
    out
}
