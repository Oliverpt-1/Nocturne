//! `nocturne-verify` - an offline decoder and verifier for Morpho Midnight offer payloads.
//!
//! A Midnight offer is a deep, heavily-nested payload, and a signer staring at a 32-byte digest
//! (or a wall of hex) cannot tell what they are about to authorize. This tool reproduces, entirely
//! offline, the two things that matter:
//!
//! * **what the bytes say** - decode a `take` payload / offer / ratifier blob into readable terms;
//! * **what the signature commits to** - reproduce the Merkle root and the EIP-712 digest, and
//!   confirm the signature recovers to the intended maker.
//!
//! Nothing here touches the network or holds a key. It is a deliberately independent
//! reimplementation, parity-checked against the Midnight contracts, so it can catch a bug (or a
//! tampered field) in whatever produced the payload.
//!
//! See the crate README for the full workflow and the independence caveat.

use std::process::ExitCode;

use clap::{Parser, Subcommand, ValueEnum};
use nocturne::*;

mod eip712;
mod render;

#[derive(Parser)]
#[command(
    name = "nocturne-verify",
    version,
    about = "Offline decoder + signature/Merkle-root verifier for Morpho Midnight offers"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Decode a raw payload (take or bundle calldata, offer, ratifier data, or a getter
    /// return) into human-readable terms.
    Decode {
        /// The payload as hex (`0x`-prefixed or not).
        payload: String,
        /// Force the payload type instead of auto-detecting from the selector.
        #[arg(long, value_enum)]
        r#type: Option<PayloadType>,
        /// Reference Unix time, used to compute APR from the tick.
        #[arg(long)]
        now: Option<u64>,
        /// Emit JSON instead of the text report.
        #[arg(long)]
        json: bool,
    },
    /// Verify a `take` or bundle payload: reproduce the signed Merkle root(s) and confirm
    /// each signature recovers to its offer's maker. Exits non-zero if any check fails.
    Verify {
        /// The `take` or bundle calldata as hex.
        payload: String,
        /// Chain id for the EIP-712 domain (defaults to the offer's `market.chainId`).
        #[arg(long)]
        chain_id: Option<u64>,
        /// Also assert the recovered signer equals this address.
        #[arg(long)]
        expected_maker: Option<String>,
        /// Reference Unix time, used to compute APR in the printed terms.
        #[arg(long)]
        now: Option<u64>,
    },
    /// Reproduce the Merkle root and EIP-712 digest from the offer terms you *intend* to sign
    /// (JSON files), so you can compare against what a wallet displays.
    Digest {
        /// One or more offer JSON files (each a serialized `Offer`). Order sets leaf indices.
        offers: Vec<std::path::PathBuf>,
        /// Chain id for the EIP-712 domain (defaults to the first offer's `market.chainId`).
        #[arg(long)]
        chain_id: Option<u64>,
        /// Ratifier / verifying contract (defaults to the first offer's `ratifier`).
        #[arg(long)]
        ratifier: Option<String>,
        /// Assert the computed digest equals this 32-byte hex value; exit non-zero otherwise.
        #[arg(long)]
        expect: Option<String>,
        /// Emit the full EIP-712 typed-data JSON (for diffing against an eth_signTypedData_v4
        /// wallet) instead of the digest summary.
        #[arg(long)]
        eip712: bool,
    },
}

#[derive(Clone, Copy, ValueEnum)]
enum PayloadType {
    Take,
    Bundle,
    Offer,
    Ratifier,
    Cancel,
    MarketState,
    Position,
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    let result = match cli.command {
        Command::Decode {
            payload,
            r#type,
            now,
            json,
        } => cmd_decode(&payload, r#type, now, json),
        Command::Verify {
            payload,
            chain_id,
            expected_maker,
            now,
        } => cmd_verify(&payload, chain_id, expected_maker.as_deref(), now),
        Command::Digest {
            offers,
            chain_id,
            ratifier,
            expect,
            eip712,
        } => cmd_digest(
            &offers,
            chain_id,
            ratifier.as_deref(),
            expect.as_deref(),
            eip712,
        ),
    };
    match result {
        Ok(code) => code,
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}

// ---- commands ----------------------------------------------------------------

fn cmd_decode(
    payload: &str,
    forced: Option<PayloadType>,
    now: Option<u64>,
    json: bool,
) -> Result<ExitCode, String> {
    let bytes = parse_hex(payload)?;
    let kind = forced.map(Ok).unwrap_or_else(|| detect(&bytes))?;

    match kind {
        PayloadType::Take => {
            let t = decode_take_calldata(&bytes).map_err(|e| e.to_string())?;
            if json {
                print_json(&render::take_json(&t));
            } else {
                println!("Decoded Midnight take(...) payload\n");
                print!("{}", render::take_text(&t, now));
            }
        }
        PayloadType::Bundle => {
            let b = decode_bundle_calldata(&bytes).map_err(bundle_err)?;
            if json {
                print_json(&render::bundle_json(&b));
            } else {
                println!("Decoded Midnight bundle payload\n");
                print!("{}", render::bundle_text(&b, now));
            }
        }
        PayloadType::Offer => {
            let o = decode_offer(&bytes).map_err(|e| e.to_string())?;
            if json {
                print_json(&render::offer_json(&o));
            } else {
                println!("Decoded offer\n");
                print!("{}", render::offer_text(&o, now));
            }
        }
        PayloadType::Ratifier => {
            let rd = decode_any_ratifier_data(&bytes).map_err(|e| e.to_string())?;
            if json {
                print_json(&render::ratifier_payload_json(&rd));
            } else {
                println!("Decoded ratifier data\n");
                print!("{}", render::ratifier_payload_text(&rd));
            }
        }
        PayloadType::Cancel => {
            let (maker, root) = decode_cancel_root_calldata(&bytes).map_err(|e| e.to_string())?;
            println!("Decoded cancelRoot(...) payload\n");
            println!("  maker : {}", render::checksum(&maker));
            println!("  root  : {}", render::hex_bytes(&root));
        }
        PayloadType::MarketState => {
            let m = decode_market_state(&bytes).map_err(|e| e.to_string())?;
            println!("Decoded marketState return\n");
            print!("{}", render::market_state_text(&m));
        }
        PayloadType::Position => {
            let p = decode_position(&bytes).map_err(|e| e.to_string())?;
            println!("Decoded position return\n");
            print!("{}", render::position_text(&p));
        }
    }
    Ok(ExitCode::SUCCESS)
}

fn cmd_verify(
    payload: &str,
    chain_id: Option<u64>,
    expected_maker: Option<&str>,
    now: Option<u64>,
) -> Result<ExitCode, String> {
    let bytes = parse_hex(payload)?;
    if bytes.len() >= 4
        && BundleKind::from_selector([bytes[0], bytes[1], bytes[2], bytes[3]]).is_some()
    {
        return cmd_verify_bundle(&bytes, chain_id, expected_maker, now);
    }
    let t = decode_take_calldata(&bytes).map_err(|e| e.to_string())?;

    println!("Verifying Midnight take(...) payload\n");
    print!("{}", render::take_text(&t, now));
    println!("\nchecks:");

    let mut ok = true;
    let mut partial = false;
    verify_offer_checks(
        &t.offer,
        &t.ratifier_data,
        chain_id,
        expected_maker,
        &mut ok,
        &mut partial,
    )?;

    println!();
    Ok(verdict(ok, partial, 1))
}

/// Print the overall RESULT line and pick the exit code: 0 when every check passed, 1 when
/// something failed, 2 when nothing failed but SetterRatifier data leaves the on-chain root
/// ratification check open (offline tools cannot read contract state).
fn verdict(ok: bool, partial: bool, fills: usize) -> ExitCode {
    if !ok {
        println!("RESULT: FAIL - do NOT trust this payload; see failed checks above.");
        ExitCode::FAILURE
    } else if partial {
        println!(
            "RESULT: PARTIAL - terms and Merkle membership verified, but SetterRatifier root \
             ratification is on-chain state this tool cannot check offline. Run the cast \
             command(s) above to complete the verification."
        );
        ExitCode::from(2)
    } else if fills == 1 {
        println!("RESULT: PASS - this signature authorizes exactly the offer shown above.");
        ExitCode::SUCCESS
    } else {
        println!(
            "RESULT: PASS - all {fills} fill signatures authorize exactly the offers shown above."
        );
        ExitCode::SUCCESS
    }
}

fn cmd_verify_bundle(
    bytes: &[u8],
    chain_id: Option<u64>,
    expected_maker: Option<&str>,
    now: Option<u64>,
) -> Result<ExitCode, String> {
    let b = decode_bundle_calldata(bytes).map_err(bundle_err)?;

    println!("Verifying Midnight bundle payload\n");
    print!("{}", render::bundle_summary_text(&b));

    if b.fills.is_empty() {
        println!("\nRESULT: FAIL - bundle contains no fills (nothing to verify).");
        return Ok(ExitCode::FAILURE);
    }

    let mut ok = true;
    let mut partial = false;
    for (i, fill) in b.fills.iter().enumerate() {
        println!("\nfill[{i}]:");
        print!("{}", render::fill_text(fill, now));
        println!("\nchecks (fill[{i}]):");
        verify_offer_checks(
            &fill.offer,
            &fill.ratifier_data,
            chain_id,
            expected_maker,
            &mut ok,
            &mut partial,
        )?;
    }

    println!();
    Ok(verdict(ok, partial, b.fills.len()))
}

/// The per-offer verification core shared by bare takes and bundle fills: leaf membership,
/// tree-height cap, then per-layout authorization - digest reconstruction + signer recovery
/// for EcrecoverRatifier data, or the on-chain root-ratification pointer for SetterRatifier
/// data (which carries no signature; sets `partial`). Failures clear `ok`.
fn verify_offer_checks(
    offer: &Offer,
    rd: &RatifierPayload,
    chain_id: Option<u64>,
    expected_maker: Option<&str>,
    ok: &mut bool,
    partial: &mut bool,
) -> Result<(), String> {
    // Chain id: explicit flag wins; otherwise trust the offer's own market.chainId.
    let chain_id_word = match chain_id {
        Some(id) => word_from_u64(id),
        None => offer.market.chain_id,
    };
    let ratifier = offer.ratifier;

    // 1. The offer's leaf sits under the claimed root at the claimed index.
    let leaf = hash_offer(offer);
    let leaf_ok = verify_leaf(rd.root(), &leaf, rd.leaf_index(), rd.proof());
    check(ok, leaf_ok, "offer leaf is under the claimed Merkle root");

    // 2. The tree fits the on-chain cap: HashLib.offerTreeTypeHash reverts TreeTooHigh above
    //    height 20, so a longer proof can never ratify and has no digest to sign.
    let height_ok = rd.proof().len() <= MAX_TREE_HEIGHT;
    check(ok, height_ok, "proof height is at most 20 (TreeTooHigh)");

    match rd {
        RatifierPayload::Ecrecover(rd) => {
            // 3. Recompute the EIP-712 digest and recover the signer.
            let digest =
                height_ok.then(|| tree_digest(rd.root, rd.proof.len(), chain_id_word, &ratifier));
            let signer = digest.and_then(|d| recover(&d, &rd.sig));
            let signer_is_maker = signer.as_ref() == Some(&offer.maker);
            check(
                ok,
                signer_is_maker,
                "signature recovers to the offer's maker",
            );
            if let Some(s) = signer {
                println!("      recovered signer  : {}", render::checksum(&s));
            } else {
                println!("      recovered signer  : (invalid signature)");
            }
            if let Some(d) = digest {
                println!("      digest            : {}", render::hex_bytes(&d));
            }
            // Malleability note: on-chain ecrecover (and therefore `recover`) accepts a high-s
            // signature, but low-s tooling may reject or rewrite it - flag it even on a PASS.
            if signer.is_some() && is_high_s(&rd.sig.s) {
                println!(
                    "  [WARN] signature is high-s (malleable, not low-s normalized); \
                     on-chain ecrecover accepts it"
                );
            }

            // 4. Optional explicit maker assertion.
            if let Some(exp) = expected_maker {
                let exp_addr = parse_addr(exp)?;
                check(
                    ok,
                    signer.as_ref() == Some(&exp_addr),
                    "recovered signer equals --expected-maker",
                );
            }
        }
        RatifierPayload::Setter(rd) => {
            // 3. SetterRatifier data carries no signature: `isRatified` instead requires
            //    isRootRatified[offer.maker][root] == true, which is contract storage this
            //    offline tool cannot read. Point at the exact call that completes the check.
            *partial = true;
            println!("  [INFO] SetterRatifier data: no signature travels with this payload");
            println!(
                "  [INFO] authorization is on-chain state; complete the check with:\n\
                 \x20        cast call {} \"isRootRatified(address,bytes32)(bool)\" {} {} \\\n\
                 \x20          --rpc-url <chain-{} rpc>",
                render::checksum(&ratifier),
                render::checksum(&offer.maker),
                render::hex_bytes(&rd.root),
                word_to_u256(&offer.market.chain_id),
            );

            // 4. Optional explicit maker assertion: with no signer to recover, assert the
            //    offer's maker field itself (that is whose ratification binds on-chain).
            if let Some(exp) = expected_maker {
                let exp_addr = parse_addr(exp)?;
                check(
                    ok,
                    offer.maker == exp_addr,
                    "offer maker equals --expected-maker",
                );
            }
        }
    }

    // 5. Cross-check: if a chain id was supplied, it should match the offer's own.
    if let Some(id) = chain_id {
        let matches = offer.market.chain_id == word_from_u64(id);
        check(ok, matches, "--chain-id matches the offer's market.chainId");
    }
    Ok(())
}

fn cmd_digest(
    paths: &[std::path::PathBuf],
    chain_id: Option<u64>,
    ratifier: Option<&str>,
    expect: Option<&str>,
    eip712: bool,
) -> Result<ExitCode, String> {
    if paths.is_empty() {
        return Err("provide at least one offer JSON file".to_string());
    }
    let mut offers = Vec::new();
    for p in paths {
        let text = std::fs::read_to_string(p).map_err(|e| format!("{}: {e}", p.display()))?;
        let offer: Offer =
            serde_json::from_str(&text).map_err(|e| format!("{}: {e}", p.display()))?;
        offers.push(offer);
    }

    let chain_id_word = match chain_id {
        Some(id) => word_from_u64(id),
        None => offers[0].market.chain_id,
    };
    let ratifier_addr = match ratifier {
        Some(r) => parse_addr(r)?,
        None => offers[0].ratifier,
    };

    let leaves: Vec<Word> = offers.iter().map(hash_offer).collect();
    let tree = OfferTree::build(leaves.clone()).map_err(|e| format!("tree: {e:?}"))?;
    let root = tree.root();
    let height = tree.height();
    let domain = domain_separator(chain_id_word, &ratifier_addr);
    let digest = tree_digest(root, height, chain_id_word, &ratifier_addr);

    if eip712 {
        let td = eip712::typed_data(&offers, chain_id_word, &ratifier_addr, height);
        print_json(&td);
        return Ok(ExitCode::SUCCESS);
    }

    println!("Reproduced EIP-712 signing digest from intended terms\n");
    println!("  chain id            : {}", word_to_u256(&chain_id_word));
    println!(
        "  ratifier (verifier) : {}",
        render::checksum(&ratifier_addr)
    );
    for (i, leaf) in leaves.iter().enumerate() {
        println!("  leaf[{i}]             : {}", render::hex_bytes(leaf));
    }
    println!("  merkle root         : {}", render::hex_bytes(&root));
    println!("  tree height         : {height}");
    println!("  domain separator    : {}", render::hex_bytes(&domain));
    println!("  DIGEST (to sign)    : {}", render::hex_bytes(&digest));

    if let Some(exp) = expect {
        let exp_word = parse_word(exp)?;
        println!();
        if exp_word == digest {
            println!("MATCH: the wallet digest matches the intended terms.");
            Ok(ExitCode::SUCCESS)
        } else {
            println!(
                "MISMATCH: --expect {} != {}",
                render::hex_bytes(&exp_word),
                render::hex_bytes(&digest)
            );
            println!("Do NOT sign: the digest does not correspond to these terms.");
            Ok(ExitCode::FAILURE)
        }
    } else {
        Ok(ExitCode::SUCCESS)
    }
}

// ---- helpers -----------------------------------------------------------------

fn check(ok: &mut bool, pass: bool, label: &str) {
    println!("  [{}] {label}", if pass { "PASS" } else { "FAIL" });
    if !pass {
        *ok = false;
    }
}

fn detect(bytes: &[u8]) -> Result<PayloadType, String> {
    if bytes.len() >= 4 {
        let sel = [bytes[0], bytes[1], bytes[2], bytes[3]];
        if sel == TAKE_SELECTOR {
            return Ok(PayloadType::Take);
        }
        if sel == CANCEL_ROOT_SELECTOR {
            return Ok(PayloadType::Cancel);
        }
        if BundleKind::from_selector(sel).is_some() {
            return Ok(PayloadType::Bundle);
        }
    }
    if decode_offer(bytes).is_ok() {
        return Ok(PayloadType::Offer);
    }
    Err(
        "could not auto-detect payload type; pass --type (getter returns like \
         market-state/position have no selector and must be specified)"
            .to_string(),
    )
}

/// Decode-error message for bundle payloads, with a hint for the common truncated-copy case
/// (a bundle whose ABI offsets point past its own end).
fn bundle_err(e: DecodeError) -> String {
    match e {
        DecodeError::TooShort { .. } => format!(
            "{e} - the bundle's ABI offsets point past the payload end, so the hex was likely \
             truncated when copied; fetch the complete calldata and retry"
        ),
        e => e.to_string(),
    }
}

fn print_json(v: &serde_json::Value) {
    println!("{}", serde_json::to_string_pretty(v).unwrap());
}

fn parse_hex(s: &str) -> Result<Vec<u8>, String> {
    let s = s.trim().trim_start_matches("0x");
    hex::decode(s).map_err(|e| format!("invalid hex: {e}"))
}

fn parse_addr(s: &str) -> Result<Address, String> {
    let b = parse_hex(s)?;
    if b.len() != 20 {
        return Err(format!("address must be 20 bytes, got {}", b.len()));
    }
    let mut a = [0u8; 20];
    a.copy_from_slice(&b);
    Ok(a)
}

fn parse_word(s: &str) -> Result<Word, String> {
    let b = parse_hex(s)?;
    if b.len() != 32 {
        return Err(format!("expected 32 bytes, got {}", b.len()));
    }
    let mut w = [0u8; 32];
    w.copy_from_slice(&b);
    Ok(w)
}
