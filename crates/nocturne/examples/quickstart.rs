//! First-offer path: build an offer, sign its tree, and verify the ratifier proof locally.
//!
//!   cargo run -p nocturne-midnight --example quickstart

use nocturne::*;

fn main() {
    let signer = LocalSigner::from_bytes(&[0x42u8; 32]).unwrap();
    let maker = signer.address();
    let ratifier = [0xbb; 20];
    let chain_id = word_from_u64(1);

    // build a market and a "lend at 7.2% APR" offer
    let market = MarketBuilder::new(1, [0x11; 20], [0x22; 20])
        .collateral(
            [0x33; 20],
            U256::from(770_000_000_000_000_000u64),
            U256::from(1u64),
            [0x44; 20],
        )
        .maturity(2_000_000_000)
        .build_checked()
        .expect("valid market");
    let offer = OfferBuilder::new(market, maker)
        .lend()
        .apr(7.2, 1_700_000_000)
        .expiry(2_000_000_000)
        .ratifier(ratifier)
        .max_units(1_000_000)
        .build_checked()
        .expect("valid offer");

    // assign the offer's content-addressed group, build its tree, and sign the root
    let descriptor = OfferTree::from_entries([offer]).unwrap();
    let offer = &descriptor.offers[0];
    let tree = descriptor.tree;
    let digest = tree_digest(tree.root(), tree.height(), chain_id, &ratifier);
    let sig = signer.sign_digest(&digest).unwrap();
    let ok = verify(
        offer,
        &tree.root(),
        0,
        &tree.proof(0).unwrap(),
        &sig,
        chain_id,
        &ratifier,
        &maker,
    );

    println!("maker        {}", to_hex(&maker));
    println!("tick         {}", word_to_u128(&offer.tick).unwrap());
    println!("root         {}", to_hex(&tree.root()));
    println!("verifies     {ok}");
    assert!(ok);
}

fn to_hex(b: &[u8]) -> String {
    let mut s = String::from("0x");
    for x in b {
        s.push_str(&format!("{x:02x}"));
    }
    s
}
