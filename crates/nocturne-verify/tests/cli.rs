//! End-to-end CLI tests: build a real signed payload with the library, then drive the compiled
//! `nocturne-verify` binary over it and assert on its output and exit codes.

use std::{
    process::Command,
    sync::atomic::{AtomicU64, Ordering},
};

use nocturne::*;

const BIN: &str = env!("CARGO_BIN_EXE_nocturne-verify");
static NEXT_TEMP_NONCE: AtomicU64 = AtomicU64::new(0);

fn offer_for(maker: Address) -> Offer {
    Offer {
        market: Market {
            chain_id: word_from_u64(31337),
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
        maker,
        start: word_from_u64(0),
        expiry: word_from_u64(4_000_000_000),
        tick: word_from_u64(3372),
        group: word_from_u64(1),
        callback: [0u8; 20],
        callback_data: Vec::new(),
        receiver_if_maker_is_seller: [0u8; 20],
        ratifier: [0xbb; 20],
        reduce_only: false,
        max_units: 1_000_000,
        max_assets: 0,
        continuous_fee_cap: word_from_u64(0),
    }
}

/// Returns (take-calldata hex, offer, signer address).
fn signed_take() -> (String, Offer, Address) {
    signed_take_sig(false)
}

/// Like [`signed_take`], but with `high_s` the signature is replaced by its high-`s`
/// (malleable) counterpart `(r, n - s, flipped v)`, which on-chain `ecrecover` still accepts.
fn signed_take_sig(high_s: bool) -> (String, Offer, Address) {
    let signer = LocalSigner::from_bytes(&[0x42; 32]).unwrap();
    let offer = offer_for(signer.address());
    let chain_id = word_from_u64(31337);
    let tree = OfferTree::build(vec![hash_offer(&offer)]).unwrap();
    let mut sig = signer
        .sign_digest(&tree_digest(
            tree.root(),
            tree.height(),
            chain_id,
            &offer.ratifier,
        ))
        .unwrap();
    if high_s {
        sig = Sig {
            r: sig.r,
            s: high_s_counterpart(&sig.s),
            v: if sig.v == 27 { 28 } else { 27 },
        };
    }
    let rd = encode_ratifier_data(&sig, &tree.root(), 0, &tree.proof(0).unwrap());
    let calldata = encode_take_calldata(
        &offer,
        &rd,
        U256::from(250_000u64),
        &[0x77; 20],
        &[0x88; 20],
        &[0u8; 20],
        &[],
    );
    (
        format!("0x{}", hex::encode(&calldata)),
        offer,
        signer.address(),
    )
}

fn run(args: &[&str]) -> (String, String, i32) {
    let out = Command::new(BIN).args(args).output().expect("run binary");
    (
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
        out.status.code().unwrap_or(-1),
    )
}

fn repay_withdraw_payload() -> String {
    let market = offer_for([0; 20]).market;
    let calldata = encode_repay_withdraw_collateral_calldata(
        &market,
        U256::from(1_000u64),
        &[0x77; 20],
        &TokenPermit {
            kind: 2,
            data: vec![0xde, 0xad, 0xbe, 0xef],
        },
        &[CollateralWithdrawal {
            collateral_index: U256::ZERO,
            assets: U256::from(2_000u64),
        }],
        &[0x88; 20],
        U256::ZERO,
        &[0u8; 20],
        U256::from(4_000_000_000u64),
    );
    format!("0x{}", hex::encode(calldata))
}

#[test]
fn decode_take_shows_terms() {
    let (hex, _offer, _) = signed_take();
    let (stdout, _err, code) = run(&["decode", &hex]);
    assert_eq!(code, 0);
    assert!(stdout.contains("take(...)"), "{stdout}");
    assert!(stdout.contains("BUY (maker lends)"), "{stdout}");
    assert!(stdout.contains("price"), "{stdout}");
    assert!(stdout.contains("chain id            : 31337"), "{stdout}");
}

#[test]
fn decode_json_is_valid_json() {
    let (hex, _offer, _) = signed_take();
    let (stdout, _err, code) = run(&["decode", &hex, "--json"]);
    assert_eq!(code, 0);
    let v: serde_json::Value = serde_json::from_str(&stdout).expect("valid json");
    assert_eq!(v["offer"]["buy"], serde_json::Value::Bool(true));
    assert_eq!(v["units"], serde_json::Value::String("250000".to_string()));
}

#[test]
fn decode_repay_withdraw_auto_detects_and_shows_terms() {
    let payload = repay_withdraw_payload();
    let (stdout, stderr, code) = run(&["decode", &payload]);
    assert_eq!(code, 0, "stdout:\n{stdout}\nstderr:\n{stderr}");
    assert!(
        stdout.contains("midnightBundlesV1RepayAndWithdrawCollateral"),
        "{stdout}"
    );
    assert!(stdout.contains("repay assets        : 1000"), "{stdout}");
    assert!(stdout.contains("collateral withdrawals : 1"), "{stdout}");
    assert!(
        stdout.contains("deadline            : 4000000000"),
        "{stdout}"
    );
}

#[test]
fn decode_repay_withdraw_json_is_valid() {
    let payload = repay_withdraw_payload();
    let (stdout, stderr, code) = run(&["decode", &payload, "--type", "repay-withdraw", "--json"]);
    assert_eq!(code, 0, "stdout:\n{stdout}\nstderr:\n{stderr}");
    let value: serde_json::Value = serde_json::from_str(&stdout).expect("valid json");
    assert_eq!(
        value["function"],
        "midnightBundlesV1RepayAndWithdrawCollateral"
    );
    assert_eq!(value["repayAssets"], "1000");
    assert_eq!(value["loanTokenPermit"]["kind"], 2);
    assert_eq!(value["collateralWithdrawals"].as_array().unwrap().len(), 1);
}

#[test]
fn decode_json_is_valid_for_cancel_ratify_and_getter_returns() {
    let maker = [0x42; 20];
    let root = [0x77; 32];
    let cancel = format!(
        "0x{}",
        hex::encode(encode_cancel_root_calldata(&maker, &root))
    );
    let ratify = format!(
        "0x{}",
        hex::encode(encode_set_is_root_ratified_calldata(&maker, &root, true))
    );

    let mut market_state = Vec::new();
    for value in 1..=13 {
        market_state.extend_from_slice(&word_from_u128(value));
    }
    let market_state = format!("0x{}", hex::encode(market_state));

    let mut position = Vec::new();
    for value in 1..=6 {
        position.extend_from_slice(&word_from_u128(value));
    }
    let position = format!("0x{}", hex::encode(position));

    for (args, expected) in [
        (
            vec!["decode", &cancel, "--json"],
            ("maker", serde_json::json!(render_addr(&maker))),
        ),
        (
            vec!["decode", &ratify, "--json"],
            ("ratified", serde_json::json!(true)),
        ),
        (
            vec!["decode", &market_state, "--type", "market-state", "--json"],
            ("totalUnits", serde_json::json!("1")),
        ),
        (
            vec!["decode", &position, "--type", "position", "--json"],
            ("credit", serde_json::json!("1")),
        ),
    ] {
        let (stdout, stderr, code) = run(&args);
        assert_eq!(code, 0, "{stderr}");
        let value: serde_json::Value = serde_json::from_str(&stdout).expect("valid JSON");
        assert_eq!(value[expected.0], expected.1, "{stdout}");
    }
}

#[test]
fn verify_passes_for_good_payload() {
    let (hex, _offer, signer) = signed_take();
    let (stdout, _err, code) = run(&["verify", &hex, "--chain-id", "31337"]);
    assert_eq!(code, 0, "{stdout}");
    assert!(stdout.contains("RESULT: PASS"), "{stdout}");
    assert!(stdout.contains(&render_addr(&signer)), "{stdout}");
    // A low-s signature must not trip the malleability warning.
    assert!(!stdout.contains("[WARN]"), "{stdout}");
}

#[test]
fn verify_warns_but_passes_for_high_s_signature() {
    // ecrecover accepts the malleated high-s counterpart, so the verdict is PASS - but the
    // tool must flag the signature as malleable.
    let (hex, _offer, signer) = signed_take_sig(true);
    let (stdout, _err, code) = run(&["verify", &hex, "--chain-id", "31337"]);
    assert_eq!(code, 0, "{stdout}");
    assert!(stdout.contains("RESULT: PASS"), "{stdout}");
    assert!(stdout.contains(&render_addr(&signer)), "{stdout}");
    assert!(
        stdout.contains("[WARN]") && stdout.contains("high-s"),
        "{stdout}"
    );
}

#[test]
fn verify_fails_for_wrong_expected_maker() {
    let (hex, _offer, _) = signed_take();
    let (stdout, _err, code) = run(&[
        "verify",
        &hex,
        "--chain-id",
        "31337",
        "--expected-maker",
        "0x000000000000000000000000000000000000dead",
    ]);
    assert_eq!(code, 1, "{stdout}");
    assert!(stdout.contains("RESULT: FAIL"), "{stdout}");
}

#[test]
fn verify_fails_for_proof_taller_than_onchain_cap() {
    // HashLib.offerTreeTypeHash reverts TreeTooHigh above height 20, so a 21-element proof can
    // never ratify - the tool must FAIL, not PASS. Fold the leaf up under zero siblings for a
    // consistent (root, proof) without materializing a 2^21 tree.
    let signer = LocalSigner::from_bytes(&[0x42; 32]).unwrap();
    let offer = offer_for(signer.address());
    let proof = vec![[0u8; 32]; MAX_TREE_HEIGHT + 1];
    let mut root = hash_offer(&offer);
    for sib in &proof {
        root = hash_node(&root, sib);
    }
    // No digest exists for height 21 (tree_digest would panic), so any signature does.
    let sig = signer.sign_digest(&keccak(b"no such digest")).unwrap();
    let rd = encode_ratifier_data(&sig, &root, 0, &proof);
    let calldata = encode_take_calldata(
        &offer,
        &rd,
        U256::from(250_000u64),
        &[0x77; 20],
        &[0x88; 20],
        &[0u8; 20],
        &[],
    );
    let payload = format!("0x{}", hex::encode(&calldata));
    let (stdout, _err, code) = run(&["verify", &payload, "--chain-id", "31337"]);
    assert_eq!(code, 1, "{stdout}");
    assert!(stdout.contains("RESULT: FAIL"), "{stdout}");
    assert!(stdout.contains("TreeTooHigh"), "{stdout}");
}

#[test]
fn verify_fails_for_out_of_range_leaf_index() {
    // HashLib.isLeaf reverts LeafIndexOutOfRange unless leafIndex >> proof.length == 0. On a
    // single-leaf tree the empty proof folds identically for any index, so only the range
    // check separates index 2 (reverts on-chain) from index 0 (passes).
    let signer = LocalSigner::from_bytes(&[0x42; 32]).unwrap();
    let offer = offer_for(signer.address());
    let chain_id = word_from_u64(31337);
    let tree = OfferTree::build(vec![hash_offer(&offer)]).unwrap();
    let sig = signer
        .sign_digest(&tree_digest(
            tree.root(),
            tree.height(),
            chain_id,
            &offer.ratifier,
        ))
        .unwrap();
    let rd = encode_ratifier_data(&sig, &tree.root(), 2, &tree.proof(0).unwrap());
    let calldata = encode_take_calldata(
        &offer,
        &rd,
        U256::from(250_000u64),
        &[0x77; 20],
        &[0x88; 20],
        &[0u8; 20],
        &[],
    );
    let payload = format!("0x{}", hex::encode(&calldata));
    let (stdout, _err, code) = run(&["verify", &payload, "--chain-id", "31337"]);
    assert_eq!(code, 1, "{stdout}");
    assert!(stdout.contains("RESULT: FAIL"), "{stdout}");
}

#[test]
fn digest_matches_and_mismatches() {
    let (_hex, offer, _) = signed_take();
    let dir = std::env::temp_dir();
    let path = dir.join(format!("nocturne_verify_offer_{}.json", std::process::id()));
    std::fs::write(&path, serde_json::to_string(&offer).unwrap()).unwrap();
    let path_s = path.to_str().unwrap();

    // Capture the digest the tool computes.
    let (stdout, _e, code) = run(&["digest", path_s, "--chain-id", "31337"]);
    assert_eq!(code, 0, "{stdout}");
    let digest_line = stdout
        .lines()
        .find(|l| l.contains("DIGEST (to sign)"))
        .expect("digest line");
    let digest = digest_line.split(':').nth(1).unwrap().trim().to_string();

    // Feeding it back via --expect must MATCH.
    let (stdout, _e, code) = run(&["digest", path_s, "--chain-id", "31337", "--expect", &digest]);
    assert_eq!(code, 0, "{stdout}");
    assert!(stdout.contains("MATCH"), "{stdout}");

    // A wrong expected digest must MISMATCH and exit non-zero.
    let wrong = "0x".to_string() + &"11".repeat(32);
    let (stdout, _e, code) = run(&["digest", path_s, "--chain-id", "31337", "--expect", &wrong]);
    assert_eq!(code, 1, "{stdout}");
    assert!(stdout.contains("MISMATCH"), "{stdout}");

    let _ = std::fs::remove_file(&path);
}

#[test]
fn digest_eip712_emits_valid_typed_data() {
    let (_hex, offer, _) = signed_take();
    let dir = std::env::temp_dir();
    let path = dir.join(format!(
        "nocturne_verify_eip712_{}.json",
        std::process::id()
    ));
    std::fs::write(&path, serde_json::to_string(&offer).unwrap()).unwrap();

    let (stdout, _e, code) = run(&[
        "digest",
        path.to_str().unwrap(),
        "--chain-id",
        "31337",
        "--eip712",
    ]);
    assert_eq!(code, 0, "{stdout}");
    let v: serde_json::Value = serde_json::from_str(&stdout).expect("valid json");
    assert_eq!(v["primaryType"], "OfferTree");
    assert_eq!(v["domain"]["chainId"], "31337");
    assert!(v["types"]["Offer"].is_array());
    assert!(v["message"]["offerTree"].is_object());
    assert_eq!(
        v["message"]["offerTree"]["buy"],
        serde_json::Value::Bool(true)
    );

    let _ = std::fs::remove_file(&path);
}

#[test]
fn digest_eip712_enforces_matching_and_mismatching_assertions() {
    let (_hex, offer, _) = signed_take();
    let path_buf = std::env::temp_dir().join(format!(
        "nocturne_verify_eip712_assertions_{}.json",
        std::process::id()
    ));
    std::fs::write(&path_buf, serde_json::to_string(&offer).unwrap()).unwrap();
    let path = path_buf.to_str().unwrap();

    let tree = OfferTree::build(vec![hash_offer(&offer)]).unwrap();
    let root = format!("0x{}", hex::encode(tree.root()));
    let digest = format!(
        "0x{}",
        hex::encode(tree_digest(
            tree.root(),
            tree.height(),
            offer.market.chain_id,
            &offer.ratifier,
        ))
    );

    let (stdout, stderr, code) = run(&[
        "digest",
        path,
        "--eip712",
        "--expect",
        &digest,
        "--expect-root",
        &root,
    ]);
    assert_eq!(code, 0, "{stderr}");
    serde_json::from_str::<serde_json::Value>(&stdout).expect("matching output is JSON");

    let bad_digest = format!("0x{}", "11".repeat(32));
    let (stdout, stderr, code) = run(&["digest", path, "--eip712", "--expect", &bad_digest]);
    assert_eq!(code, 1, "{stdout}");
    assert!(
        stdout.is_empty(),
        "mismatch must not emit a signable document"
    );
    assert!(stderr.contains("MISMATCH: --expect"), "{stderr}");

    let bad_root = format!("0x{}", "22".repeat(32));
    let (stdout, stderr, code) = run(&["digest", path, "--eip712", "--expect-root", &bad_root]);
    assert_eq!(code, 1, "{stdout}");
    assert!(
        stdout.is_empty(),
        "mismatch must not emit a signable document"
    );
    assert!(stderr.contains("MISMATCH: --expect-root"), "{stderr}");

    let _ = std::fs::remove_file(path_buf);
}

#[test]
fn bad_hex_errors_cleanly() {
    let (_out, err, code) = run(&["decode", "0xzzzz"]);
    assert_eq!(code, 1);
    assert!(err.contains("invalid hex"), "{err}");
}

/// EIP-55 checksum of an address, matching the binary's rendering.
fn render_addr(a: &Address) -> String {
    let lower = hex::encode(a);
    let hash = keccak(lower.as_bytes());
    let mut out = String::from("0x");
    for (i, c) in lower.chars().enumerate() {
        if c.is_ascii_alphabetic() {
            let byte = hash[i / 2];
            let nibble = if i % 2 == 0 { byte >> 4 } else { byte & 0x0f };
            out.push(if nibble >= 8 {
                c.to_ascii_uppercase()
            } else {
                c
            });
        } else {
            out.push(c);
        }
    }
    out
}

// ---- bundle payloads -----------------------------------------------------------

/// Returns (bundle-calldata hex, both offers, maker) for a 2-fill BuyWithAssetsTarget bundle
/// signed over one 2-leaf tree. With `tamper`, fill[1]'s offer tick is changed AFTER signing,
/// so its leaf no longer sits under the signed root.
fn signed_bundle(tamper: bool) -> (String, [Offer; 2], Address) {
    let signer = LocalSigner::from_bytes(&[0x42; 32]).unwrap();
    let mut offers = [offer_for(signer.address()), offer_for(signer.address())];
    offers[1].tick = word_from_u64(3376);
    let chain_id = word_from_u64(31337);
    let tree = OfferTree::build(offers.iter().map(hash_offer).collect()).unwrap();
    let sig = signer
        .sign_digest(&tree_digest(
            tree.root(),
            tree.height(),
            chain_id,
            &offers[0].ratifier,
        ))
        .unwrap();
    if tamper {
        offers[1].tick = word_from_u64(1000);
    }
    let fills = offers
        .iter()
        .enumerate()
        .map(|(i, offer)| {
            let raw = encode_ratifier_data(&sig, &tree.root(), i, &tree.proof(i).unwrap());
            OfferFill {
                offer: offer.clone(),
                ratifier_data: decode_any_ratifier_data(&raw).unwrap(),
                ratifier_data_raw: raw,
                units: U256::from(100_000u64),
            }
        })
        .collect();
    let bundle = BundleCall {
        kind: BundleKind::BuyWithAssetsTarget,
        target: U256::from(500_000u64),
        limit: U256::from(501_570u64),
        taker: [0x77; 20],
        reduce_only: false,
        side: BundleSide::Buy {
            loan_token_permit: TokenPermit {
                kind: 0,
                data: Vec::new(),
            },
            collateral_withdrawals: Vec::new(),
            collateral_receiver: [0x88; 20],
        },
        fills,
        referral_fee_pct: U256::ZERO,
        referral_fee_recipient: [0u8; 20],
        max_continuous_fee: U256::MAX,
        deadline: U256::from(4_000_000_000u64),
    };
    (
        format!("0x{}", hex::encode(encode_bundle_calldata(&bundle))),
        offers,
        signer.address(),
    )
}

#[test]
fn decode_bundle_shows_wrapper_and_fills() {
    let (hex, _offers, _) = signed_bundle(false);
    let (stdout, _err, code) = run(&["decode", &hex]);
    assert_eq!(code, 0, "{stdout}");
    assert!(
        stdout.contains("bundle (midnightBundlesV1BuyWithAssetsTargetAndWithdrawCollateral)"),
        "{stdout}"
    );
    assert!(stdout.contains("fills               : 2"), "{stdout}");
    assert!(stdout.contains("fill[1]:"), "{stdout}");
    assert!(stdout.contains("target buyer assets : 500000"), "{stdout}");
    assert!(stdout.contains("min units           : 501570"), "{stdout}");
}

#[test]
fn decode_bundle_json_is_valid_json() {
    let (hex, _offers, _) = signed_bundle(false);
    let (stdout, _err, code) = run(&["decode", &hex, "--json"]);
    assert_eq!(code, 0);
    let v: serde_json::Value = serde_json::from_str(&stdout).expect("valid json");
    assert_eq!(
        v["function"],
        "midnightBundlesV1BuyWithAssetsTargetAndWithdrawCollateral"
    );
    assert_eq!(v["fills"].as_array().unwrap().len(), 2);
}

#[test]
fn verify_bundle_passes_and_checks_every_fill() {
    let (hex, _offers, signer) = signed_bundle(false);
    let (stdout, _err, code) = run(&["verify", &hex, "--chain-id", "31337"]);
    assert_eq!(code, 0, "{stdout}");
    assert!(stdout.contains("checks (fill[0]):"), "{stdout}");
    assert!(stdout.contains("checks (fill[1]):"), "{stdout}");
    assert!(
        stdout.contains("RESULT: PASS - all 2 fill signatures"),
        "{stdout}"
    );
    assert!(stdout.contains(&render_addr(&signer)), "{stdout}");
}

#[test]
fn verify_bundle_fails_when_one_fill_is_tampered() {
    // fill[0] is intact, fill[1]'s tick was changed after signing: one bad fill must fail the
    // whole bundle.
    let (hex, _offers, _) = signed_bundle(true);
    let (stdout, _err, code) = run(&["verify", &hex, "--chain-id", "31337"]);
    assert_eq!(code, 1, "{stdout}");
    assert!(stdout.contains("RESULT: FAIL"), "{stdout}");
    // The intact fill still shows its passing leaf check before the tampered one fails.
    assert!(
        stdout.contains("[PASS] offer leaf is under the claimed Merkle root"),
        "{stdout}"
    );
    assert!(
        stdout.contains("[FAIL] offer leaf is under the claimed Merkle root"),
        "{stdout}"
    );
}

#[test]
fn truncated_bundle_payload_errors_with_hint() {
    // Real production payload whose tail was lost in copy-paste: the tool must refuse it with
    // a truncation hint, never report on a partial bundle.
    let payload = include_str!("../../nocturne/tests/data/truncated_bundle.hex").trim();
    for cmd in ["decode", "verify"] {
        let (stdout, stderr, code) = run(&[cmd, payload]);
        assert_eq!(code, 1, "{cmd}: {stdout}{stderr}");
        assert!(stderr.contains("truncated"), "{cmd}: {stderr}");
        assert!(!stdout.contains("RESULT: PASS"), "{cmd}: {stdout}");
    }
}

// ---- SetterRatifier payloads ----------------------------------------------------

/// A take whose ratifierData uses the SetterRatifier layout (root, leafIndex, proof - no
/// signature). With `tamper`, the offer tick is changed after the tree was built.
fn setter_take(tamper: bool) -> String {
    let mut offer = offer_for([0x42; 20]);
    let tree = OfferTree::build(vec![hash_offer(&offer)]).unwrap();
    if tamper {
        offer.tick = word_from_u64(1000);
    }
    let rd = encode_setter_ratifier_data(&tree.root(), 0, &tree.proof(0).unwrap());
    let calldata = encode_take_calldata(
        &offer,
        &rd,
        U256::from(250_000u64),
        &[0x77; 20],
        &[0x88; 20],
        &[0u8; 20],
        &[],
    );
    format!("0x{}", hex::encode(&calldata))
}

#[test]
fn verify_setter_take_is_partial_with_onchain_pointer() {
    let payload = setter_take(false);
    let (stdout, _err, code) = run(&["verify", &payload, "--chain-id", "31337"]);
    assert_eq!(code, 2, "{stdout}");
    assert!(stdout.contains("RESULT: PARTIAL"), "{stdout}");
    assert!(stdout.contains("no signature travels"), "{stdout}");
    assert!(
        stdout.contains("cast call") && stdout.contains("isRootRatified(address,bytes32)(bool)"),
        "{stdout}"
    );
    assert!(
        stdout.contains("[PASS] offer leaf is under the claimed Merkle root"),
        "{stdout}"
    );
}

#[test]
fn verify_tampered_setter_take_fails() {
    let payload = setter_take(true);
    let (stdout, _err, code) = run(&["verify", &payload, "--chain-id", "31337"]);
    assert_eq!(code, 1, "{stdout}");
    assert!(stdout.contains("RESULT: FAIL"), "{stdout}");
    assert!(
        stdout.contains("[FAIL] offer leaf is under the claimed Merkle root"),
        "{stdout}"
    );
}

#[test]
fn verify_production_setter_bundle_is_partial() {
    // The complete production bundle (6 SetterRatifier fills on Base) that motivated setter
    // support: every fill's terms and Merkle membership must verify, the verdict must be
    // PARTIAL (root ratification lives on-chain), and each fill must get a cast pointer.
    let payload = include_str!("../../nocturne/tests/data/setter_bundle_full.hex").trim();
    let (stdout, _err, code) = run(&["verify", payload]);
    assert_eq!(code, 2, "{stdout}");
    assert!(stdout.contains("fills               : 6"), "{stdout}");
    assert_eq!(
        stdout
            .matches("[PASS] offer leaf is under the claimed Merkle root")
            .count(),
        6,
        "{stdout}"
    );
    assert_eq!(stdout.matches("cast call").count(), 6, "{stdout}");
    assert!(!stdout.contains("[FAIL]"), "{stdout}");
    assert!(stdout.contains("RESULT: PARTIAL"), "{stdout}");
}

// ---- setIsRootRatified (maker ratification) payloads -----------------------------

/// Two same-maker offers written as JSON files plus the maker's ratify calldata for their
/// tree root. Returns (calldata hex, [offer paths], root hex).
fn ratify_fixture() -> (String, [String; 2], String) {
    let maker: Address = [0x42; 20];
    let mut offers = [offer_for(maker), offer_for(maker)];
    offers[1].tick = word_from_u64(3376);
    let dir = std::env::temp_dir();
    let nonce = NEXT_TEMP_NONCE.fetch_add(1, Ordering::Relaxed);
    let mut paths = Vec::new();
    for (i, o) in offers.iter().enumerate() {
        let p = dir.join(format!(
            "nocturne_ratify_offer_{}_{nonce}_{i}.json",
            std::process::id(),
        ));
        std::fs::write(&p, serde_json::to_string(o).unwrap()).unwrap();
        paths.push(p.to_str().unwrap().to_string());
    }
    let tree = OfferTree::build(offers.iter().map(hash_offer).collect()).unwrap();
    let calldata = encode_set_is_root_ratified_calldata(&maker, &tree.root(), true);
    (
        format!("0x{}", hex::encode(&calldata)),
        [paths[0].clone(), paths[1].clone()],
        format!("0x{}", hex::encode(tree.root())),
    )
}

#[test]
fn decode_production_ratify_payload() {
    let payload = include_str!("../../nocturne/tests/data/setter_ratify_prod.hex").trim();
    let (stdout, _err, code) = run(&["decode", payload]);
    assert_eq!(code, 0, "{stdout}");
    assert!(stdout.contains("setIsRootRatified"), "{stdout}");
    assert!(
        stdout.contains("0xd418224aE3c510B645112FD9275CCFD50F996ee4"),
        "{stdout}"
    );
    assert!(stdout.contains("RATIFY"), "{stdout}");
}

#[test]
fn verify_ratify_without_offers_is_partial() {
    let (payload, _paths, _root) = ratify_fixture();
    let (stdout, _err, code) = run(&["verify", &payload]);
    assert_eq!(code, 2, "{stdout}");
    assert!(stdout.contains("RESULT: PARTIAL"), "{stdout}");
    assert!(stdout.contains("--offers"), "{stdout}");
}

#[test]
fn verify_ratify_with_matching_offers_passes() {
    let (payload, paths, _root) = ratify_fixture();
    let (stdout, stderr, code) = run(&["verify", &payload, "--offers", &paths[0], &paths[1]]);
    assert_eq!(code, 0, "stdout:\n{stdout}\nstderr:\n{stderr}");
    assert!(
        stdout.contains("[PASS] root commits to exactly the supplied offers"),
        "{stdout}"
    );
    assert!(stdout.contains("RESULT: PASS"), "{stdout}");
}

#[test]
fn verify_ratify_with_wrong_offers_fails() {
    // Same offers, wrong order: different leaf indices, different root.
    let (payload, paths, _root) = ratify_fixture();
    let (stdout, stderr, code) = run(&["verify", &payload, "--offers", &paths[1], &paths[0]]);
    assert_eq!(code, 1, "stdout:\n{stdout}\nstderr:\n{stderr}");
    assert!(stdout.contains("RESULT: FAIL"), "{stdout}");
}

#[test]
fn digest_expect_root_matches_and_mismatches() {
    let (_payload, paths, root) = ratify_fixture();
    let (stdout, _err, code) = run(&["digest", &paths[0], &paths[1], "--expect-root", &root]);
    assert_eq!(code, 0, "{stdout}");
    assert!(stdout.contains("MATCH: the root commits"), "{stdout}");

    let bad = format!("0x{}", "ab".repeat(32));
    let (stdout, _err, code) = run(&["digest", &paths[0], &paths[1], "--expect-root", &bad]);
    assert_eq!(code, 1, "{stdout}");
    assert!(stdout.contains("Do NOT ratify"), "{stdout}");
}

// ---- EIP-712 typed-data (maker off-chain signing) payloads -----------------------

fn write_temp(name: &str, contents: &str) -> String {
    let nonce = NEXT_TEMP_NONCE.fetch_add(1, Ordering::Relaxed);
    let p = std::env::temp_dir().join(format!("nocturne_{}_{nonce}_{name}", std::process::id()));
    std::fs::write(&p, contents).unwrap();
    p.to_str().unwrap().to_string()
}

#[test]
fn verify_typed_production_payload_passes() {
    // A real maker typed-data export from the app (23 offers + 9 zero-padding leaves,
    // height 5, EcrecoverRatifier domain on Base).
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/data/typed_tree_prod.json"
    );
    let (stdout, _err, code) = run(&["verify-typed", path]);
    assert_eq!(code, 0, "{stdout}");
    assert!(stdout.contains("23 real, 9 zero padding"), "{stdout}");
    assert!(
        stdout.contains("[PASS] document is the canonical encoding of the offers shown"),
        "{stdout}"
    );
    assert!(stdout.contains("RESULT: PASS"), "{stdout}");
}

#[test]
fn verify_typed_fails_on_tampered_types_table() {
    // Widening maxUnits to uint256 in the types table changes what the wallet hashes without
    // changing any displayed value - the canonical-encoding check must catch it.
    let raw = include_str!("data/typed_tree_prod.json");
    let tampered = raw.replacen(
        r#"{"name":"maxUnits","type":"uint128"}"#,
        r#"{"name":"maxUnits","type":"uint256"}"#,
        1,
    );
    assert_ne!(raw, tampered, "tamper target not found");
    let path = write_temp("tampered_types.json", &tampered);
    let (stdout, _err, code) = run(&["verify-typed", &path]);
    assert_eq!(code, 1, "{stdout}");
    assert!(
        stdout.contains("[FAIL] document is the canonical encoding of the offers shown"),
        "{stdout}"
    );
    assert!(stdout.contains("RESULT: FAIL"), "{stdout}");
}

#[test]
fn verify_typed_checks_leaves_against_intended_offers() {
    // Two-offer tree: --offers with the same offers passes; a value tampered in the typed
    // data (which the canonical check alone cannot catch) fails the intent comparison.
    let maker: Address = [0x42; 20];
    let mut offers = [offer_for(maker), offer_for(maker)];
    offers[1].tick = word_from_u64(3376);
    let td = serde_json::to_string(&nocturne_typed_data(&offers)).unwrap();
    let td_path = write_temp("typed_two.json", &td);
    let o0 = write_temp(
        "typed_intent_0.json",
        &serde_json::to_string(&offers[0]).unwrap(),
    );
    let o1 = write_temp(
        "typed_intent_1.json",
        &serde_json::to_string(&offers[1]).unwrap(),
    );

    let (stdout, _err, code) = run(&["verify-typed", &td_path, "--offers", &o0, &o1]);
    assert_eq!(code, 0, "{stdout}");
    assert!(
        stdout.contains("[PASS] leaves equal the intended offers passed via --offers"),
        "{stdout}"
    );

    // Tamper one displayed value: tick 3376 -> 1000. Canonical encoding stays valid, so only
    // the --offers intent check can flag it.
    let tampered = td.replacen(r#""tick":"3376""#, r#""tick":"1000""#, 1);
    assert_ne!(td, tampered, "tamper target not found");
    let bad_path = write_temp("typed_two_bad.json", &tampered);
    let (stdout, _err, code) = run(&["verify-typed", &bad_path, "--offers", &o0, &o1]);
    assert_eq!(code, 1, "{stdout}");
    assert!(
        stdout.contains("[FAIL] leaves equal the intended offers passed via --offers"),
        "{stdout}"
    );
}

#[test]
fn verify_typed_accepts_multi_maker_trees() {
    // EcrecoverRatifier accepts a signer authorized by each offer's maker. The makers in one
    // tree therefore need not be identical; authorization is checked when the tree is used.
    let mut offers = [offer_for([0x42; 20]), offer_for([0x43; 20])];
    offers[1].tick = word_from_u64(3376);
    let td = serde_json::to_string(&nocturne_typed_data(&offers)).unwrap();
    let path = write_temp("typed_multi_maker.json", &td);

    let (stdout, _err, code) = run(&["verify-typed", &path]);
    assert_eq!(code, 0, "{stdout}");
    assert!(
        stdout.contains("[PASS] document is the canonical encoding of the offers shown"),
        "{stdout}"
    );
    assert!(stdout.contains("RESULT: PASS"), "{stdout}");
}

/// Emit typed data through the binary (digest --eip712) so the test exercises the same
/// document shape the tool itself produces.
fn nocturne_typed_data(offers: &[Offer; 2]) -> serde_json::Value {
    let o0 = write_temp(
        "gen_offer_0.json",
        &serde_json::to_string(&offers[0]).unwrap(),
    );
    let o1 = write_temp(
        "gen_offer_1.json",
        &serde_json::to_string(&offers[1]).unwrap(),
    );
    let (stdout, _err, code) = run(&["digest", &o0, &o1, "--eip712"]);
    assert_eq!(code, 0, "{stdout}");
    serde_json::from_str(&stdout).unwrap()
}
