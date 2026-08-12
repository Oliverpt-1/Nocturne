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
    /// Verify a `take`, bundle, or `setIsRootRatified` payload: reproduce the Merkle root(s)
    /// and confirm each authorization. Exits non-zero if any check fails.
    Verify {
        /// The calldata as hex.
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
        /// For a `setIsRootRatified` payload: your intended offer JSON files (order sets leaf
        /// indices). The payload's root must commit to exactly these offers.
        #[arg(long, num_args = 1..)]
        offers: Vec<std::path::PathBuf>,
    },
    /// Verify an EIP-712 typed-data document (the `eth_signTypedData_v4` JSON a wallet asks a
    /// maker to sign): decode every offer, prove the document is the canonical encoding of
    /// those offers, and reproduce the digest. Exits non-zero if any check fails.
    VerifyTyped {
        /// Path to the typed-data JSON file.
        file: std::path::PathBuf,
        /// Reference Unix time, used to compute APR in the printed terms.
        #[arg(long)]
        now: Option<u64>,
        /// Also assert the leaves equal these intended offer JSON files (in order; remaining
        /// leaves must be zero-offer padding).
        #[arg(long, num_args = 1..)]
        offers: Vec<std::path::PathBuf>,
        /// Assert the computed digest equals this 32-byte hex value.
        #[arg(long)]
        expect: Option<String>,
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
        /// Assert the computed Merkle root equals this 32-byte hex value (the SetterRatifier
        /// flow: the root shown in a setIsRootRatified prompt); exit non-zero otherwise.
        #[arg(long)]
        expect_root: Option<String>,
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
    Ratify,
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
            offers,
        } => cmd_verify(&payload, chain_id, expected_maker.as_deref(), now, &offers),
        Command::VerifyTyped {
            file,
            now,
            offers,
            expect,
        } => cmd_verify_typed(&file, now, &offers, expect.as_deref()),
        Command::Digest {
            offers,
            chain_id,
            ratifier,
            expect,
            expect_root,
            eip712,
        } => cmd_digest(
            &offers,
            chain_id,
            ratifier.as_deref(),
            expect.as_deref(),
            expect_root.as_deref(),
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
        PayloadType::Ratify => {
            let r = decode_set_is_root_ratified_calldata(&bytes).map_err(|e| e.to_string())?;
            if json {
                print_json(&render::ratify_json(&r));
            } else {
                println!("Decoded setIsRootRatified(...) payload\n");
                println!("  maker  : {}", render::checksum(&r.maker));
                println!("  root   : {}", render::hex_bytes(&r.root));
                println!(
                    "  action : {}",
                    if r.ratified {
                        "RATIFY - authorize every offer under this root"
                    } else {
                        "UNRATIFY - revoke every offer under this root"
                    }
                );
            }
        }
        PayloadType::Cancel => {
            let (maker, root) = decode_cancel_root_calldata(&bytes).map_err(|e| e.to_string())?;
            if json {
                print_json(&render::cancel_json(&maker, &root));
            } else {
                println!("Decoded cancelRoot(...) payload\n");
                println!("  maker : {}", render::checksum(&maker));
                println!("  root  : {}", render::hex_bytes(&root));
            }
        }
        PayloadType::MarketState => {
            let m = decode_market_state(&bytes).map_err(|e| e.to_string())?;
            if json {
                print_json(&render::market_state_json(&m));
            } else {
                println!("Decoded marketState return\n");
                print!("{}", render::market_state_text(&m));
            }
        }
        PayloadType::Position => {
            let p = decode_position(&bytes).map_err(|e| e.to_string())?;
            if json {
                print_json(&render::position_json(&p));
            } else {
                println!("Decoded position return\n");
                print!("{}", render::position_text(&p));
            }
        }
    }
    Ok(ExitCode::SUCCESS)
}

fn cmd_verify(
    payload: &str,
    chain_id: Option<u64>,
    expected_maker: Option<&str>,
    now: Option<u64>,
    offers: &[std::path::PathBuf],
) -> Result<ExitCode, String> {
    let bytes = parse_hex(payload)?;
    if bytes.len() >= 4 {
        let sel = [bytes[0], bytes[1], bytes[2], bytes[3]];
        if BundleKind::from_selector(sel).is_some() {
            return cmd_verify_bundle(&bytes, chain_id, expected_maker, now);
        }
        if sel == SET_IS_ROOT_RATIFIED_SELECTOR {
            return cmd_verify_ratify(&bytes, offers);
        }
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
    } else {
        if fills == 1 {
            println!("RESULT: PASS - this signature authorizes exactly the offer shown above.");
        } else {
            println!(
                "RESULT: PASS - all {fills} fill signatures authorize exactly the offers shown \
                 above."
            );
        }
        println!(
            "        The tool has proven the payload matches the terms shown; only YOU can \
             check the terms match the trade you intend. Read them before proceeding."
        );
        ExitCode::SUCCESS
    }
}

/// Verify a maker's `setIsRootRatified` transaction: decode it and, when the maker's intended
/// offers are supplied, prove the root commits to exactly those offers - the maker-side
/// anti-blind-signing check (the payload alone is a bare 32-byte root the wallet cannot
/// explain).
fn cmd_verify_ratify(bytes: &[u8], offer_paths: &[std::path::PathBuf]) -> Result<ExitCode, String> {
    let r = decode_set_is_root_ratified_calldata(bytes).map_err(|e| e.to_string())?;

    println!("Verifying SetterRatifier ratification payload\n");
    println!("  maker               : {}", render::checksum(&r.maker));
    println!("  root                : {}", render::hex_bytes(&r.root));
    println!(
        "  action              : {}",
        if r.ratified {
            "RATIFY - authorize every offer under this root"
        } else {
            "UNRATIFY - revoke every offer under this root"
        }
    );

    if offer_paths.is_empty() {
        println!(
            "\n  [INFO] a bare root cannot be verified by itself; pass --offers <your offer \
             JSON files>\n         to prove this root commits to exactly your intended offers"
        );
        println!(
            "\nRESULT: PARTIAL - payload decoded, but the root was not checked against your \
             intended terms."
        );
        return Ok(ExitCode::from(2));
    }

    let offers = load_offers(offer_paths)?;
    let leaves: Vec<Word> = offers.iter().map(hash_offer).collect();
    let tree = OfferTree::build(leaves).map_err(|e| format!("tree: {e:?}"))?;

    println!("\nchecks:");
    let mut ok = true;

    // Every offer in a tree must share the payload's maker: setIsRootRatified binds
    // isRootRatified[maker][root], so an offer with a different maker can never ratify.
    check(
        &mut ok,
        offers.iter().all(|o| o.maker == r.maker),
        "every offer's maker equals the payload's maker",
    );

    let root_matches = tree.root() == r.root;
    check(
        &mut ok,
        root_matches,
        "root commits to exactly the supplied offers",
    );
    println!(
        "      computed root     : {}",
        render::hex_bytes(&tree.root())
    );
    println!(
        "      offers committed  : {} (tree height {})",
        offers.len(),
        tree.height()
    );

    println!();
    if ok {
        println!(
            "RESULT: PASS - this transaction {} exactly the {} offer(s) you supplied, and \
             nothing else.",
            if r.ratified { "authorizes" } else { "revokes" },
            offers.len()
        );
        Ok(ExitCode::SUCCESS)
    } else {
        println!("RESULT: FAIL - do NOT sign; the root does not correspond to your terms.");
        Ok(ExitCode::FAILURE)
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

/// Verify the EIP-712 typed data a wallet asks a maker to sign (the "off-chain signing" flow):
/// parse the offers out, prove the document is byte-for-byte the canonical encoding of those
/// offers (so the wallet's hash equals the digest computed here), and show every term.
fn cmd_verify_typed(
    file: &std::path::Path,
    now: Option<u64>,
    offer_paths: &[std::path::PathBuf],
    expect: Option<&str>,
) -> Result<ExitCode, String> {
    let text = std::fs::read_to_string(file).map_err(|e| format!("{}: {e}", file.display()))?;
    let doc: serde_json::Value = serde_json::from_str(&text).map_err(|e| e.to_string())?;
    let parsed = eip712::parse_typed_data(&doc)?;

    let zero_offer = |o: &Offer| o.maker == [0u8; 20];
    let real: Vec<&Offer> = parsed.offers.iter().filter(|o| !zero_offer(o)).collect();

    println!("Verifying EIP-712 typed-data payload (maker off-chain signing)\n");
    println!("  chain id            : {}", word_to_u256(&parsed.chain_id));
    println!(
        "  ratifier (verifier) : {}",
        render::checksum(&parsed.ratifier)
    );
    println!(
        "  offers              : {} real, {} zero padding (tree height {})",
        real.len(),
        parsed.offers.len() - real.len(),
        parsed.height
    );

    for (i, offer) in parsed.offers.iter().enumerate() {
        if zero_offer(offer) {
            println!("\nleaf[{i}]: (zero-offer padding - maker is 0x0, can never be taken)");
        } else {
            println!("\nleaf[{i}]:");
            print!("{}", render::offer_text(offer, now));
        }
    }

    println!("\nchecks:");
    let mut ok = true;

    // The load-bearing check: regenerating the canonical document from the parsed offers must
    // reproduce the input exactly (up to address casing / key order). Then the wallet hashing
    // THIS document produces exactly the digest computed below - no hidden fields, no tampered
    // types table, no misread values.
    let regenerated = eip712::typed_data(
        &parsed.offers,
        parsed.chain_id,
        &parsed.ratifier,
        parsed.height,
    );
    check(
        &mut ok,
        eip712::docs_equivalent(&doc, &regenerated),
        "document is the canonical encoding of the offers shown",
    );

    check(
        &mut ok,
        parsed.height <= MAX_TREE_HEIGHT,
        "tree height is at most 20 (TreeTooHigh)",
    );
    check(
        &mut ok,
        real.iter().all(|o| o.ratifier == parsed.ratifier),
        "every offer's ratifier equals the verifying contract",
    );
    check(
        &mut ok,
        real.iter().all(|o| o.market.chain_id == parsed.chain_id),
        "every offer's market.chainId equals the domain chain id",
    );
    check(
        &mut ok,
        real.windows(2).all(|w| w[0].maker == w[1].maker),
        "every offer shares one maker (the signer)",
    );

    // Optional: assert the real leaves are exactly the maker's intended offers, in order.
    if !offer_paths.is_empty() {
        let intended = load_offers(offer_paths)?;
        let matches =
            real.len() == intended.len() && real.iter().zip(&intended).all(|(a, b)| **a == *b);
        check(
            &mut ok,
            matches,
            "leaves equal the intended offers passed via --offers",
        );
    }

    let leaves: Vec<Word> = parsed.offers.iter().map(hash_offer).collect();
    let tree = OfferTree::build(leaves).map_err(|e| format!("tree: {e:?}"))?;
    let digest = tree_digest(
        tree.root(),
        parsed.height,
        parsed.chain_id,
        &parsed.ratifier,
    );
    println!(
        "      merkle root       : {}",
        render::hex_bytes(&tree.root())
    );
    println!("      DIGEST (to sign)  : {}", render::hex_bytes(&digest));

    if let Some(exp) = expect {
        let exp_word = parse_word(exp)?;
        check(&mut ok, exp_word == digest, "digest equals --expect");
    }

    println!();
    if ok {
        println!(
            "RESULT: PASS - signing this typed data authorizes exactly the {} offer(s) shown \
             above, and nothing else.",
            real.len()
        );
        println!(
            "        The tool has proven the bytes match the terms shown; only YOU can check \
             the terms match your intent. Read them before signing."
        );
        Ok(ExitCode::SUCCESS)
    } else {
        println!("RESULT: FAIL - do NOT sign; see failed checks above.");
        Ok(ExitCode::FAILURE)
    }
}

fn cmd_digest(
    paths: &[std::path::PathBuf],
    chain_id: Option<u64>,
    ratifier: Option<&str>,
    expect: Option<&str>,
    expect_root: Option<&str>,
    eip712: bool,
) -> Result<ExitCode, String> {
    let offers = load_offers(paths)?;

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

    // Parse assertions before the EIP-712 output branch so `--eip712` cannot silently bypass
    // an explicit safety check. Successful EIP-712 output remains pure JSON; mismatches are
    // reported on stderr and return failure without emitting a document.
    let expected_digest = expect.map(parse_word).transpose()?;
    let expected_root = expect_root.map(parse_word).transpose()?;

    if eip712 {
        let mut ok = true;
        if let Some(exp) = expected_digest {
            if exp != digest {
                eprintln!(
                    "MISMATCH: --expect {} != {}",
                    render::hex_bytes(&exp),
                    render::hex_bytes(&digest)
                );
                ok = false;
            }
        }
        if let Some(exp) = expected_root {
            if exp != root {
                eprintln!(
                    "MISMATCH: --expect-root {} != {}",
                    render::hex_bytes(&exp),
                    render::hex_bytes(&root)
                );
                ok = false;
            }
        }
        if !ok {
            return Ok(ExitCode::FAILURE);
        }
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

    let mut code = ExitCode::SUCCESS;
    if let Some(exp_word) = expected_digest {
        println!();
        if exp_word == digest {
            println!("MATCH: the wallet digest matches the intended terms.");
        } else {
            println!(
                "MISMATCH: --expect {} != {}",
                render::hex_bytes(&exp_word),
                render::hex_bytes(&digest)
            );
            println!("Do NOT sign: the digest does not correspond to these terms.");
            code = ExitCode::FAILURE;
        }
    }
    if let Some(exp_word) = expected_root {
        println!();
        if exp_word == root {
            println!("MATCH: the root commits to exactly these offers.");
        } else {
            println!(
                "MISMATCH: --expect-root {} != {}",
                render::hex_bytes(&exp_word),
                render::hex_bytes(&root)
            );
            println!("Do NOT ratify: the root does not correspond to these terms.");
            code = ExitCode::FAILURE;
        }
    }
    Ok(code)
}

// ---- helpers -----------------------------------------------------------------

/// Load serialized `Offer`s from JSON files, one per leaf, in order.
fn load_offers(paths: &[std::path::PathBuf]) -> Result<Vec<Offer>, String> {
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
    Ok(offers)
}

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
        if sel == SET_IS_ROOT_RATIFIED_SELECTOR {
            return Ok(PayloadType::Ratify);
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
