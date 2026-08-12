//! Generate a realistic, fully-signed sample payload for exercising the `nocturne-verify` CLI.
//!
//! Prints the `take` calldata, ratifier data, the signed digest, and the maker to stdout, and
//! writes the offer as JSON to `sample-offer.json` (or the path given as the first argument) so it
//! can be fed to `nocturne-verify digest`. Dummy key and addresses; nothing touches a chain.
//!
//!   cargo run -p nocturne-verify --example sample

use nocturne::*;
use std::io::Write;

fn main() {
    let signer = LocalSigner::from_bytes(&[0x42; 32]).unwrap();
    let chain_id = word_from_u64(31337);
    let offer = Offer {
        market: Market {
            chain_id,
            midnight: [0x11; 20],
            loan_token: [0x22; 20],
            collateral_params: vec![CollateralParams {
                token: [0x33; 20],
                lltv: word_from_u128(770_000_000_000_000_000),
                liquidation_cursor: word_from_u128(300_000_000_000_000_000),
                oracle: [0x44; 20],
            }],
            maturity: word_from_u64(4_000_000_000),
            rcf_threshold: word_from_u64(1000),
            enter_gate: [0u8; 20],
            liquidator_gate: [0u8; 20],
        },
        buy: true,
        maker: signer.address(),
        start: word_from_u64(1_600_000_000),
        expiry: word_from_u64(4_000_000_000),
        tick: word_from_u64(3372),
        group: word_from_u64(1),
        callback: [0u8; 20],
        callback_data: vec![],
        receiver_if_maker_is_seller: [0u8; 20],
        ratifier: [0xbb; 20],
        reduce_only: false,
        max_units: 1_000_000,
        max_assets: 0,
        continuous_fee_cap: word_from_u64(0),
    };

    let tree = OfferTree::build(vec![hash_offer(&offer)]).unwrap();
    let digest = tree_digest(tree.root(), tree.height(), chain_id, &offer.ratifier);
    let sig = signer.sign_digest(&digest).unwrap();
    let ratifier_data = encode_ratifier_data(&sig, &tree.root(), 0, &tree.proof(0));
    let take = encode_take_calldata(
        &offer,
        &ratifier_data,
        U256::from(250_000u64),
        &[0x77; 20],
        &[0x88; 20],
        &[0u8; 20],
        &[],
    );

    let path = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "sample-offer.json".to_string());
    let mut f = std::fs::File::create(&path).unwrap();
    f.write_all(serde_json::to_string_pretty(&offer).unwrap().as_bytes())
        .unwrap();

    println!("# maker  : 0x{}", hex::encode(signer.address()));
    println!("# digest : 0x{}", hex::encode(digest));
    println!("# offer JSON written to: {path}");
    println!("# take calldata:");
    println!("0x{}", hex::encode(&take));
    eprintln!("\n# ratifier data (for `decode --type ratifier`):");
    eprintln!("0x{}", hex::encode(&ratifier_data));
}
