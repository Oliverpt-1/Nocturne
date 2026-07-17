//! Fuzz / property suite for `nocturne`.
//!
//! Two parts:
//!  * **Part A — proptest property tests** (pure Rust): Merkle proof soundness, the full
//!    sign → recover → verify round-trip, `tick_to_price` range + monotonicity, and a
//!    "never panics on adversarial input" robustness sweep over `validate_offer` /
//!    `simulate_take` / `take_amounts`.
//!  * **Part B — differential parity vs the real contracts**: 32 deterministically generated
//!    offers + ticks whose `HashLib.hashOffer` / `TickLib.tickToPrice` values are produced by
//!    `fixtures/GenFuzz.t.sol` (run against the Midnight contracts at rev
//!    f47568c9e45a9b70830b82a130b47393dcafec33) and baked below. The Rust generator mirrors the
//!    Solidity one byte-for-byte, so `hash_offer` / `tick_to_price` must reproduce every value.

use k256::ecdsa::SigningKey;
use nocturne::*;
use proptest::prelude::*;

/// 1 WAD (1e18) — the maximum a price can reach.
fn wad_u256() -> U256 {
    U256::from(1_000_000_000_000_000_000u64)
}

// ------------------------------------------------------------------------------------------------
// Strategies
// ------------------------------------------------------------------------------------------------

prop_compose! {
    fn arb_collateral()(
        token in any::<[u8; 20]>(),
        lltv in any::<[u8; 32]>(),
        cursor in any::<[u8; 32]>(),
        oracle in any::<[u8; 20]>(),
    ) -> CollateralParams {
        CollateralParams { token, lltv, liquidation_cursor: cursor, oracle }
    }
}

fn arb_market() -> impl Strategy<Value = Market> {
    (
        any::<[u8; 32]>(),                             // chain_id
        any::<[u8; 20]>(),                             // midnight
        any::<[u8; 20]>(),                             // loan_token
        prop::collection::vec(arb_collateral(), 1..4), // collateral_params
        any::<[u8; 32]>(),                             // maturity
        any::<[u8; 32]>(),                             // rcf_threshold
        any::<[u8; 20]>(),                             // enter_gate
        any::<[u8; 20]>(),                             // liquidator_gate
    )
        .prop_map(
            |(chain_id, midnight, loan_token, cps, maturity, rcf, eg, lg)| Market {
                chain_id,
                midnight,
                loan_token,
                collateral_params: cps,
                maturity,
                rcf_threshold: rcf,
                enter_gate: eg,
                liquidator_gate: lg,
            },
        )
}

/// A fully random `Offer` — every field is arbitrary bytes. Used for hashing / signing / validation
/// where the code path never does unbounded arithmetic, so full-range values are safe.
fn arb_offer() -> impl Strategy<Value = Offer> {
    (
        arb_market(),
        any::<bool>(),                             // buy
        any::<[u8; 20]>(),                         // maker
        any::<[u8; 32]>(),                         // start
        any::<[u8; 32]>(),                         // expiry
        any::<[u8; 32]>(),                         // tick
        any::<[u8; 32]>(),                         // group
        any::<[u8; 20]>(),                         // callback
        prop::collection::vec(any::<u8>(), 0..64), // callback_data
        any::<[u8; 20]>(),                         // receiver_if_maker_is_seller
        any::<[u8; 20]>(),                         // ratifier
    )
        .prop_flat_map(
            |(market, buy, maker, start, expiry, tick, group, callback, cbd, recv, ratifier)| {
                (
                    Just((
                        market, buy, maker, start, expiry, tick, group, callback, cbd, recv,
                        ratifier,
                    )),
                    any::<bool>(),     // reduce_only
                    any::<u128>(),     // max_units
                    any::<u128>(),     // max_assets
                    any::<[u8; 32]>(), // continuous_fee_cap
                )
                    .prop_map(
                        |(
                            (
                                market,
                                buy,
                                maker,
                                start,
                                expiry,
                                tick,
                                group,
                                callback,
                                cbd,
                                recv,
                                ratifier,
                            ),
                            reduce_only,
                            max_units,
                            max_assets,
                            cfc,
                        )| Offer {
                            market,
                            buy,
                            maker,
                            start,
                            expiry,
                            tick,
                            group,
                            callback,
                            callback_data: cbd,
                            receiver_if_maker_is_seller: recv,
                            ratifier,
                            reduce_only,
                            max_units,
                            max_assets,
                            continuous_fee_cap: cfc,
                        },
                    )
            },
        )
}

/// A power-of-two count of random leaves, count in {1,2,4,...,256}.
fn arb_pow2_leaves() -> impl Strategy<Value = Vec<Word>> {
    (0usize..=8).prop_flat_map(|e| prop::collection::vec(any::<[u8; 32]>(), 1usize << e))
}

// ------------------------------------------------------------------------------------------------
// Part A — property tests
// ------------------------------------------------------------------------------------------------

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    /// Every leaf's proof verifies; corrupting one proof word breaks it.
    #[test]
    fn merkle_proofs_are_sound(leaves in arb_pow2_leaves()) {
        let n = leaves.len();
        let tree = OfferTree::build(leaves.clone()).unwrap();
        let root = tree.root();
        prop_assert_eq!(tree.height(), n.trailing_zeros() as usize);

        for i in 0..n {
            let proof = tree.proof(i);
            prop_assert!(verify_leaf(&root, &leaves[i], i, &proof), "leaf {} must verify", i);

            // Corrupting any proof word must make verification fail (skip the 1-leaf tree,
            // whose proof is empty).
            if !proof.is_empty() {
                let mut bad = proof.clone();
                bad[0][0] ^= 1;
                prop_assert!(!verify_leaf(&root, &leaves[i], i, &bad));
            }
        }
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(128))]

    /// sign_digest -> recover -> verify round-trips; tampering the offer or the maker fails.
    #[test]
    fn signing_roundtrip(
        scalar in any::<[u8; 32]>(),
        offer in arb_offer(),
        chain_id in any::<[u8; 32]>(),
        ratifier in any::<[u8; 20]>(),
    ) {
        let sk = SigningKey::from_bytes(&scalar.into());
        prop_assume!(sk.is_ok()); // reject the (astronomically rare) zero / >= order scalar
        let sk = sk.unwrap();
        let signer = signer_address(&sk);

        let leaf = hash_offer(&offer);
        let tree = OfferTree::build(vec![leaf]).unwrap();
        let root = tree.root();
        let proof = tree.proof(0);
        let digest = tree_digest(root, tree.height(), chain_id, &ratifier);
        let sig = sign_digest(&sk, &digest);

        prop_assert_eq!(recover(&digest, &sig), Some(signer));
        prop_assert!(verify(&offer, &root, 0, &proof, &sig, chain_id, &ratifier, &signer));

        // Flipping a byte of the offer changes the leaf -> proof no longer matches the root.
        let mut tampered = offer.clone();
        tampered.group[0] ^= 1;
        prop_assert!(!verify(&tampered, &root, 0, &proof, &sig, chain_id, &ratifier, &signer));

        // A wrong maker is rejected (ecrecover returns the true signer, not `wrong`).
        let mut wrong = signer;
        wrong[0] ^= 1;
        prop_assert!(!verify(&offer, &root, 0, &proof, &sig, chain_id, &ratifier, &wrong));
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(512))]

    /// In-range ticks price to Ok, never above 1 WAD.
    #[test]
    fn tick_in_range_is_bounded(tick in 0u64..=MAX_TICK) {
        let p = tick_to_price(tick).expect("in-range tick must price");
        prop_assert!(p <= wad_u256(), "tick {} priced above WAD", tick);
    }

    /// Price is non-decreasing in tick.
    #[test]
    fn tick_price_is_monotonic(a in 0u64..=MAX_TICK, b in 0u64..=MAX_TICK) {
        let (lo, hi) = if a <= b { (a, b) } else { (b, a) };
        prop_assert!(tick_to_price(lo).unwrap() <= tick_to_price(hi).unwrap());
    }

    /// Out-of-range ticks return an error, never a price.
    #[test]
    fn tick_out_of_range_errors(tick in (MAX_TICK + 1)..=u64::MAX) {
        prop_assert!(tick_to_price(tick).is_err());
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    /// `validate_offer` / `is_valid` never panic on fully adversarial offers.
    #[test]
    fn validate_never_panics(
        offer in arb_offer(),
        chain_id in any::<u64>(),
        now in any::<u64>(),
        midnight in any::<[u8; 20]>(),
        tick_spacing in any::<u8>(),
        continuous_fee in any::<u128>(),
        loss in any::<bool>(),
    ) {
        let ctx = ValidateCtx {
            chain_id: Some(chain_id),
            midnight: Some(midnight),
            now: Some(now),
            market: Some(MarketSnapshot { tick_spacing, loss_factor_maxed: loss, continuous_fee }),
        };
        // A panic here fails the test; we only assert it returns.
        let _ = validate_offer(&offer, &ctx);
        let _ = is_valid(&offer, &ctx);
        // The stateless subset (default ctx) must also be safe.
        let _ = validate_offer(&offer, &ValidateCtx::default());
    }

    /// `take_amounts` / `simulate_take` return (Ok or graceful Err) without panicking. The offer
    /// keeps an adversarial (full-range) `tick` word to exercise the tick error paths, while the
    /// numeric ctx is kept in plausible ranges so the outcome math has no reason to overflow.
    #[test]
    fn simulate_never_panics(
        mut offer in arb_offer(),
        maturity in any::<u64>(),
        now in any::<u64>(),
        units in any::<u64>(),
        continuous_fee in any::<u64>(),
        cbps in any::<[u16; 7]>(),
        consumed in any::<u128>(),
        credit in any::<u128>(),
        debt in any::<u128>(),
        pending in any::<u128>(),
        tick_spacing in any::<u8>(),
        taker_is_maker in any::<bool>(),
        loss in any::<bool>(),
    ) {
        // Keep maturity within a plausible (u64) horizon so `continuous_fee * ttm` stays in range.
        offer.market.maturity = word_from_u64(maturity);

        let ctx = SimCtx {
            now,
            market: SimMarket {
                tick_spacing,
                continuous_fee: continuous_fee as u128,
                settlement_fee_cbp: cbps,
                loss_factor_maxed: loss,
            },
            consumed,
            maker_position: Position { credit, debt, pending_fee: pending },
            taker_position: Position { credit: debt, debt: credit, pending_fee: pending },
            taker_is_maker,
        };

        // All three must return; a panic (e.g. an unwrap firing in the library) fails the test.
        let _ = take_amounts(&offer, U256::from(units), now, cbps);
        let _ = simulate_take(&offer, U256::from(units), &ctx);
    }
}

// ------------------------------------------------------------------------------------------------
// Part B — differential parity vs the real contracts (see fixtures/GenFuzz.t.sol)
// ------------------------------------------------------------------------------------------------

/// `(HashLib.hashOffer(offer_i), TickLib.tickToPrice(tick_i))` for i in 0..32, emitted by
/// `fixtures/GenFuzz.t.sol` against the Midnight contracts at rev
/// f47568c9e45a9b70830b82a130b47393dcafec33.
const VECTORS: &[(&str, &str)] = &[
    (
        "0x9d789446496208f38638a4af3262d2957049cb5962907e570a9ed30ec38e2ae7",
        "979964600000000000",
    ),
    (
        "0x8224c1c15d968e1b9733ffa4f778a43946586d0f8f87c275ed6733dc0070875d",
        "147300000000000",
    ),
    (
        "0x8f1719a06d84f2798dc6c45dbe32c5588db090717e785df3d6a4db04b2255a52",
        "999758700000000000",
    ),
    (
        "0x9e6b217ae7dd0929d9ceefced02dcfd8aab9a28efbe63e2243ed5caaa97b0432",
        "76672300000000000",
    ),
    (
        "0xa28308629ff373714de612dda5f923f5ba49cc84ad966fad0690c19dd8ccd09b",
        "950620000000000000",
    ),
    (
        "0x93d585d5162ecc4ef1279d97bf3237cde7193b6e5970e292373c494e3a0cc3fd",
        "291700000000000",
    ),
    (
        "0xfb93b05cc889af8a7079322fc0bb2cc19d025f6602ca36b06db54373bed756d5",
        "600000000000",
    ),
    (
        "0x9c520b1063c4f62367ed9a12a6fd22fa0927166e20c1e0859e474302f17e4d02",
        "997347800000000000",
    ),
    (
        "0x2839eff1826b4ceea1d3f5257656820e1f64ca32800332f3ce2d4fd8de5aaa01",
        "999999900000000000",
    ),
    (
        "0x693372dd17f6e8e776228d4a176df57207847de81875088f08558ecdc018e61d",
        "317975100000000000",
    ),
    (
        "0xa872532c741f7ffcf484b0e6b7536352a17a0aa87008fe11dd06ce1cbfbe9237",
        "400000000000",
    ),
    (
        "0xc41df9134413366c322263425395bfb4983a818277c3845f42e676bc44a091bc",
        "999999900000000000",
    ),
    (
        "0x691aa6894e12c1bda16fdb9d89154af57fb5dd8641f06c6c1912303f1dc390e4",
        "999999500000000000",
    ),
    (
        "0xfe897475f4d97818de62cf8318fbd1a573ac23a8212d1893c3a374eaf070a779",
        "985788800000000000",
    ),
    (
        "0xe8809c70818db509871211aaf9c2313c9a1c6c67393f8ef295079bb3784047e6",
        "5900000000000",
    ),
    (
        "0xbf0d8c39c0a96bf6b5b5a2e53bd9ad9582f172fb2695762a49afd94ce42bcd12",
        "120700000000000",
    ),
    (
        "0x460eb7f697fc67b9ee28f792a214a82ad7a968b38cb432f4d06ca8bb8587ed1e",
        "200000000000",
    ),
    (
        "0xe9f1f6e20565af4f050afc2ac9bb3ce517bafdacf08af7f72effc276c89d4966",
        "999999300000000000",
    ),
    (
        "0x618adfdbefb8905106adec7aa4db9a5b70ad2b9a60157ac767ad561626681808",
        "999999900000000000",
    ),
    (
        "0x86d99b13c447183a448a0e0058eb1dc2a67adefdf0c33c1bfede99c9bc07c44d",
        "126182100000000000",
    ),
    (
        "0x26df219196a88a6dabf8286fb08a3d06a56034db88a01900cb94e1c70487437c",
        "190900000000000",
    ),
    (
        "0x0960ee9dc26fe15c6215a7c5b4e0197f6a5115e0233c5ecbe6b8bcc38ed72e4b",
        "43855800000000000",
    ),
    (
        "0x73301d8a142d92072ec8c6ee4782d15e1abafcac2504a94943f766cd79b7f300",
        "9300000000000",
    ),
    (
        "0x0f5dfaac07959d7de182794c4406042e544b9c091e2fdabccef7b2b466d3e368",
        "999996600000000000",
    ),
    (
        "0x5b19303d10915a5623e8d2c1aa73e41e83e66b98fabfb8f92cfce8f8e744cf57",
        "999989300000000000",
    ),
    (
        "0x46661b2eeab903ee6a4c8706921c9692ab577c6e81e8111da77facbbf40309b9",
        "654392800000000000",
    ),
    (
        "0x610fd6468a5f6df8771120d7329558d3880578d59016daea485423bf16f2251c",
        "24000000000000",
    ),
    (
        "0xb1a9d0bad05f01911897c391a6f54530bc06d6a004243b87007b59e55275c510",
        "999985400000000000",
    ),
    (
        "0xe16cf3875ae54c3ab353b2cfeef14c0f3369f7cbb5bb9d3f2ad7cfa4e66a543b",
        "999995400000000000",
    ),
    (
        "0x74729a41aac8aedd2e412856ce822dc1ffce2a08aa72dc840601522a08c99fc1",
        "937802400000000000",
    ),
    (
        "0x1d5e61debeedb2300e0a7507acb7fd90c1853d7c929bc8538fe36a2a5770cdb5",
        "601962500000000000",
    ),
    (
        "0xc71e0093910cc0af7b690187a2db77c63dc34450100b43752121fba06b7fbed5",
        "16793400000000000",
    ),
];

fn hx32(s: &str) -> Word {
    let s = s.trim_start_matches("0x");
    let mut w = [0u8; 32];
    for i in 0..32 {
        w[i] = u8::from_str_radix(&s[2 * i..2 * i + 2], 16).unwrap();
    }
    w
}

/// `seed_i = keccak256(abi.encode(uint256(i)))` — keccak of the 32-byte big-endian of `i`.
fn seed(i: u64) -> Word {
    keccak(&u256_to_word(U256::from(i)))
}

/// `seed2_i = keccak256(abi.encode(seed_i))` — keccak of the 32 bytes of `seed_i`.
fn seed2(i: u64) -> Word {
    keccak(&seed(i))
}

/// `tick_i = uint256(seed_i) % (MAX_TICK + 1)`.
fn tick_of(i: u64) -> u64 {
    let m = word_to_u256(&seed(i)) % U256::from(MAX_TICK + 1);
    u64::try_from(m).unwrap()
}

/// Low 20 bytes of a word — `address(uint160(x))`.
fn addr_of_u256(x: U256) -> Address {
    let w = u256_to_word(x);
    let mut a = [0u8; 20];
    a.copy_from_slice(&w[12..]);
    a
}

/// Low 16 bytes of a word — `uint128(uint256(x))`.
fn u128_trunc(w: &Word) -> u128 {
    let mut b = [0u8; 16];
    b.copy_from_slice(&w[16..]);
    u128::from_be_bytes(b)
}

/// Byte-for-byte mirror of `GenFuzzTest.makeOffer(i)`.
fn gen_offer(i: u64) -> Offer {
    let s = seed(i);
    let s2 = seed2(i);
    let su = word_to_u256(&s);
    let s2u = word_to_u256(&s2);

    let market = Market {
        chain_id: u256_to_word(U256::from(i + 1)),
        midnight: addr_of_u256(su),
        loan_token: addr_of_u256(s2u),
        collateral_params: vec![CollateralParams {
            token: addr_of_u256(su >> 96),
            lltv: s,
            liquidation_cursor: u256_to_word(U256::from(i)),
            oracle: addr_of_u256(s2u >> 96),
        }],
        maturity: u256_to_word(s2u % U256::from(4_000_000_000u64)),
        rcf_threshold: u256_to_word(U256::from(i) * U256::from(1000u64)),
        enter_gate: [0u8; 20],
        liquidator_gate: [0u8; 20],
    };

    // callbackData = first (i % 40) bytes of abi.encodePacked(seed, seed2) (64 bytes).
    let mut both = Vec::with_capacity(64);
    both.extend_from_slice(&s);
    both.extend_from_slice(&s2);
    let len = (i % 40) as usize;
    let callback_data = both[..len].to_vec();

    Offer {
        market,
        buy: i % 2 == 0,
        maker: addr_of_u256(s2u >> 8),
        start: [0u8; 32],
        expiry: u256_to_word(U256::from(2_000_000_000u64 + i)),
        tick: u256_to_word(U256::from(tick_of(i))),
        group: s,
        callback: [0u8; 20],
        callback_data,
        receiver_if_maker_is_seller: [0u8; 20],
        ratifier: [0u8; 20],
        reduce_only: i % 3 == 0,
        max_units: u128_trunc(&s),
        max_assets: 0,
        continuous_fee_cap: s2,
    }
}

#[test]
fn hash_offer_matches_hashlib_over_32_vectors() {
    for (i, (expect_hash, _)) in VECTORS.iter().enumerate() {
        let got = hash_offer(&gen_offer(i as u64));
        assert_eq!(got, hx32(expect_hash), "hashOffer mismatch at i={i}");
    }
}

#[test]
fn tick_to_price_matches_ticklib_over_32_vectors() {
    for (i, (_, expect_price)) in VECTORS.iter().enumerate() {
        let tick = tick_of(i as u64);
        let got = tick_to_price(tick).expect("generated tick is always in range");
        let want = U256::from_str_radix(expect_price, 10).unwrap();
        assert_eq!(got, want, "tickToPrice mismatch at i={i} (tick={tick})");
    }
}
