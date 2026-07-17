//! E2E driver - uses ONLY the nocturne tools to produce everything a real, deployed
//! Midnight needs, so a shell/cast layer can submit it to anvil and check the results.
//! Not a unit test; invoked by e2e/run.sh. Addresses come from env (the live deploy); the
//! anvil default keys/params are baked.
//!
//!   cargo run --example e2e -- gen
//!   cargo run --example e2e -- decode-market <hex>
//!   cargo run --example e2e -- decode-position <hex>

use k256::ecdsa::SigningKey;
use nocturne::*;

// anvil defaults
const PK1: [u8; 32] = hexlit("59c6995e998f97a5a0044966f0945389dc9e86dae88c7a8412f4603b6b78690d"); // maker/lender
const ACCOUNT0: &str = "f39Fd6e51aad88F6F4ce6aB8827279cffFb92266"; // taker/seller
const CHAIN_ID: u64 = 31337;
const MATURITY: u64 = 4_000_000_000;
const EXPIRY: u64 = 4_000_000_000;
const TICK: u64 = 3372; // price 0.5 WAD
const UNITS: u128 = 1_000_000;
const GROUP: u64 = 1;
const LLTV: u64 = 770_000_000_000_000_000;
const CURSOR: u64 = 300_000_000_000_000_000;
// on-chain settlement-fee curve set by the deploy; ttm >= 360d so the flat cbp6 (0.005 WAD) applies
const CBPS: [u16; 7] = [14, 14, 98, 417, 1250, 2500, 5000];
const TARGET_SELLER_ASSETS: u128 = 99_000; // sizing: size a take to hit ~this many seller assets

const fn hexlit(s: &str) -> [u8; 32] {
    let b = s.as_bytes();
    let mut out = [0u8; 32];
    let mut i = 0;
    while i < 32 {
        out[i] = (hexval(b[2 * i]) << 4) | hexval(b[2 * i + 1]);
        i += 1;
    }
    out
}
const fn hexval(c: u8) -> u8 {
    match c {
        b'0'..=b'9' => c - b'0',
        b'a'..=b'f' => c - b'a' + 10,
        b'A'..=b'F' => c - b'A' + 10,
        _ => 0,
    }
}

fn env_addr(k: &str) -> Address {
    let s = std::env::var(k).unwrap_or_else(|_| panic!("missing env {k}"));
    addr(s.trim_start_matches("0x"))
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

fn offer(maker: Address, ratifier: Address) -> Offer {
    OfferBuilder::new(market(), maker)
        .buy()
        .tick(TICK)
        .expiry(EXPIRY)
        .group_u64(GROUP)
        .ratifier(ratifier)
        .max_units(u128::MAX)
        .continuous_fee_cap(U256::MAX)
        .build()
}

fn gen() {
    let sk = SigningKey::from_bytes(&PK1.into()).unwrap();
    let maker = signer_address(&sk);
    let ratifier = env_addr("RATIFIER");
    let authorizer = env_addr("AUTHORIZER");
    let chain_id = word_from_u64(CHAIN_ID);

    let offer = offer(maker, ratifier);
    let tree = OfferTree::build(vec![hash_offer(&offer)]).unwrap();
    let root = tree.root();
    let proof = tree.proof(0);

    // offer signature (maker's key) + self-check it verifies
    let digest = tree_digest(root, tree.height(), chain_id, &ratifier);
    let sig = sign_digest(&sk, &digest);
    assert!(
        verify(&offer, &root, 0, &proof, &sig, chain_id, &ratifier, &maker),
        "verify failed"
    );

    // authorization to let the ratifier act for the maker (hot-key delegation tool)
    let auth = Authorization::new(maker, ratifier, true, U256::ZERO, U256::from(MATURITY));
    let auth_sig = sign_authorization(&sk, &auth, chain_id, &authorizer);

    // calldata
    let ratifier_data = encode_ratifier_data(&sig, &root, 0, &proof);
    let take = encode_take_calldata(
        &offer,
        &ratifier_data,
        U256::from(UNITS),
        &addr(ACCOUNT0), // taker
        &addr(ACCOUNT0), // receiverIfTakerIsSeller (buy offer -> taker is seller)
        &[0u8; 20],
        &[],
    );
    let cancel = encode_cancel_root_calldata(&maker, &root);

    // predictions (fresh positions, real non-zero settlement fee, ttm >= 360d -> flat cbp6)
    let ctx = SimCtx {
        now: 1,
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
    let out = simulate_take(&offer, U256::from(UNITS), &ctx).unwrap();

    // sizing: how many units to receive ~TARGET_SELLER_ASSETS, and what that take really yields
    let units2 = seller_assets_to_units(&offer, U256::from(TARGET_SELLER_ASSETS), 1, CBPS).unwrap();
    let pred2 = take_amounts(&offer, units2, 1, CBPS).unwrap();
    let take2 = encode_take_calldata(
        &offer,
        &ratifier_data,
        units2,
        &addr(ACCOUNT0),
        &addr(ACCOUNT0),
        &[0u8; 20],
        &[],
    );

    // validate: a tick not aligned to spacing (5 % 4 != 0) must be flagged AND revert on-chain
    let bad = OfferBuilder::new(market(), maker)
        .buy()
        .tick(5)
        .expiry(EXPIRY)
        .group_u64(GROUP + 1)
        .ratifier(ratifier)
        .max_units(u128::MAX)
        .continuous_fee_cap(U256::MAX)
        .build();
    let bad_ctx = ValidateCtx {
        market: Some(MarketSnapshot {
            tick_spacing: DEFAULT_TICK_SPACING,
            loss_factor_maxed: false,
            continuous_fee: 0,
        }),
        ..Default::default()
    };
    let bad_flagged = validate_offer(&bad, &bad_ctx).contains(&OfferError::TickNotAccessible);
    let bad_root = OfferTree::build(vec![hash_offer(&bad)]).unwrap().root();
    let bad_rd = encode_ratifier_data(
        &sign_digest(&sk, &tree_digest(bad_root, 0, chain_id, &ratifier)),
        &bad_root,
        0,
        &[],
    );
    let bad_take = encode_take_calldata(
        &bad,
        &bad_rd,
        U256::from(UNITS),
        &addr(ACCOUNT0),
        &addr(ACCOUNT0),
        &[0u8; 20],
        &[],
    );

    println!("MAKER {}", hx(&maker));
    println!("ROOT {}", hx(&root));
    println!("TAKE_CALLDATA {}", hx(&take));
    println!("CANCEL_CALLDATA {}", hx(&cancel));
    // authorization tuple + our signature (submitted via cast)
    println!("AUTH_AUTHORIZER {}", hx(&maker));
    println!("AUTH_AUTHORIZED {}", hx(&ratifier));
    println!("AUTH_NONCE 0");
    println!("AUTH_DEADLINE {MATURITY}");
    println!("AUTH_V {}", auth_sig.v);
    println!("AUTH_R {}", hx(&auth_sig.r));
    println!("AUTH_S {}", hx(&auth_sig.s));
    // predictions the chain must match
    println!("PRED_BUYER_ASSETS {}", out.amounts.buyer_assets);
    println!("PRED_SELLER_ASSETS {}", out.amounts.seller_assets);
    println!("PRED_BUYER_CREDIT_INCREASE {}", out.buyer_credit_increase);
    println!("PRED_SELLER_DEBT_INCREASE {}", out.seller_debt_increase);
    println!("PRED_NEW_CONSUMED {}", out.new_consumed);
    // sizing: take2 sized to target
    println!("TAKE2_CALLDATA {}", hx(&take2));
    println!("SIZING_UNITS {units2}");
    println!("PRED_SELLER_ASSETS2 {}", pred2.seller_assets);
    // validate: bad-tick offer
    println!("BAD_TAKE_CALLDATA {}", hx(&bad_take));
    println!("VALIDATE_FLAGGED_TICK {bad_flagged}");
}

fn read_hex_arg(a: &str) -> Vec<u8> {
    let a = a.trim().trim_start_matches("0x");
    (0..a.len() / 2)
        .map(|i| u8::from_str_radix(&a[2 * i..2 * i + 2], 16).unwrap())
        .collect()
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    match args.get(1).map(String::as_str) {
        Some("gen") => gen(),
        Some("decode-market") => {
            let m = decode_market_state(&read_hex_arg(&args[2])).unwrap();
            println!("TICK_SPACING {}", m.tick_spacing);
            println!("CONTINUOUS_FEE {}", m.continuous_fee);
            println!("TOTAL_UNITS {}", m.total_units);
        }
        Some("decode-position") => {
            let p = decode_position(&read_hex_arg(&args[2])).unwrap();
            println!("CREDIT {}", p.credit);
            println!("DEBT {}", p.debt);
        }
        _ => panic!("usage: e2e <gen|decode-market|decode-position> [hex]"),
    }
}
