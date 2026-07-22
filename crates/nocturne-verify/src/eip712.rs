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
