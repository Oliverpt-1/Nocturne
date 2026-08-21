//! Take an offer: validate it, size the take in notional terms, simulate the exact fill, and
//! encode the `take` calldata a taker would submit.
//!
//!   cargo run --example take_offer

use nocturne::*;

const CHAIN_ID: u64 = 1;
const MIDNIGHT: Address = [0x11; 20];
const NOW: u64 = 1_700_000_000;
const MATURITY: u64 = 1_731_536_000; // NOW + 1 year
const CBPS: [u16; 7] = [14, 14, 98, 417, 1250, 2500, 5000]; // settlementFeeCbp0..6
const TAKER: Address = [0x77; 20];

fn main() {
    // The maker's side, condensed (see quickstart / quote_book): a signed "lend at 7.2% APR"
    // offer in a one-leaf tree.
    let signer = LocalSigner::from_bytes(&[0x42u8; 32]).unwrap();
    let maker = signer.address();
    let ratifier = [0xbb; 20];
    let chain_id = word_from_u64(CHAIN_ID);
    let market = MarketBuilder::new(CHAIN_ID, MIDNIGHT, [0x22; 20])
        .collateral(
            [0x33; 20],
            U256::from(770_000_000_000_000_000u64),
            U256::from(1u64),
            [0x44; 20],
        )
        .maturity(MATURITY)
        .build_checked()
        .expect("valid market");
    let offer = OfferBuilder::new(market, maker)
        .lend()
        .apr(7.2, NOW)
        .expiry(MATURITY)
        .ratifier(ratifier)
        .max_units(5_000_000)
        .build_checked()
        .unwrap();
    let descriptor = OfferTree::from_entries([offer]).unwrap();
    let offer = &descriptor.offers[0];
    let tree = descriptor.tree;
    let digest = tree_digest(tree.root(), tree.height(), chain_id, &ratifier);
    let sig = signer.sign_digest(&digest).unwrap();

    // ---- the taker's side ----

    // 1. Validate: would `take` accept this offer at all?
    let ctx = ValidateCtx {
        chain_id: Some(CHAIN_ID),
        midnight: Some(MIDNIGHT),
        now: Some(NOW),
        market: Some(MarketSnapshot {
            tick_spacing: DEFAULT_TICK_SPACING,
            loss_factor_maxed: false,
            continuous_fee: 0,
        }),
    };
    let problems = validate_offer(offer, &ctx);
    assert!(problems.is_empty(), "offer rejected: {problems:?}");

    // 2. Size by notional: how many units must the taker (the seller here - the maker is
    //    lending, i.e. buying credit) lift to receive 500_000 loan-token assets?
    let units = seller_assets_to_units(offer, U256::from(500_000u64), NOW, CBPS).unwrap();

    // 3. Simulate the exact fill: amounts moved, position deltas, and any revert reasons.
    let sim = SimCtx {
        now: NOW,
        market: SimMarket {
            tick_spacing: DEFAULT_TICK_SPACING,
            continuous_fee: 0,
            settlement_fee_cbp: CBPS,
            loss_factor_maxed: false,
        },
        consumed: 0,
        maker_position: Position::default(),
        taker_position: Position::default(),
        taker_is_maker: false,
    };
    let outcome = simulate_take(offer, units, &sim).unwrap();
    assert!(
        outcome.reverts.is_empty(),
        "take would revert: {:?}",
        outcome.reverts
    );
    println!("units           {units}");
    println!("buyer pays      {}", outcome.amounts.buyer_assets);
    println!("seller receives {}", outcome.amounts.seller_assets);
    println!("settlement fee  {}", outcome.amounts.settlement_fee_assets);
    println!("seller debt +   {}", outcome.seller_debt_increase);

    // 4. Encode the transaction: the signature and Merkle proof travel as ratifier data inside
    //    the `take` calldata.
    let rd = encode_ratifier_data(&sig, &tree.root(), 0, &tree.proof(0).unwrap());
    let calldata = encode_take_calldata(offer, &rd, units, &TAKER, &TAKER, &[0u8; 20], &[]);

    // Round-trip: the bytes decode back to exactly what we meant to send.
    let call = decode_take_calldata(&calldata).unwrap();
    assert_eq!(&call.offer, offer);
    assert_eq!(call.units, units);
    assert_eq!(call.taker, TAKER);
    println!("take calldata   {} bytes", calldata.len());
}
