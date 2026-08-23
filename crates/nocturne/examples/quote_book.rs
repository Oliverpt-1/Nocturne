//! Maker quoting path: build a ladder of APR-priced lend offers, sign one tree, then construct a
//! cancel-and-replace re-quote when fair value moves.
//!
//!   cargo run -p nocturne-midnight --example quote_book

use nocturne::*;

const CHAIN_ID: u64 = 1;
const NOW: u64 = 1_700_000_000;
const MATURITY: u64 = 1_731_536_000; // NOW + 1 year
const RUNGS: u64 = 4;
const APR_STEP: f64 = 2.0; // % between rungs
const MAX_UNITS: u128 = 5_000_000;

fn market() -> Market {
    MarketBuilder::new(CHAIN_ID, [0x11; 20], [0x22; 20])
        .collateral(
            [0x33; 20],
            U256::from(770_000_000_000_000_000u64),
            U256::from(1u64),
            [0x44; 20],
        )
        .maturity(MATURITY)
        .build_checked()
        .expect("valid market")
}

/// Lend offers laddered up from `fair_apr`; the tree assigns one group per standalone rung.
fn ladder(maker: Address, ratifier: Address, fair_apr: f64) -> Vec<Offer> {
    (0..RUNGS)
        .map(|i| {
            OfferBuilder::new(market(), maker)
                .lend()
                .apr(fair_apr + APR_STEP * i as f64, NOW)
                .expiry(MATURITY)
                .ratifier(ratifier)
                .max_units(MAX_UNITS)
                .build_checked()
                .expect("valid rung")
        })
        .collect()
}

/// Hash the book into one tree and sign its root - one signature covers every offer.
fn sign_book(
    signer: &LocalSigner,
    offers: Vec<Offer>,
    chain_id: Word,
    ratifier: &Address,
) -> (Vec<Offer>, OfferTree, Sig) {
    let descriptor = OfferTree::from_entries(offers).unwrap();
    let offers = descriptor.offers;
    let tree = descriptor.tree;
    let digest = tree_digest(tree.root(), tree.height(), chain_id, ratifier);
    let sig = signer.sign_digest(&digest).unwrap();
    (offers, tree, sig)
}

fn main() {
    let signer = LocalSigner::from_bytes(&[0x42u8; 32]).unwrap();
    let maker = signer.address();
    let ratifier = [0xbb; 20];
    let chain_id = word_from_u64(CHAIN_ID);
    let ttm = MATURITY - NOW;

    // Quote the book at fair value 7%.
    let offers = ladder(maker, ratifier, 7.0);
    let (offers, tree, sig) = sign_book(&signer, offers, chain_id, &ratifier);
    for (i, offer) in offers.iter().enumerate() {
        let tick = word_to_u128(&offer.tick).unwrap() as u64;
        println!(
            "rung {i}: tick {tick:4}  realized APR {:.3}%",
            tick_to_apr(tick, ttm).unwrap()
        );
        // A taker lifts rung i by submitting (sig, root, i, proof(i)) - prove it would ratify.
        let proof = tree.proof(i).unwrap();
        assert!(verify(
            offer,
            &tree.root(),
            i,
            &proof,
            &sig,
            chain_id,
            &ratifier,
            &maker
        ));
    }
    println!("root 0x{}", hex(&tree.root()));

    // Fair value moves 50bp. Cancel the old root on-chain, then sign a fresh ladder.
    let cancel = encode_cancel_root_calldata(&maker, &tree.root());
    println!(
        "cancel-and-replace: cancelRoot calldata {} bytes",
        cancel.len()
    );

    let offers = ladder(maker, ratifier, 7.5);
    let (_offers, tree, _sig) = sign_book(&signer, offers, chain_id, &ratifier);
    println!("new root 0x{}", hex(&tree.root()));
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}
