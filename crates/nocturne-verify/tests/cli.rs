//! End-to-end CLI tests: build a real signed payload with the library, then drive the compiled
//! `nocturne-verify` binary over it and assert on its output and exit codes.

use std::process::Command;

use nocturne::*;

const BIN: &str = env!("CARGO_BIN_EXE_nocturne-verify");

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
    let rd = encode_ratifier_data(&sig, &tree.root(), 0, &tree.proof(0));
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
fn verify_passes_for_good_payload() {
    let (hex, _offer, signer) = signed_take();
    let (stdout, _err, code) = run(&["verify", &hex, "--chain-id", "31337"]);
    assert_eq!(code, 0, "{stdout}");
    assert!(stdout.contains("RESULT: PASS"), "{stdout}");
    assert!(stdout.contains(&render_addr(&signer)), "{stdout}");
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
