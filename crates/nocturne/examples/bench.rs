//! Benchmarks the maker re-quote pipeline: hash N offer leaves -> build tree -> all proofs -> sign root.
//! This is the loop an MM repeats every time the market moves. Prints machine-readable lines.

use k256::ecdsa::SigningKey;
use nocturne::*;
use rayon::prelude::*;
use std::time::Instant;

fn u256(x: u64) -> Word {
    let mut w = [0u8; 32];
    w[24..].copy_from_slice(&x.to_be_bytes());
    w
}

fn sample_offer(i: u64) -> Offer {
    let market = Market {
        chain_id: u256(1),
        midnight: [0x11; 20],
        loan_token: [0x22; 20],
        collateral_params: vec![CollateralParams {
            token: [0x33; 20],
            lltv: u256(860_000_000_000_000_000),
            liquidation_cursor: u256(1),
            oracle: [0x44; 20],
        }],
        maturity: u256(1_800_000_000),
        rcf_threshold: u256(1000),
        enter_gate: [0u8; 20],
        liquidator_gate: [0u8; 20],
    };
    Offer {
        market,
        buy: i % 2 == 0,
        maker: [0x55; 20],
        start: u256(0),
        expiry: u256(2_000_000_000),
        tick: u256(i % 6744), // spread quotes across the tick grid
        group: u256(i),
        callback: [0u8; 20],
        callback_data: Vec::new(),
        receiver_if_maker_is_seller: [0u8; 20],
        ratifier: [0xbb; 20],
        reduce_only: false,
        max_units: 1_000_000u128 + i as u128,
        max_assets: 0,
        continuous_fee_cap: u256(0),
    }
}

fn pipeline(offers: &[Offer], sk: &SigningKey, parallel: bool) -> (Word, u128) {
    let ratifier = [0xbbu8; 20];
    let chain_id = u256(1);
    let t = Instant::now();

    let leaves: Vec<Word> = if parallel {
        offers.par_iter().map(hash_offer).collect()
    } else {
        offers.iter().map(hash_offer).collect()
    };
    let tree = OfferTree::build(leaves).unwrap();
    // generate a proof for every leaf (takers need these to lift individual offers)
    let _proofs: Vec<Vec<Word>> = if parallel {
        (0..offers.len())
            .into_par_iter()
            .map(|i| tree.proof(i))
            .collect()
    } else {
        (0..offers.len()).map(|i| tree.proof(i)).collect()
    };
    let digest = tree_digest(tree.root(), tree.height(), chain_id, &ratifier);
    let _sig = sign_digest(sk, &digest);

    (tree.root(), t.elapsed().as_micros())
}

fn main() {
    let sk = SigningKey::from_bytes(&[0x42u8; 32].into()).unwrap();

    // Cross-check root for a fixed small tree so the JS baseline can assert byte-parity.
    let small: Vec<Offer> = (0..4).map(sample_offer).collect();
    let (root, _) = pipeline(&small, &sk, false);
    println!("CROSSCHECK_ROOT_N4 0x{}", hex(&root));

    for &n in &[1024usize, 4096, 16384] {
        let offers: Vec<Offer> = (0..n as u64).map(sample_offer).collect();
        // warmup
        pipeline(&offers, &sk, true);

        let reps = 20;
        let mut st = u128::MAX;
        let mut pt = u128::MAX;
        for _ in 0..reps {
            st = st.min(pipeline(&offers, &sk, false).1);
            pt = pt.min(pipeline(&offers, &sk, true).1);
        }
        println!(
            "RESULT n={n} single_us={st} parallel_us={pt} single_per_offer_ns={:.0} parallel_per_offer_ns={:.0}",
            st as f64 * 1000.0 / n as f64,
            pt as f64 * 1000.0 / n as f64
        );
    }
}

fn hex(w: &Word) -> String {
    w.iter().map(|b| format!("{b:02x}")).collect()
}
