//! Market-making loop driver - the SDK quotes a *grid of lend offers by APR* into one signed tree
//! and emits, per rung, the resolved tick, the realized APR, the take calldata, and predicted
//! fills. The shell (e2e/mm.sh) drives rounds against a real Midnight on anvil: quote a grid by
//! APR, take full + partial, move fair value, re-quote, cancel-and-replace, and check inventory -
//! every fill vs the SDK's take_amounts predictions, proving APR -> tick -> price -> on-chain fill.
//!
//!   cargo run --example mm_loop -- grid <fair_apr> <group_base>

use k256::ecdsa::SigningKey;
use nocturne::*;

const PK1: [u8; 32] = hexlit("59c6995e998f97a5a0044966f0945389dc9e86dae88c7a8412f4603b6b78690d"); // maker
const ACCOUNT0: &str = "f39Fd6e51aad88F6F4ce6aB8827279cffFb92266"; // taker
const CHAIN_ID: u64 = 31337;
const NOW: u64 = 1_000_000_000; // anvil genesis timestamp (mm.sh starts anvil with --timestamp)
const MATURITY: u64 = 1_031_536_000; // NOW + 1 year
const EXPIRY: u64 = 1_031_000_000;
const LLTV: u64 = 770_000_000_000_000_000;
const CURSOR: u64 = 300_000_000_000_000_000;
const CBPS: [u16; 7] = [14, 14, 98, 417, 1250, 2500, 5000];
const GRID: u64 = 4; // 4 rungs -> height-2 tree
const APR_STEP: f64 = 4.0; // % between rungs (wide enough for distinct ticks near par)
const MAX_UNITS: u128 = 5_000_000;
const FULL: u128 = 1_000_000;
const PARTIAL: u128 = 400_000;

const fn hexlit(s: &str) -> [u8; 32] {
    let b = s.as_bytes();
    let mut out = [0u8; 32];
    let mut i = 0;
    while i < 32 {
        out[i] = (hv(b[2 * i]) << 4) | hv(b[2 * i + 1]);
        i += 1;
    }
    out
}
const fn hv(c: u8) -> u8 {
    match c {
        b'0'..=b'9' => c - b'0',
        b'a'..=b'f' => c - b'a' + 10,
        _ => 0,
    }
}

fn env_addr(k: &str) -> Address {
    addr(std::env::var(k).unwrap().trim_start_matches("0x"))
}
fn addr(s: &str) -> Address {
    let mut a = [0u8; 20];
    for i in 0..20 {
        a[i] = u8::from_str_radix(&s[2 * i..2 * i + 2], 16).unwrap();
    }
    a
}
fn hx(b: &[u8]) -> String {
    let mut s = String::from("0x");
    for x in b {
        s.push_str(&format!("{x:02x}"));
    }
    s
}

fn market() -> Market {
    MarketBuilder::new(CHAIN_ID, env_addr("MIDNIGHT"), env_addr("LOAN"))
        .collateral(
            env_addr("COLLATERAL"),
            U256::from(LLTV),
            U256::from(CURSOR),
            env_addr("ORACLE"),
        )
        .maturity(MATURITY)
        .build()
}

fn rung(maker: Address, ratifier: Address, apr: f64, group: u64) -> Offer {
    OfferBuilder::new(market(), maker)
        .lend() // maker lends: buys credit
        .apr(apr, NOW) // APR -> tick, snapped to the accessible grid
        .expiry(EXPIRY)
        .group_u64(group)
        .ratifier(ratifier)
        .max_units(MAX_UNITS)
        .continuous_fee_cap(U256::MAX)
        .build_checked()
        .expect("apr build")
}

fn main() {
    let a: Vec<String> = std::env::args().collect();
    assert_eq!(a[1], "grid", "usage: mm_loop grid <fair_apr> <group_base>");
    let fair_apr: f64 = a[2].parse().unwrap();
    let group_base: u64 = a[3].parse().unwrap();

    let sk = SigningKey::from_bytes(&PK1.into()).unwrap();
    let maker = signer_address(&sk);
    let ratifier = env_addr("RATIFIER");
    let chain_id = word_from_u64(CHAIN_ID);
    let ttm = MATURITY - NOW;

    // grid: rungs at fair_apr, fair_apr+STEP, ... (a rate ladder), quoted BY APR
    let aprs: Vec<f64> = (0..GRID).map(|i| fair_apr + APR_STEP * i as f64).collect();
    let offers: Vec<Offer> = aprs
        .iter()
        .enumerate()
        .map(|(i, &apr)| rung(maker, ratifier, apr, group_base + i as u64))
        .collect();
    let tree = OfferTree::build(offers.iter().map(hash_offer).collect()).unwrap();
    let root = tree.root();
    let sig = sign_digest(&sk, &tree_digest(root, tree.height(), chain_id, &ratifier));

    println!("MAKER {}", hx(&maker));
    println!("ROOT {}", hx(&root));
    println!("CANCEL {}", hx(&encode_cancel_root_calldata(&maker, &root)));

    let take = |o: &Offer, i: usize, units: u128| {
        let rd = encode_ratifier_data(&sig, &root, i, &tree.proof(i));
        encode_take_calldata(
            o,
            &rd,
            U256::from(units),
            &addr(ACCOUNT0),
            &addr(ACCOUNT0),
            &[0u8; 20],
            &[],
        )
    };
    for (i, o) in offers.iter().enumerate() {
        let tick = word_to_u128(&o.tick).unwrap() as u64;
        let full = take_amounts(o, U256::from(FULL), NOW, CBPS).unwrap();
        let part = take_amounts(o, U256::from(PARTIAL), NOW, CBPS).unwrap();
        println!("R{i}_APR {:.2}", aprs[i]);
        println!("R{i}_TICK {tick}");
        println!("R{i}_REALIZED_APR {:.4}", tick_to_apr(tick, ttm).unwrap());
        println!("R{i}_GROUP {}", group_base + i as u64);
        println!("R{i}_TAKE_FULL {}", hx(&take(o, i, FULL)));
        println!("R{i}_TAKE_PARTIAL {}", hx(&take(o, i, PARTIAL)));
        println!("R{i}_FULL_SELLER_ASSETS {}", full.seller_assets);
        println!("R{i}_PARTIAL_SELLER_ASSETS {}", part.seller_assets);
    }
    println!("FULL {FULL}");
    println!("PARTIAL {PARTIAL}");
}
