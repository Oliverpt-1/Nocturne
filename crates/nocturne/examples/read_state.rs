//! Read-only path: decode checked-in production calldata into typed views without an RPC call.
//!
//! The inputs are two real Base-mainnet transactions checked into the test fixtures: a taker's
//! bundle fill (`IMidnightBundles`) and the maker's `SetterRatifier.setIsRootRatified` whose
//! root covers the bundle's first fill.
//!
//!   cargo run -p nocturne-midnight --example read_state

use nocturne::*;

fn fixture(hex_str: &str) -> Vec<u8> {
    hex::decode(hex_str.trim().trim_start_matches("0x")).unwrap()
}

fn main() {
    // A production taker bundle: which fill function, who takes, and what fills it carries.
    let bundle = decode_bundle_calldata(&fixture(include_str!(
        "../tests/data/setter_bundle_full.hex"
    )))
    .unwrap();
    println!("call   {}", bundle.kind.function_name());
    println!("taker  0x{}", hex(&bundle.taker));
    println!("{} {}", bundle.kind.target_label(), bundle.target);
    for (i, fill) in bundle.fills.iter().enumerate() {
        let offer = &fill.offer;
        let tick = word_to_u128(&offer.tick).unwrap() as u64;
        println!(
            "fill {i}: {} {} units at tick {tick} (price {} WAD), root 0x{}…",
            if offer.buy {
                "maker buys"
            } else {
                "maker sells"
            },
            fill.units,
            tick_to_price(tick).unwrap(),
            hex(&fill.ratifier_data.root()[..4]),
        );
    }

    // The maker's ratification of that root.
    let ratify = decode_set_is_root_ratified_calldata(&fixture(include_str!(
        "../tests/data/setter_ratify_prod.hex"
    )))
    .unwrap();
    assert!(ratify.ratified);
    println!(
        "maker 0x{} ratified root 0x{}",
        hex(&ratify.maker),
        hex(&ratify.root)
    );

    // The two transactions link up: the ratified root is exactly the root fill[0] claims.
    assert_eq!(*bundle.fills[0].ratifier_data.root(), ratify.root);
    println!("fill[0] root matches the ratified root");
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}
