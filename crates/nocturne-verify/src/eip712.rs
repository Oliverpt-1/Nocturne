//! Emit the EIP-712 typed data (`{domain, types, primaryType, message}`) whose hash equals the
//! digest `tree_digest` produces.
//!
//! A wallet that signs via `eth_signTypedData_v4` displays this structure; emitting it lets an
//! operator diff the tool's reconstruction, field by field, against what the wallet shows. The
//! `types` are parsed directly from the library's EIP-712 type-string constants (`OFFER_TYPE`,
//! `MARKET_TYPE`, `COLLATERAL_PARAMS_TYPE`, `EIP712_DOMAIN_TYPE`), so they cannot drift from what
//! the signature actually commits to. The `message` mirrors the offer fields in the same order.

use nocturne::*;
use serde_json::{json, Value};

use crate::render::{checksum, hex_bytes};

/// Parse an EIP-712 type string `Name(t1 n1,t2 n2,...)` into `(Name, [{name,type}, ...])`.
fn parse_type(s: &str) -> (String, Vec<Value>) {
    let open = s.find('(').expect("type string has '('");
    let name = s[..open].to_string();
    let inner = &s[open + 1..s.rfind(')').expect("type string has ')'")];
    let fields = if inner.is_empty() {
        Vec::new()
    } else {
        inner
            .split(',')
            .map(|field| {
                let (ty, nm) = field.split_once(' ').expect("field is 'type name'");
                json!({ "name": nm, "type": ty })
            })
            .collect()
    };
    (name, fields)
}

fn cp_message(cp: &CollateralParams) -> Value {
    json!({
        "token": checksum(&cp.token),
        "lltv": word_to_u256(&cp.lltv).to_string(),
        "liquidationCursor": word_to_u256(&cp.liquidation_cursor).to_string(),
        "oracle": checksum(&cp.oracle),
    })
}

fn market_message(m: &Market) -> Value {
    json!({
        "chainId": word_to_u256(&m.chain_id).to_string(),
        "midnight": checksum(&m.midnight),
        "loanToken": checksum(&m.loan_token),
        "collateralParams": m.collateral_params.iter().map(cp_message).collect::<Vec<_>>(),
        "maturity": word_to_u256(&m.maturity).to_string(),
        "rcfThreshold": word_to_u256(&m.rcf_threshold).to_string(),
        "enterGate": checksum(&m.enter_gate),
        "liquidatorGate": checksum(&m.liquidator_gate),
    })
}

fn offer_message(o: &Offer) -> Value {
    json!({
        "market": market_message(&o.market),
        "buy": o.buy,
        "maker": checksum(&o.maker),
        "start": word_to_u256(&o.start).to_string(),
        "expiry": word_to_u256(&o.expiry).to_string(),
        "tick": word_to_u256(&o.tick).to_string(),
        "group": hex_bytes(&o.group),
        "callback": checksum(&o.callback),
        "callbackData": hex_bytes(&o.callback_data),
        "receiverIfMakerIsSeller": checksum(&o.receiver_if_maker_is_seller),
        "ratifier": checksum(&o.ratifier),
        "reduceOnly": o.reduce_only,
        "maxUnits": o.max_units.to_string(),
        "maxAssets": o.max_assets.to_string(),
        "continuousFeeCap": word_to_u256(&o.continuous_fee_cap).to_string(),
    })
}

/// The `offerTree` field type for a tree of the given height: `Offer`, `Offer[2]`, `Offer[2][2]`...
fn offer_tree_field_type(height: usize) -> String {
    let mut t = String::from("Offer");
    for _ in 0..height {
        t.push_str("[2]");
    }
    t
}

/// Build the full EIP-712 typed-data document for a tree over `offers` (whose length must be
/// `2^height`), bound to `chain_id` and `ratifier` (the verifying contract).
pub fn typed_data(offers: &[Offer], chain_id: Word, ratifier: &Address, height: usize) -> Value {
    let (_, offer_fields) = parse_type(OFFER_TYPE);
    let (_, market_fields) = parse_type(MARKET_TYPE);
    let (_, cp_fields) = parse_type(COLLATERAL_PARAMS_TYPE);
    let (_, domain_fields) = parse_type(EIP712_DOMAIN_TYPE);

    // Fold the flat offer list into the nested Offer[2]^height array the type describes.
    let mut nodes: Vec<Value> = offers.iter().map(offer_message).collect();
    for _ in 0..height {
        nodes = nodes.chunks(2).map(|c| json!([c[0], c[1]])).collect();
    }
    let offer_tree = nodes.into_iter().next().expect("non-empty tree");

    json!({
        "primaryType": "OfferTree",
        "domain": {
            "chainId": word_to_u256(&chain_id).to_string(),
            "verifyingContract": checksum(ratifier),
        },
        "types": {
            "EIP712Domain": domain_fields,
            "OfferTree": [{ "name": "offerTree", "type": offer_tree_field_type(height) }],
            "Offer": offer_fields,
            "Market": market_fields,
            "CollateralParams": cp_fields,
        },
        "message": { "offerTree": offer_tree },
    })
}

// ---- parsing (the inverse of `typed_data`, for verifying wallet prompts) --------
//
// Declarative serde mirror of the message shape, strict on every level: unknown fields,
// missing fields, non-pair nesting, and out-of-range values are all errors. A field this
// parser ignored could still change what the wallet hashes, so nothing is ignored.

use serde::Deserialize;

/// A typed-data document parsed back into library types.
pub struct ParsedTypedData {
    /// The tree's leaves in order (including any zero-offer padding).
    pub offers: Vec<Offer>,
    /// The domain chain id.
    pub chain_id: Word,
    /// The domain verifying contract (the ratifier).
    pub ratifier: Address,
    /// The tree height (nesting depth of `offerTree`).
    pub height: usize,
}

/// A uint256 as typed data carries it: a decimal string.
struct Uint(Word);

impl<'de> Deserialize<'de> for Uint {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let s = String::deserialize(d)?;
        let u = U256::from_str_radix(s.trim(), 10).map_err(serde::de::Error::custom)?;
        Ok(Uint(u256_to_word(u)))
    }
}

/// A `0x`-prefixed fixed- or variable-length hex string.
struct Hex<const N: usize>([u8; N]);

impl<'de, const N: usize> Deserialize<'de> for Hex<N> {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let s = String::deserialize(d)?;
        let b = hex::decode(s.trim().trim_start_matches("0x")).map_err(serde::de::Error::custom)?;
        b.try_into().map(Hex).map_err(|b: Vec<u8>| {
            serde::de::Error::custom(format!("expected {N} bytes, got {}", b.len()))
        })
    }
}

struct HexBytes(Vec<u8>);

impl<'de> Deserialize<'de> for HexBytes {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let s = String::deserialize(d)?;
        hex::decode(s.trim().trim_start_matches("0x"))
            .map(HexBytes)
            .map_err(serde::de::Error::custom)
    }
}

/// A uint128 as typed data carries it: a decimal string that must fit 128 bits.
struct Uint128(u128);

impl<'de> Deserialize<'de> for Uint128 {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let s = String::deserialize(d)?;
        s.trim()
            .parse()
            .map(Uint128)
            .map_err(serde::de::Error::custom)
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct CpMsg {
    token: Hex<20>,
    lltv: Uint,
    liquidation_cursor: Uint,
    oracle: Hex<20>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct MarketMsg {
    chain_id: Uint,
    midnight: Hex<20>,
    loan_token: Hex<20>,
    collateral_params: Vec<CpMsg>,
    maturity: Uint,
    rcf_threshold: Uint,
    enter_gate: Hex<20>,
    liquidator_gate: Hex<20>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct OfferMsg {
    market: MarketMsg,
    buy: bool,
    maker: Hex<20>,
    start: Uint,
    expiry: Uint,
    tick: Uint,
    group: Hex<32>,
    callback: Hex<20>,
    callback_data: HexBytes,
    receiver_if_maker_is_seller: Hex<20>,
    ratifier: Hex<20>,
    reduce_only: bool,
    max_units: Uint128,
    max_assets: Uint128,
    continuous_fee_cap: Uint,
}

impl From<OfferMsg> for Offer {
    fn from(o: OfferMsg) -> Self {
        Offer {
            market: Market {
                chain_id: o.market.chain_id.0,
                midnight: o.market.midnight.0,
                loan_token: o.market.loan_token.0,
                collateral_params: o
                    .market
                    .collateral_params
                    .into_iter()
                    .map(|cp| CollateralParams {
                        token: cp.token.0,
                        lltv: cp.lltv.0,
                        liquidation_cursor: cp.liquidation_cursor.0,
                        oracle: cp.oracle.0,
                    })
                    .collect(),
                maturity: o.market.maturity.0,
                rcf_threshold: o.market.rcf_threshold.0,
                enter_gate: o.market.enter_gate.0,
                liquidator_gate: o.market.liquidator_gate.0,
            },
            buy: o.buy,
            maker: o.maker.0,
            start: o.start.0,
            expiry: o.expiry.0,
            tick: o.tick.0,
            group: o.group.0,
            callback: o.callback.0,
            callback_data: o.callback_data.0,
            receiver_if_maker_is_seller: o.receiver_if_maker_is_seller.0,
            ratifier: o.ratifier.0,
            reduce_only: o.reduce_only,
            max_units: o.max_units.0,
            max_assets: o.max_assets.0,
            continuous_fee_cap: o.continuous_fee_cap.0,
        }
    }
}

/// One node of the nested `Offer[2]^h` tree: either a pair or a leaf offer.
#[derive(Deserialize)]
#[serde(untagged)]
enum TreeNode {
    Pair(Box<[TreeNode; 2]>),
    Leaf(Box<OfferMsg>),
}

impl TreeNode {
    /// Flatten depth-first (the order `typed_data`'s fold produces), enforcing uniform depth.
    fn leaves(self, depth: usize, height: usize, out: &mut Vec<Offer>) -> Result<(), String> {
        match self {
            TreeNode::Leaf(o) if depth == height => {
                out.push((*o).into());
                Ok(())
            }
            TreeNode::Leaf(_) => Err(format!("leaf at depth {depth}, expected {height}")),
            TreeNode::Pair(_) if depth == height => {
                Err(format!("nesting deeper than height {height}"))
            }
            TreeNode::Pair(pair) => {
                let [l, r] = *pair;
                l.leaves(depth + 1, height, out)?;
                r.leaves(depth + 1, height, out)
            }
        }
    }

    fn depth(&self) -> usize {
        match self {
            TreeNode::Leaf(_) => 0,
            TreeNode::Pair(pair) => 1 + pair[0].depth(),
        }
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct MessageMsg {
    offer_tree: TreeNode,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct DomainMsg {
    chain_id: Uint,
    verifying_contract: Hex<20>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct TypedDataMsg {
    primary_type: String,
    domain: DomainMsg,
    /// Kept as raw JSON: the types table is checked by canonical regeneration
    /// ([`docs_equivalent`]), not field-by-field.
    #[allow(dead_code)]
    types: Value,
    message: MessageMsg,
}

/// Parse a full EIP-712 typed-data document (the shape [`typed_data`] emits, i.e. what an
/// `eth_signTypedData_v4` wallet displays) back into typed offers and domain values.
pub fn parse_typed_data(doc: &Value) -> Result<ParsedTypedData, String> {
    let td: TypedDataMsg = serde_json::from_value(doc.clone()).map_err(|e| e.to_string())?;
    if td.primary_type != "OfferTree" {
        return Err(format!(
            "primaryType: expected \"OfferTree\", got \"{}\"",
            td.primary_type
        ));
    }
    let height = td.message.offer_tree.depth();
    let mut offers = Vec::new();
    td.message
        .offer_tree
        .leaves(0, height, &mut offers)
        .map_err(|e| format!("message.offerTree: {e}"))?;
    Ok(ParsedTypedData {
        offers,
        chain_id: td.domain.chain_id.0,
        ratifier: td.domain.verifying_contract.0,
        height,
    })
}

/// Compare two typed-data documents up to hex-string case (address checksum casing does not
/// affect `eth_signTypedData_v4` hashing) and object key order.
pub fn docs_equivalent(a: &Value, b: &Value) -> bool {
    fn norm(v: &Value) -> Value {
        match v {
            Value::String(s) if s.starts_with("0x") => Value::String(s.to_lowercase()),
            Value::Array(items) => Value::Array(items.iter().map(norm).collect()),
            Value::Object(m) => {
                Value::Object(m.iter().map(|(k, v)| (k.clone(), norm(v))).collect())
            }
            other => other.clone(),
        }
    }
    norm(a) == norm(b)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Offer {
        Offer {
            market: Market {
                chain_id: word_from_u64(1),
                midnight: [0x11; 20],
                loan_token: [0x22; 20],
                collateral_params: vec![CollateralParams {
                    token: [0x33; 20],
                    lltv: word_from_u128(1),
                    liquidation_cursor: word_from_u128(2),
                    oracle: [0x44; 20],
                }],
                maturity: word_from_u64(100),
                rcf_threshold: word_from_u64(1),
                enter_gate: [0u8; 20],
                liquidator_gate: [0u8; 20],
            },
            buy: true,
            maker: [0x55; 20],
            start: word_from_u64(0),
            expiry: word_from_u64(200),
            tick: word_from_u64(8),
            group: word_from_u64(1),
            callback: [0u8; 20],
            callback_data: Vec::new(),
            receiver_if_maker_is_seller: [0u8; 20],
            ratifier: [0xbb; 20],
            reduce_only: false,
            max_units: 10,
            max_assets: 0,
            continuous_fee_cap: word_from_u64(0),
        }
    }

    #[test]
    fn parsed_types_match_library_constants() {
        // The parsed field arrays must reconstruct the exact library type strings.
        for s in [
            OFFER_TYPE,
            MARKET_TYPE,
            COLLATERAL_PARAMS_TYPE,
            EIP712_DOMAIN_TYPE,
        ] {
            let (name, fields) = parse_type(s);
            let inner: Vec<String> = fields
                .iter()
                .map(|f| {
                    format!(
                        "{} {}",
                        f["type"].as_str().unwrap(),
                        f["name"].as_str().unwrap()
                    )
                })
                .collect();
            assert_eq!(format!("{name}({})", inner.join(",")), s);
        }
    }

    #[test]
    fn single_offer_is_object_not_array() {
        let td = typed_data(&[sample()], word_from_u64(1), &[0xbb; 20], 0);
        assert_eq!(td["primaryType"], "OfferTree");
        assert_eq!(td["types"]["OfferTree"][0]["type"], "Offer");
        assert!(td["message"]["offerTree"].is_object());
        assert_eq!(td["domain"]["chainId"], "1");
    }

    #[test]
    fn two_offers_nest_as_array() {
        let td = typed_data(&[sample(), sample()], word_from_u64(1), &[0xbb; 20], 1);
        assert_eq!(td["types"]["OfferTree"][0]["type"], "Offer[2]");
        assert!(td["message"]["offerTree"].is_array());
        assert_eq!(td["message"]["offerTree"].as_array().unwrap().len(), 2);
    }
}
