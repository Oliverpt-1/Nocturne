//! Parity for `tick_to_price` against the real `TickLib.tickToPrice`.
//!
//! The (tick, price) vectors below are printed by `fixtures/GenSim.t.sol`, run against the
//! Midnight contracts at rev f47568c9e45a9b70830b82a130b47393dcafec33. See that file to
//! regenerate. Baked so `cargo test` stays standalone.

use nocturne::*;

// (tick, price) exactly as TickLib.tickToPrice emits.
const VECTORS: &[(u64, &str)] = &[
    (0, "0"),
    (1, "0"),
    (4, "100000000000"),
    (100, "100000000000"),
    (1000, "7300000000000"),
    (3371, "498753100000000000"),
    (3372, "500000000000000000"),
    (3373, "501246900000000000"),
    (5000, "999702500000000000"),
    (6740, "999999900000000000"),
    (6743, "1000000000000000000"),
    (6744, "1000000000000000000"),
];

#[test]
fn tick_to_price_matches_ticklib() {
    for &(tick, expected) in VECTORS {
        let got = tick_to_price(tick).expect("tick in range");
        let want = U256::from_str_radix(expected, 10).unwrap();
        assert_eq!(got, want, "tick {tick}: got {got}, want {want}");
    }
}

#[test]
fn tick_to_price_is_monotonic_nondecreasing() {
    // TickLib is designed so price is non-decreasing in tick.
    let mut prev = U256::ZERO;
    for tick in 0..=MAX_TICK {
        let p = tick_to_price(tick).unwrap();
        assert!(p >= prev, "price decreased at tick {tick}");
        prev = p;
    }
    // Ends at 1 WAD.
    assert_eq!(tick_to_price(MAX_TICK).unwrap(), U256::from(1_000_000_000_000_000_000u64));
}

#[test]
fn tick_to_price_rejects_out_of_range() {
    assert_eq!(tick_to_price(MAX_TICK + 1), Err(SimError::TickOutOfRange((MAX_TICK + 1) as u128)));
}
