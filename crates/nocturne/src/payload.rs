//! Midnight maker-mempool payload framing and ABI codec.
//!
//! A payload is `version || uint32(gzip_len) || gzip(abi.encode(items))`, where each item is an
//! offer paired with the opaque ratifier data a taker later passes to `Midnight.take`.

use std::io::{Cursor, Read, Write};

use alloy_primitives::{Address as SolAddress, Bytes, FixedBytes};
use alloy_sol_types::{sol, SolValue};
use flate2::{read::GzDecoder, Compression, GzBuilder};

use crate::{
    word_to_u256, Address, CollateralParams, Market, Offer, MAX_COLLATERALS, MAX_TICK, U256,
};

/// Current payload wire version.
pub const PAYLOAD_VERSION: u8 = 1;
/// Largest complete framed payload accepted by the protocol ecosystem.
pub const MAX_PAYLOAD_BYTES: usize = 1_000_000;
/// Largest attribution suffix accepted after the declared gzip stream.
pub const MAX_ATTRIBUTION_SUFFIX_BYTES: usize = 256;
/// Largest compressed ABI payload, reserving space for the header and attribution suffix.
pub const MAX_COMPRESSED_ITEMS_BYTES: usize =
    MAX_PAYLOAD_BYTES - PAYLOAD_HEADER_BYTES - MAX_ATTRIBUTION_SUFFIX_BYTES;
/// Largest decompressed ABI item buffer.
pub const MAX_DECOMPRESSED_ITEMS_BYTES: usize = 6_000_000;

const PAYLOAD_HEADER_BYTES: usize = 5;
const WAD: u128 = 1_000_000_000_000_000_000;
const MAX_TIMESTAMP_SECONDS: u128 = 999_999_999_999;

sol! {
    struct WireCollateralParams {
        address token;
        uint256 lltv;
        uint256 liquidation_cursor;
        address oracle;
    }

    struct WireMarket {
        uint256 chain_id;
        address midnight;
        address loan_token;
        WireCollateralParams[] collateral_params;
        uint256 maturity;
        uint256 rcf_threshold;
        address enter_gate;
        address liquidator_gate;
    }

    struct WireOffer {
        WireMarket market;
        bool buy;
        address maker;
        uint256 start;
        uint256 expiry;
        uint256 tick;
        bytes32 group;
        address callback;
        bytes callback_data;
        address receiver_if_maker_is_seller;
        address ratifier;
        bool reduce_only;
        uint256 max_units;
        uint256 max_assets;
        uint256 continuous_fee_cap;
    }

    struct WirePayloadItem {
        WireOffer offer;
        bytes ratifier_data;
    }
}

/// One published maker offer and the ratifier data needed to take it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PayloadItem {
    pub offer: Offer,
    pub ratifier_data: Vec<u8>,
}

/// Errors returned by the mempool payload codec.
#[derive(Debug, thiserror::Error)]
pub enum PayloadError {
    #[error("items payload is empty")]
    Empty,
    #[error("payload exceeds {MAX_PAYLOAD_BYTES} bytes")]
    PayloadTooLarge,
    #[error("invalid version: expected {PAYLOAD_VERSION}, got {0}")]
    InvalidVersion(u8),
    #[error("payload too short for header")]
    HeaderTooShort,
    #[error("payload truncated before declared gzip length")]
    Truncated,
    #[error("trailing suffix exceeds {MAX_ATTRIBUTION_SUFFIX_BYTES} bytes: got {0}")]
    AttributionTooLarge(usize),
    #[error("compressed items exceed {MAX_COMPRESSED_ITEMS_BYTES} bytes")]
    CompressedTooLarge,
    #[error("decompressed items exceed {MAX_DECOMPRESSED_ITEMS_BYTES} bytes")]
    DecompressedTooLarge,
    #[error("items payload exceeds caller limit of {0} items")]
    TooManyItems(usize),
    #[error("compression failed: {0}")]
    Compression(std::io::Error),
    #[error("items ABI decode failed: {0}")]
    Abi(String),
    #[error("items payload has non-canonical ABI bytes")]
    NonCanonicalAbi,
    #[error("invalid offer bytes: {0}")]
    InvalidOffer(&'static str),
    #[error("decoded maxUnits or maxAssets does not fit uint128")]
    OfferCapOverflow,
}

/// Codec for the versioned, compressed payload published to `MidnightMempool`.
pub struct Payload;

impl Payload {
    /// Encode payload items into transaction-ready bytes.
    pub fn encode(items: &[PayloadItem]) -> Result<Vec<u8>, PayloadError> {
        if items.is_empty() {
            return Err(PayloadError::Empty);
        }
        for item in items {
            validate_payload_offer(&item.offer)?;
        }

        let abi = encode_items_abi(items);
        if abi.len() > MAX_DECOMPRESSED_ITEMS_BYTES {
            return Err(PayloadError::DecompressedTooLarge);
        }

        let mut encoder = GzBuilder::new()
            .mtime(0)
            .write(Vec::new(), Compression::default());
        encoder.write_all(&abi).map_err(PayloadError::Compression)?;
        let compressed = encoder.finish().map_err(PayloadError::Compression)?;
        if compressed.len() > MAX_COMPRESSED_ITEMS_BYTES {
            return Err(PayloadError::CompressedTooLarge);
        }

        let mut payload = Vec::with_capacity(PAYLOAD_HEADER_BYTES + compressed.len());
        payload.push(PAYLOAD_VERSION);
        payload.extend_from_slice(&(compressed.len() as u32).to_be_bytes());
        payload.extend_from_slice(&compressed);
        Ok(payload)
    }

    /// Decode a payload, ignoring a bounded attribution suffix after its gzip stream.
    pub fn decode(
        payload: &[u8],
        max_items: Option<usize>,
    ) -> Result<Vec<PayloadItem>, PayloadError> {
        if payload.len() > MAX_PAYLOAD_BYTES {
            return Err(PayloadError::PayloadTooLarge);
        }
        if payload.len() < PAYLOAD_HEADER_BYTES {
            return Err(PayloadError::HeaderTooShort);
        }
        if payload[0] != PAYLOAD_VERSION {
            return Err(PayloadError::InvalidVersion(payload[0]));
        }

        let compressed_len = u32::from_be_bytes(payload[1..5].try_into().unwrap()) as usize;
        if compressed_len > MAX_COMPRESSED_ITEMS_BYTES {
            return Err(PayloadError::CompressedTooLarge);
        }
        let compressed_end = PAYLOAD_HEADER_BYTES
            .checked_add(compressed_len)
            .ok_or(PayloadError::Truncated)?;
        if compressed_end > payload.len() {
            return Err(PayloadError::Truncated);
        }
        let suffix_len = payload.len() - compressed_end;
        if suffix_len > MAX_ATTRIBUTION_SUFFIX_BYTES {
            return Err(PayloadError::AttributionTooLarge(suffix_len));
        }

        let cursor = Cursor::new(&payload[PAYLOAD_HEADER_BYTES..compressed_end]);
        let decoder = GzDecoder::new(cursor);
        let mut limited = decoder.take((MAX_DECOMPRESSED_ITEMS_BYTES + 1) as u64);
        let mut abi = Vec::new();
        limited
            .read_to_end(&mut abi)
            .map_err(PayloadError::Compression)?;
        if abi.len() > MAX_DECOMPRESSED_ITEMS_BYTES {
            return Err(PayloadError::DecompressedTooLarge);
        }

        preflight_item_count(&abi, max_items)?;
        let items = decode_items_abi(&abi)?;
        if items.is_empty() {
            return Err(PayloadError::Empty);
        }
        if let Some(limit) = max_items {
            if limit == 0 || items.len() > limit {
                return Err(PayloadError::TooManyItems(limit));
            }
        }
        for item in &items {
            validate_payload_offer(&item.offer)?;
        }
        if encode_items_abi(&items) != abi {
            return Err(PayloadError::NonCanonicalAbi);
        }
        Ok(items)
    }
}

// ABI encodes a single dynamic array as offset=32 followed by a 256-bit item count. Check the
// count before asking the general decoder to allocate from untrusted input.
fn preflight_item_count(bytes: &[u8], max_items: Option<usize>) -> Result<(), PayloadError> {
    if bytes.len() < 64 {
        return Err(PayloadError::Abi(
            "items payload truncated before ABI array head".into(),
        ));
    }
    if bytes[..31].iter().any(|byte| *byte != 0) || bytes[31] != 32 {
        return Err(PayloadError::Abi("invalid items ABI array offset".into()));
    }
    if let Some(limit) = max_items {
        let count_word = &bytes[32..64];
        let oversized = count_word[..24].iter().any(|byte| *byte != 0)
            || u64::from_be_bytes(count_word[24..].try_into().unwrap()) > limit as u64;
        if oversized {
            return Err(PayloadError::TooManyItems(limit));
        }
    }
    Ok(())
}

fn encode_items_abi(items: &[PayloadItem]) -> Vec<u8> {
    items
        .iter()
        .map(|item| WirePayloadItem {
            offer: offer_to_wire(&item.offer),
            ratifier_data: Bytes::copy_from_slice(&item.ratifier_data),
        })
        .collect::<Vec<_>>()
        .abi_encode()
}

fn decode_items_abi(bytes: &[u8]) -> Result<Vec<PayloadItem>, PayloadError> {
    let items = Vec::<WirePayloadItem>::abi_decode(bytes)
        .map_err(|error| PayloadError::Abi(error.to_string()))?;
    items
        .into_iter()
        .map(|item| {
            Ok(PayloadItem {
                offer: wire_to_offer(item.offer)?,
                ratifier_data: item.ratifier_data.to_vec(),
            })
        })
        .collect()
}

fn offer_to_wire(offer: &Offer) -> WireOffer {
    WireOffer {
        market: WireMarket {
            chain_id: word_to_u256(&offer.market.chain_id),
            midnight: address_to_wire(&offer.market.midnight),
            loan_token: address_to_wire(&offer.market.loan_token),
            collateral_params: offer
                .market
                .collateral_params
                .iter()
                .map(|params| WireCollateralParams {
                    token: address_to_wire(&params.token),
                    lltv: word_to_u256(&params.lltv),
                    liquidation_cursor: word_to_u256(&params.liquidation_cursor),
                    oracle: address_to_wire(&params.oracle),
                })
                .collect(),
            maturity: word_to_u256(&offer.market.maturity),
            rcf_threshold: word_to_u256(&offer.market.rcf_threshold),
            enter_gate: address_to_wire(&offer.market.enter_gate),
            liquidator_gate: address_to_wire(&offer.market.liquidator_gate),
        },
        buy: offer.buy,
        maker: address_to_wire(&offer.maker),
        start: word_to_u256(&offer.start),
        expiry: word_to_u256(&offer.expiry),
        tick: word_to_u256(&offer.tick),
        group: FixedBytes::from(offer.group),
        callback: address_to_wire(&offer.callback),
        callback_data: Bytes::copy_from_slice(&offer.callback_data),
        receiver_if_maker_is_seller: address_to_wire(&offer.receiver_if_maker_is_seller),
        ratifier: address_to_wire(&offer.ratifier),
        reduce_only: offer.reduce_only,
        max_units: U256::from(offer.max_units),
        max_assets: U256::from(offer.max_assets),
        continuous_fee_cap: word_to_u256(&offer.continuous_fee_cap),
    }
}

fn wire_to_offer(offer: WireOffer) -> Result<Offer, PayloadError> {
    let max_units: u128 = offer
        .max_units
        .try_into()
        .map_err(|_| PayloadError::OfferCapOverflow)?;
    let max_assets: u128 = offer
        .max_assets
        .try_into()
        .map_err(|_| PayloadError::OfferCapOverflow)?;
    Ok(Offer {
        market: Market {
            chain_id: offer.market.chain_id.to_be_bytes(),
            midnight: address_from_wire(&offer.market.midnight),
            loan_token: address_from_wire(&offer.market.loan_token),
            collateral_params: offer
                .market
                .collateral_params
                .into_iter()
                .map(|params| CollateralParams {
                    token: address_from_wire(&params.token),
                    lltv: params.lltv.to_be_bytes(),
                    liquidation_cursor: params.liquidation_cursor.to_be_bytes(),
                    oracle: address_from_wire(&params.oracle),
                })
                .collect(),
            maturity: offer.market.maturity.to_be_bytes(),
            rcf_threshold: offer.market.rcf_threshold.to_be_bytes(),
            enter_gate: address_from_wire(&offer.market.enter_gate),
            liquidator_gate: address_from_wire(&offer.market.liquidator_gate),
        },
        buy: offer.buy,
        maker: address_from_wire(&offer.maker),
        start: offer.start.to_be_bytes(),
        expiry: offer.expiry.to_be_bytes(),
        tick: offer.tick.to_be_bytes(),
        group: offer.group.0,
        callback: address_from_wire(&offer.callback),
        callback_data: offer.callback_data.to_vec(),
        receiver_if_maker_is_seller: address_from_wire(&offer.receiver_if_maker_is_seller),
        ratifier: address_from_wire(&offer.ratifier),
        reduce_only: offer.reduce_only,
        max_units,
        max_assets,
        continuous_fee_cap: offer.continuous_fee_cap.to_be_bytes(),
    })
}

fn address_to_wire(address: &Address) -> SolAddress {
    SolAddress::from_slice(address)
}

fn address_from_wire(address: &SolAddress) -> Address {
    let mut result = [0u8; 20];
    result.copy_from_slice(address.as_slice());
    result
}

fn validate_payload_offer(offer: &Offer) -> Result<(), PayloadError> {
    if is_padding_offer(offer) {
        return Ok(());
    }
    if offer.market.collateral_params.is_empty() {
        return Err(PayloadError::InvalidOffer(
            "at least one collateral required",
        ));
    }
    if offer.market.collateral_params.len() > MAX_COLLATERALS {
        return Err(PayloadError::InvalidOffer("too many collaterals"));
    }

    let wad = U256::from(WAD);
    let mut previous_token: Option<Address> = None;
    for params in &offer.market.collateral_params {
        let lltv = word_to_u256(&params.lltv);
        let cursor = word_to_u256(&params.liquidation_cursor);
        if lltv > wad {
            return Err(PayloadError::InvalidOffer("collateral lltv exceeds WAD"));
        }
        if cursor >= wad {
            return Err(PayloadError::InvalidOffer(
                "collateral liquidation cursor must be below WAD",
            ));
        }
        let inner = cursor * (wad - lltv) / wad;
        let max_lif = wad * wad / (wad - inner);
        if max_lif > U256::from(2 * WAD) {
            return Err(PayloadError::InvalidOffer("computed max LIF exceeds 2 WAD"));
        }
        if lltv != wad && lltv * max_lif > U256::from(999_000_000_000_000_000u128 * WAD) {
            return Err(PayloadError::InvalidOffer(
                "computed max LIF is too high for LLTV",
            ));
        }
        if params.token == [0u8; 20]
            || previous_token.is_some_and(|previous| previous >= params.token)
        {
            return Err(PayloadError::InvalidOffer(
                "collaterals must be sorted and unique",
            ));
        }
        previous_token = Some(params.token);
    }

    for timestamp in [offer.market.maturity, offer.start, offer.expiry] {
        if word_to_u256(&timestamp) > U256::from(MAX_TIMESTAMP_SECONDS) {
            return Err(PayloadError::InvalidOffer("timestamp exceeds safe range"));
        }
    }
    let maturity = word_to_u256(&offer.market.maturity).to::<u64>();
    if maturity % 86_400 != 15 * 3_600 {
        return Err(PayloadError::InvalidOffer(
            "maturity must be at 15:00:00 UTC",
        ));
    }
    if offer.start > offer.expiry {
        return Err(PayloadError::InvalidOffer("start must not exceed expiry"));
    }
    if offer.expiry > offer.market.maturity {
        return Err(PayloadError::InvalidOffer(
            "expiry must not exceed maturity",
        ));
    }
    if word_to_u256(&offer.tick) > U256::from(MAX_TICK) {
        return Err(PayloadError::InvalidOffer("tick exceeds protocol maximum"));
    }
    if (offer.max_units == 0) == (offer.max_assets == 0) {
        return Err(PayloadError::InvalidOffer(
            "exactly one offer cap must be non-zero",
        ));
    }
    if offer.buy && offer.receiver_if_maker_is_seller != [0u8; 20] {
        return Err(PayloadError::InvalidOffer(
            "buy offer maker-seller receiver must be zero",
        ));
    }
    Ok(())
}

fn is_padding_offer(offer: &Offer) -> bool {
    offer.market.chain_id == [0u8; 32]
        && offer.market.midnight == [0u8; 20]
        && offer.market.loan_token == [0u8; 20]
        && offer.market.collateral_params.is_empty()
        && offer.market.maturity == [0u8; 32]
        && offer.market.rcf_threshold == [0u8; 32]
        && offer.market.enter_gate == [0u8; 20]
        && offer.market.liquidator_gate == [0u8; 20]
        && !offer.buy
        && offer.maker == [0u8; 20]
        && offer.start == [0u8; 32]
        && offer.expiry == [0u8; 32]
        && offer.tick == [0u8; 32]
        && offer.group == [0u8; 32]
        && offer.callback == [0u8; 20]
        && offer.callback_data.is_empty()
        && offer.receiver_if_maker_is_seller == [0u8; 20]
        && offer.ratifier == [0u8; 20]
        && !offer.reduce_only
        && offer.max_units == 0
        && offer.max_assets == 0
        && offer.continuous_fee_cap == [0u8; 32]
}
