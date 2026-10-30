//! Human-readable (and `--json`) rendering of decoded Midnight payloads.
//!
//! Everything here is presentation only: it turns the opaque 32-byte words into checksummed
//! addresses, dates, prices and APRs, and surfaces the security-critical fields (chain id,
//! ratifier / verifying contract, maker, expiry, caps) so a human can eyeball what a signature
//! actually commits to.

use nocturne::*;
use serde_json::{json, Value};

// ---- primitives --------------------------------------------------------------

/// `0x`-prefixed lowercase hex.
pub fn hex_bytes(b: &[u8]) -> String {
    format!("0x{}", hex::encode(b))
}

/// EIP-55 checksummed address.
pub fn checksum(addr: &Address) -> String {
    let lower = hex::encode(addr); // 40 hex chars, no prefix
    let hash = keccak(lower.as_bytes());
    let mut out = String::with_capacity(42);
    out.push_str("0x");
    for (i, c) in lower.chars().enumerate() {
        if c.is_ascii_alphabetic() {
            let byte = hash[i / 2];
            let nibble = if i % 2 == 0 { byte >> 4 } else { byte & 0x0f };
            if nibble >= 8 {
                out.push(c.to_ascii_uppercase());
            } else {
                out.push(c);
            }
        } else {
            out.push(c);
        }
    }
    out
}

/// Canonical JSON representation for an EVM address.
fn address_json(addr: &Address) -> Value {
    Value::String(checksum(addr))
}

/// Canonical JSON representation for opaque bytes.
fn bytes_json(bytes: &[u8]) -> Value {
    Value::String(hex_bytes(bytes))
}

/// Format a Unix timestamp as `YYYY-MM-DD HH:MM:SS UTC` (proleptic Gregorian, no leap seconds).
pub fn fmt_ts(ts: u64) -> String {
    let days = (ts / 86_400) as i64;
    let rem = ts % 86_400;
    let (y, m, d) = civil_from_days(days);
    let (hh, mm, ss) = (rem / 3600, (rem % 3600) / 60, rem % 60);
    format!("{y:04}-{m:02}-{d:02} {hh:02}:{mm:02}:{ss:02} UTC")
}

/// Days since 1970-01-01 -> (year, month, day). Howard Hinnant's `civil_from_days`.
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097; // [0, 146096]
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365; // [0, 399]
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // [0, 365]
    let mp = (5 * doy + 2) / 153; // [0, 11]
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32; // [1, 31]
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32; // [1, 12]
    (y + if m <= 2 { 1 } else { 0 }, m, d)
}

/// A `Word` holding a small-enough tick as `u64`.
fn tick_of(w: &Word) -> Option<u64> {
    word_to_u128(w).and_then(|v| u64::try_from(v).ok())
}

/// A WAD-scaled price (1e18 == 1.0) rendered as a decimal, falling back to the raw integer.
fn price_str(tick: u64) -> String {
    match tick_to_price(tick) {
        Ok(p) => match word_to_u128(&u256_to_word(p)) {
            Some(v) => format!("{:.6}", v as f64 / 1e18),
            None => format!("{p} (raw)"),
        },
        Err(e) => format!("(price error: {e})"),
    }
}

/// APR implied by a tick, given an optional reference `now` and the market maturity.
fn apr_str(tick: u64, now: Option<u64>, maturity: Option<u64>) -> String {
    match (now, maturity) {
        (Some(n), Some(maturity)) if n < maturity => match tick_to_apr(tick, maturity - n) {
            Ok(a) => format!("{:.4}% (ttm {} days)", a, (maturity - n) / 86_400),
            Err(e) => format!("(apr error: {e})"),
        },
        (Some(_), Some(_)) => "(already matured)".to_string(),
        (Some(_), None) => "(maturity too large)".to_string(),
        (None, _) => "(pass --now <unix> to compute)".to_string(),
    }
}

/// Render a large numeric value without losing precision in text output or JSON.
///
/// JSON consumers receive decimal strings for all integer values. This keeps values larger than
/// JavaScript's safe integer range lossless and is also the representation used by the existing
/// CLI output.
fn decimal_text(value: impl std::fmt::Display) -> String {
    value.to_string()
}

fn decimal_json(value: impl std::fmt::Display) -> Value {
    Value::String(decimal_text(value))
}

fn u256_text(value: &U256) -> String {
    decimal_text(value)
}

fn word_text(word: &Word) -> String {
    u256_text(&word_to_u256(word))
}

fn u256_json(value: &U256) -> Value {
    decimal_json(value)
}

fn word_json(word: &Word) -> Value {
    u256_json(&word_to_u256(word))
}

fn u64_of(w: &Word) -> Option<u64> {
    word_to_u128(w).and_then(|v| u64::try_from(v).ok())
}

// ---- text renderers ----------------------------------------------------------

/// Preformatted shared market values. Payload renderers own the field ordering and contextual
/// labels while this helper keeps the underlying address, byte, and numeric formatting canonical.
/// `maturity` is a Unix timestamp; collateral amounts elsewhere in the report remain token-native
/// base units, while percentage-like fields such as `lltv` are WAD values (1e18 == 100%). Other
/// protocol integers, including `rcfThreshold`, remain raw decimal values.
struct MarketText {
    chain_id: String,
    midnight: String,
    loan_token: String,
    maturity: String,
    rcf_threshold: String,
    enter_gate: String,
    liquidator_gate: String,
    collateral_params: Vec<String>,
}

fn market_text(m: &Market) -> MarketText {
    MarketText {
        chain_id: word_text(&m.chain_id),
        midnight: checksum(&m.midnight),
        loan_token: checksum(&m.loan_token),
        maturity: u256_ts(&word_to_u256(&m.maturity)),
        rcf_threshold: word_text(&m.rcf_threshold),
        enter_gate: checksum(&m.enter_gate),
        liquidator_gate: checksum(&m.liquidator_gate),
        collateral_params: m
            .collateral_params
            .iter()
            .map(|cp| {
                format!(
                    "token={} lltv={} cursor={} oracle={}",
                    checksum(&cp.token),
                    word_text(&cp.lltv),
                    word_text(&cp.liquidation_cursor),
                    checksum(&cp.oracle),
                )
            })
            .collect(),
    }
}

/// Human-readable description of a permit. The opaque permit bytes are intentionally summarized
/// by length here; JSON output retains the complete bytes through [`permit_json`].
fn permit_text(p: &TokenPermit) -> String {
    let kind = match p.kind {
        0 => "none".to_string(),
        1 => "ERC2612".to_string(),
        2 => "Permit2".to_string(),
        k => format!("unknown({k})"),
    };
    if p.data.is_empty() {
        kind
    } else {
        format!("{kind} ({} bytes)", p.data.len())
    }
}

/// Human-readable description of a collateral withdrawal. Assets are token-native base units.
fn withdrawal_text(w: &CollateralWithdrawal) -> String {
    format!(
        "index={} assets={}",
        u256_text(&w.collateral_index),
        u256_text(&w.assets)
    )
}

/// Human-readable description of a collateral supply, including its transfer permit.
fn supply_text(c: &CollateralSupply) -> String {
    format!(
        "index={} assets={} permit={}",
        u256_text(&c.collateral_index),
        u256_text(&c.assets),
        permit_text(&c.permit)
    )
}

/// Render a decoded offer as an indented, human-readable block.
pub fn offer_text(o: &Offer, now: Option<u64>) -> String {
    let mut s = String::new();
    let side = if o.buy {
        "BUY (maker lends)"
    } else {
        "SELL (maker borrows)"
    };
    let maturity = u64_of(&o.market.maturity);
    let tick = tick_of(&o.tick);
    let market = market_text(&o.market);

    s.push_str("  ── security-critical ──────────────────────────────\n");
    s.push_str(&format!("  chain id            : {}\n", market.chain_id));
    s.push_str(&format!("  midnight (contract) : {}\n", market.midnight));
    s.push_str(&format!("  maker (signer)      : {}\n", checksum(&o.maker)));
    s.push_str(&format!(
        "  ratifier (verifier) : {}\n",
        checksum(&o.ratifier)
    ));
    s.push_str(&format!(
        "  expiry              : {} ({})\n",
        word_text(&o.expiry),
        u64_of(&o.expiry)
            .map(fmt_ts)
            .unwrap_or_else(|| "(large)".into())
    ));
    match nocturne::active_cap(o) {
        Some(Cap::Units(v)) => s.push_str(&format!("  cap                 : {v} units\n")),
        Some(Cap::Assets(v)) => s.push_str(&format!("  cap                 : {v} assets\n")),
        None => {
            s.push_str("  cap                 : INVALID (both/neither maxUnits & maxAssets set)\n")
        }
    }
    s.push_str("  ── terms ──────────────────────────────────────────\n");
    s.push_str(&format!("  side                : {side}\n"));
    match tick {
        Some(t) => {
            s.push_str(&format!("  tick                : {t}\n"));
            s.push_str(&format!("  price               : {}\n", price_str(t)));
            s.push_str(&format!(
                "  apr                 : {}\n",
                apr_str(t, now, maturity)
            ));
        }
        None => s.push_str(&format!(
            "  tick                : {} (oversized)\n",
            word_text(&o.tick)
        )),
    }
    s.push_str(&format!(
        "  start               : {} ({})\n",
        word_text(&o.start),
        u64_of(&o.start)
            .map(fmt_ts)
            .unwrap_or_else(|| "(large)".into())
    ));
    s.push_str(&format!("  maturity            : {}\n", market.maturity));
    s.push_str(&format!(
        "  group               : {}\n",
        hex_bytes(&o.group)
    ));
    s.push_str(&format!("  loan token          : {}\n", market.loan_token));
    s.push_str(&format!("  reduce only         : {}\n", o.reduce_only));
    s.push_str(&format!(
        "  continuous fee cap  : {}\n",
        word_text(&o.continuous_fee_cap)
    ));
    if o.callback != [0u8; 20] {
        s.push_str(&format!(
            "  callback            : {}\n",
            checksum(&o.callback)
        ));
    }
    if !o.callback_data.is_empty() {
        s.push_str(&format!(
            "  callback data       : {}\n",
            hex_bytes(&o.callback_data)
        ));
    }
    if o.receiver_if_maker_is_seller != [0u8; 20] {
        s.push_str(&format!(
            "  receiver (maker sell): {}\n",
            checksum(&o.receiver_if_maker_is_seller)
        ));
    }
    s.push_str(&format!(
        "  collateral params   : {}\n",
        market.collateral_params.len()
    ));
    for (i, params) in market.collateral_params.iter().enumerate() {
        s.push_str(&format!("    [{i}] {params}\n"));
    }
    s
}

/// Render decoded EcrecoverRatifier data (signature, root, leaf index, proof).
pub fn ratifier_text(rd: &RatifierData) -> String {
    let mut s = String::new();
    s.push_str(&format!(
        "  root                : {}\n",
        hex_bytes(&rd.root)
    ));
    s.push_str(&format!("  leaf index          : {}\n", rd.leaf_index));
    s.push_str(&format!("  tree height         : {}\n", rd.proof.len()));
    s.push_str(&format!("  signature v         : {}\n", rd.sig.v));
    s.push_str(&format!(
        "  signature r         : {}\n",
        hex_bytes(&rd.sig.r)
    ));
    s.push_str(&format!(
        "  signature s         : {}\n",
        hex_bytes(&rd.sig.s)
    ));
    for (i, p) in rd.proof.iter().enumerate() {
        s.push_str(&format!("  proof[{i}]            : {}\n", hex_bytes(p)));
    }
    s
}

/// Render decoded SetterRatifier data (root, leaf index, proof - no signature).
pub fn setter_ratifier_text(rd: &SetterRatifierData) -> String {
    let mut s = String::new();
    s.push_str(&format!(
        "  root                : {}\n",
        hex_bytes(&rd.root)
    ));
    s.push_str(&format!("  leaf index          : {}\n", rd.leaf_index));
    s.push_str(&format!("  tree height         : {}\n", rd.proof.len()));
    s.push_str(
        "  signature           : (none - SetterRatifier; the maker ratifies the root on-chain)\n",
    );
    for (i, p) in rd.proof.iter().enumerate() {
        s.push_str(&format!("  proof[{i}]            : {}\n", hex_bytes(p)));
    }
    s
}

/// Render ratifier data in either layout.
pub fn ratifier_payload_text(rd: &RatifierPayload) -> String {
    match rd {
        RatifierPayload::Ecrecover(rd) => ratifier_text(rd),
        RatifierPayload::Setter(rd) => setter_ratifier_text(rd),
    }
}

/// Render a decoded `take` call.
pub fn take_text(t: &TakeCall, now: Option<u64>) -> String {
    let mut s = String::new();
    s.push_str("offer:\n");
    s.push_str(&offer_text(&t.offer, now));
    s.push_str("ratifier data:\n");
    s.push_str(&ratifier_payload_text(&t.ratifier_data));
    s.push_str("taker args:\n");
    s.push_str(&format!(
        "  units               : {}\n",
        decimal_text(t.units)
    ));
    s.push_str(&format!("  taker               : {}\n", checksum(&t.taker)));
    s.push_str(&format!(
        "  receiver (taker sell): {}\n",
        checksum(&t.receiver_if_taker_is_seller)
    ));
    if t.taker_callback != [0u8; 20] {
        s.push_str(&format!(
            "  taker callback      : {}\n",
            checksum(&t.taker_callback)
        ));
    }
    if !t.taker_callback_data.is_empty() {
        s.push_str(&format!(
            "  taker callback data : {}\n",
            hex_bytes(&t.taker_callback_data)
        ));
    }
    s
}

/// Render a decoded repay-and-withdraw position-management call.
pub fn repay_withdraw_text(r: &RepayWithdrawCall) -> String {
    let mut s = String::new();
    let market = market_text(&r.market);
    s.push_str("repay and withdraw:\n");
    s.push_str(&format!("  chain id            : {}\n", market.chain_id));
    s.push_str(&format!("  midnight            : {}\n", market.midnight));
    s.push_str(&format!("  loan token          : {}\n", market.loan_token));
    s.push_str(&format!("  maturity            : {}\n", market.maturity));
    s.push_str(&format!(
        "  rcf threshold       : {}\n",
        market.rcf_threshold
    ));
    s.push_str(&format!("  enter gate          : {}\n", market.enter_gate));
    s.push_str(&format!(
        "  liquidator gate     : {}\n",
        market.liquidator_gate
    ));
    s.push_str(&format!(
        "  collateral params   : {}\n",
        market.collateral_params.len()
    ));
    for (i, params) in market.collateral_params.iter().enumerate() {
        s.push_str(&format!("    [{i}] {params}\n"));
    }
    s.push_str(&format!(
        "  repay assets        : {}\n",
        u256_text(&r.repay_assets)
    ));
    s.push_str(&format!(
        "  on behalf           : {}\n",
        checksum(&r.on_behalf)
    ));
    s.push_str(&format!(
        "  loan token permit   : {}\n",
        permit_text(&r.loan_token_permit)
    ));
    s.push_str(&format!(
        "  collateral withdrawals : {}\n",
        r.collateral_withdrawals.len()
    ));
    for (i, withdrawal) in r.collateral_withdrawals.iter().enumerate() {
        s.push_str(&format!("    [{i}] {}\n", withdrawal_text(withdrawal)));
    }
    s.push_str(&format!(
        "  collateral receiver : {}\n",
        checksum(&r.collateral_receiver)
    ));
    s.push_str(&format!(
        "  referral fee pct    : {}\n",
        u256_text(&r.referral_fee_pct)
    ));
    if r.referral_fee_pct != U256::ZERO {
        s.push_str(&format!(
            "  referral recipient  : {}\n",
            checksum(&r.referral_fee_recipient)
        ));
    }
    s.push_str(&format!(
        "  deadline            : {}\n",
        u256_ts(&r.deadline)
    ));
    s
}

/// A `U256` with a timestamp reading when it fits in `u64`.
fn u256_ts(v: &U256) -> String {
    match u64::try_from(*v) {
        Ok(ts) => format!("{} ({})", u256_text(v), fmt_ts(ts)),
        Err(_) => format!("{} (large)", u256_text(v)),
    }
}

/// Render a bundle's wrapper arguments (everything except the fills themselves).
///
/// These are taker-side execution bounds, NOT covered by any maker signature - rendered so a
/// taker can eyeball what their own transaction does around the fills.
pub fn bundle_summary_text(b: &BundleCall) -> String {
    let mut s = String::new();
    s.push_str(&format!("bundle ({}):\n", b.kind.function_name()));
    s.push_str(&format!("  fills               : {}\n", b.fills.len()));
    s.push_str(&format!("  taker               : {}\n", checksum(&b.taker)));
    s.push_str(&format!("  reduce only         : {}\n", b.reduce_only));
    s.push_str(&format!(
        "  {:<20}: {}\n",
        b.kind.target_label(),
        u256_text(&b.target)
    ));
    s.push_str(&format!(
        "  {:<20}: {}\n",
        b.kind.limit_label(),
        u256_text(&b.limit)
    ));
    match &b.side {
        BundleSide::Buy {
            loan_token_permit,
            collateral_withdrawals,
            collateral_receiver,
        } => {
            s.push_str(&format!(
                "  loan token permit   : {}\n",
                permit_text(loan_token_permit)
            ));
            s.push_str(&format!(
                "  collateral withdrawals : {}\n",
                collateral_withdrawals.len()
            ));
            for (i, w) in collateral_withdrawals.iter().enumerate() {
                s.push_str(&format!("    [{i}] {}\n", withdrawal_text(w)));
            }
            s.push_str(&format!(
                "  collateral receiver : {}\n",
                checksum(collateral_receiver)
            ));
        }
        BundleSide::Sell {
            receiver,
            collateral_supplies,
        } => {
            s.push_str(&format!("  receiver            : {}\n", checksum(receiver)));
            s.push_str(&format!(
                "  collateral supplies : {}\n",
                collateral_supplies.len()
            ));
            for (i, c) in collateral_supplies.iter().enumerate() {
                s.push_str(&format!("    [{i}] {}\n", supply_text(c)));
            }
        }
    }
    s.push_str(&format!(
        "  referral fee pct    : {}\n",
        u256_text(&b.referral_fee_pct)
    ));
    if b.referral_fee_pct != U256::ZERO {
        s.push_str(&format!(
            "  referral recipient  : {}\n",
            checksum(&b.referral_fee_recipient)
        ));
    }
    s.push_str(&format!(
        "  max continuous fee  : {}\n",
        u256_text(&b.max_continuous_fee)
    ));
    s.push_str(&format!(
        "  deadline            : {}\n",
        u256_ts(&b.deadline)
    ));
    s
}

/// Render one embedded fill (offer + ratifier data + units).
pub fn fill_text(fill: &OfferFill, now: Option<u64>) -> String {
    let mut s = String::new();
    s.push_str("offer:\n");
    s.push_str(&offer_text(&fill.offer, now));
    s.push_str("ratifier data:\n");
    s.push_str(&ratifier_payload_text(&fill.ratifier_data));
    s.push_str(&format!(
        "  units               : {}\n",
        decimal_text(fill.units)
    ));
    s
}

/// Render a decoded bundle call: wrapper summary followed by every fill.
pub fn bundle_text(b: &BundleCall, now: Option<u64>) -> String {
    let mut s = bundle_summary_text(b);
    for (i, fill) in b.fills.iter().enumerate() {
        s.push_str(&format!("\nfill[{i}]:\n"));
        s.push_str(&fill_text(fill, now));
    }
    s
}

// ---- JSON builders -----------------------------------------------------------

/// Canonical JSON representation of a token permit.
fn permit_json(p: &TokenPermit) -> Value {
    json!({
        "kind": p.kind,
        "data": bytes_json(&p.data),
    })
}

/// Canonical JSON representation of a collateral withdrawal.
///
/// `assets` is a token-native amount, not a human-unit decimal.
fn withdrawal_json(w: &CollateralWithdrawal) -> Value {
    json!({
        "collateralIndex": u256_json(&w.collateral_index),
        "assets": u256_json(&w.assets),
    })
}

fn withdrawals_json(withdrawals: &[CollateralWithdrawal]) -> Value {
    Value::Array(withdrawals.iter().map(withdrawal_json).collect())
}

/// Canonical JSON representation of a collateral supply.
fn supply_json(c: &CollateralSupply) -> Value {
    json!({
        "collateralIndex": u256_json(&c.collateral_index),
        "assets": u256_json(&c.assets),
        "permit": permit_json(&c.permit),
    })
}

fn supplies_json(supplies: &[CollateralSupply]) -> Value {
    Value::Array(supplies.iter().map(supply_json).collect())
}

/// Canonical JSON representation of a market shared by offers and position actions.
fn market_json(m: &Market) -> Value {
    json!({
        "chainId": word_json(&m.chain_id),
        "midnight": address_json(&m.midnight),
        "loanToken": address_json(&m.loan_token),
        "maturity": word_json(&m.maturity),
        "rcfThreshold": word_json(&m.rcf_threshold),
        "enterGate": address_json(&m.enter_gate),
        "liquidatorGate": address_json(&m.liquidator_gate),
        "collateralParams": m.collateral_params.iter().map(|cp| json!({
            "token": address_json(&cp.token),
            "lltv": word_json(&cp.lltv),
            "liquidationCursor": word_json(&cp.liquidation_cursor),
            "oracle": address_json(&cp.oracle),
        })).collect::<Vec<_>>(),
    })
}

pub fn offer_json(o: &Offer) -> Value {
    json!({
        "market": market_json(&o.market),
        "buy": o.buy,
        "maker": address_json(&o.maker),
        "start": word_json(&o.start),
        "expiry": word_json(&o.expiry),
        "tick": word_json(&o.tick),
        "group": bytes_json(&o.group),
        "callback": address_json(&o.callback),
        "callbackData": bytes_json(&o.callback_data),
        "receiverIfMakerIsSeller": address_json(&o.receiver_if_maker_is_seller),
        "ratifier": address_json(&o.ratifier),
        "reduceOnly": o.reduce_only,
        "maxUnits": decimal_json(o.max_units),
        "maxAssets": decimal_json(o.max_assets),
        "continuousFeeCap": word_json(&o.continuous_fee_cap),
    })
}

pub fn ratifier_json(rd: &RatifierData) -> Value {
    json!({
        "type": "ecrecover",
        "signature": { "v": rd.sig.v, "r": bytes_json(&rd.sig.r), "s": bytes_json(&rd.sig.s) },
        "root": bytes_json(&rd.root),
        "leafIndex": rd.leaf_index,
        "treeHeight": rd.proof.len(),
        "proof": rd.proof.iter().map(|p| bytes_json(p)).collect::<Vec<_>>(),
    })
}

pub fn setter_ratifier_json(rd: &SetterRatifierData) -> Value {
    json!({
        "type": "setter",
        "root": bytes_json(&rd.root),
        "leafIndex": rd.leaf_index,
        "treeHeight": rd.proof.len(),
        "proof": rd.proof.iter().map(|p| bytes_json(p)).collect::<Vec<_>>(),
    })
}

pub fn ratifier_payload_json(rd: &RatifierPayload) -> Value {
    match rd {
        RatifierPayload::Ecrecover(rd) => ratifier_json(rd),
        RatifierPayload::Setter(rd) => setter_ratifier_json(rd),
    }
}

pub fn take_json(t: &TakeCall) -> Value {
    json!({
        "offer": offer_json(&t.offer),
        "ratifierData": ratifier_payload_json(&t.ratifier_data),
        "units": u256_json(&t.units),
        "taker": address_json(&t.taker),
        "receiverIfTakerIsSeller": address_json(&t.receiver_if_taker_is_seller),
        "takerCallback": address_json(&t.taker_callback),
        "takerCallbackData": bytes_json(&t.taker_callback_data),
    })
}

pub fn repay_withdraw_json(r: &RepayWithdrawCall) -> Value {
    json!({
        "function": "midnightBundlesV1RepayAndWithdrawCollateral",
        "market": market_json(&r.market),
        "repayAssets": u256_json(&r.repay_assets),
        "onBehalf": address_json(&r.on_behalf),
        "loanTokenPermit": permit_json(&r.loan_token_permit),
        "collateralWithdrawals": withdrawals_json(&r.collateral_withdrawals),
        "collateralReceiver": address_json(&r.collateral_receiver),
        "referralFeePct": u256_json(&r.referral_fee_pct),
        "referralFeeRecipient": address_json(&r.referral_fee_recipient),
        "deadline": u256_json(&r.deadline),
    })
}

pub fn bundle_json(b: &BundleCall) -> Value {
    let side = match &b.side {
        BundleSide::Buy {
            loan_token_permit,
            collateral_withdrawals,
            collateral_receiver,
        } => json!({
            "loanTokenPermit": permit_json(loan_token_permit),
            "collateralWithdrawals": withdrawals_json(collateral_withdrawals),
            "collateralReceiver": address_json(collateral_receiver),
        }),
        BundleSide::Sell {
            receiver,
            collateral_supplies,
        } => json!({
            "receiver": address_json(receiver),
            "collateralSupplies": supplies_json(collateral_supplies),
        }),
    };
    json!({
        "function": b.kind.function_name(),
        "target": u256_json(&b.target),
        "limit": u256_json(&b.limit),
        "taker": address_json(&b.taker),
        "reduceOnly": b.reduce_only,
        "side": side,
        "fills": b.fills.iter().map(|f| json!({
            "offer": offer_json(&f.offer),
            "ratifierData": ratifier_payload_json(&f.ratifier_data),
            "units": u256_json(&f.units),
        })).collect::<Vec<_>>(),
        "referralFeePct": u256_json(&b.referral_fee_pct),
        "referralFeeRecipient": address_json(&b.referral_fee_recipient),
        "maxContinuousFee": u256_json(&b.max_continuous_fee),
        "deadline": u256_json(&b.deadline),
    })
}

pub fn cancel_json(maker: &Address, root: &Word) -> Value {
    json!({
        "maker": address_json(maker),
        "root": bytes_json(root),
    })
}

pub fn ratify_json(r: &RatifyCall) -> Value {
    json!({
        "maker": address_json(&r.maker),
        "root": bytes_json(&r.root),
        "ratified": r.ratified,
    })
}

pub fn market_state_json(m: &MarketStateView) -> Value {
    json!({
        "totalUnits": decimal_json(m.total_units),
        "lossFactor": decimal_json(m.loss_factor),
        "lossFactorMaxed": m.loss_factor == u128::MAX,
        "withdrawable": decimal_json(m.withdrawable),
        "continuousFeeCredit": decimal_json(m.continuous_fee_credit),
        "settlementFeeCbp": m.settlement_fee_cbp,
        "continuousFee": m.continuous_fee,
        "tickSpacing": m.tick_spacing,
    })
}

pub fn position_json(p: &PositionView) -> Value {
    json!({
        "credit": decimal_json(p.credit),
        "pendingFee": decimal_json(p.pending_fee),
        "lastLossFactor": decimal_json(p.last_loss_factor),
        "lastAccrual": decimal_json(p.last_accrual),
        "debt": decimal_json(p.debt),
        "collateralBitmap": decimal_json(p.collateral_bitmap),
    })
}

pub fn market_state_text(m: &MarketStateView) -> String {
    format!(
        "  total units         : {}\n  loss factor         : {}{}\n  withdrawable        : {}\n  continuous fee credit: {}\n  settlement fee cbp  : {:?}\n  continuous fee      : {}\n  tick spacing        : {}\n",
        m.total_units,
        m.loss_factor,
        if m.loss_factor == u128::MAX { " (MAXED OUT)" } else { "" },
        m.withdrawable,
        m.continuous_fee_credit,
        m.settlement_fee_cbp,
        m.continuous_fee,
        m.tick_spacing,
    )
}

pub fn position_text(p: &PositionView) -> String {
    format!(
        "  credit              : {}\n  pending fee         : {}\n  last loss factor    : {}\n  last accrual        : {} ({})\n  debt                : {}\n  collateral bitmap   : {:#x}\n",
        p.credit,
        p.pending_fee,
        p.last_loss_factor,
        p.last_accrual,
        fmt_ts(p.last_accrual as u64),
        p.debt,
        p.collateral_bitmap,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn checksum_matches_eip55_examples() {
        // Canonical EIP-55 vectors.
        let a1 = hex::decode("5aAeb6053F3E94C9b9A09f33669435E7Ef1BeAed").unwrap();
        let mut addr = [0u8; 20];
        addr.copy_from_slice(&a1);
        assert_eq!(
            checksum(&addr),
            "0x5aAeb6053F3E94C9b9A09f33669435E7Ef1BeAed"
        );

        let a2 = hex::decode("fB6916095ca1df60bB79Ce92cE3Ea74c37c5d359").unwrap();
        addr.copy_from_slice(&a2);
        assert_eq!(
            checksum(&addr),
            "0xfB6916095ca1df60bB79Ce92cE3Ea74c37c5d359"
        );
    }

    #[test]
    fn timestamp_formats() {
        assert_eq!(fmt_ts(0), "1970-01-01 00:00:00 UTC");
        assert_eq!(fmt_ts(1_700_000_000), "2023-11-14 22:13:20 UTC");
    }

    #[test]
    fn oversized_words_are_not_truncated_to_u64() {
        let oversized = word_from_u128(u64::MAX as u128 + 1);
        assert_eq!(u64_of(&oversized), None);
        assert_eq!(tick_of(&oversized), None);
        assert_eq!(apr_str(3372, Some(0), None), "(maturity too large)");
    }

    #[test]
    fn shared_json_renderers_use_lossless_canonical_values() {
        let permit = TokenPermit {
            kind: 2,
            data: vec![0xde, 0xad],
        };
        assert_eq!(permit_json(&permit), json!({"kind": 2, "data": "0xdead"}));
        assert_eq!(permit_text(&permit), "Permit2 (2 bytes)");

        let withdrawal = CollateralWithdrawal {
            collateral_index: U256::from(3u64),
            assets: U256::from(4_000u64),
        };
        assert_eq!(
            withdrawal_json(&withdrawal),
            json!({"collateralIndex": "3", "assets": "4000"})
        );
        assert_eq!(withdrawal_text(&withdrawal), "index=3 assets=4000");

        let supply = CollateralSupply {
            collateral_index: U256::from(5u64),
            assets: U256::from(6_000u64),
            permit: permit.clone(),
        };
        assert_eq!(
            supply_json(&supply),
            json!({
                "collateralIndex": "5",
                "assets": "6000",
                "permit": {"kind": 2, "data": "0xdead"},
            })
        );
        assert_eq!(
            supply_text(&supply),
            "index=5 assets=6000 permit=Permit2 (2 bytes)"
        );

        let large = U256::from(u128::MAX) + U256::from(1u64);
        assert_eq!(u256_json(&large), Value::String(large.to_string()));
        assert_eq!(
            decimal_json(u128::MAX),
            Value::String(u128::MAX.to_string())
        );
    }

    #[test]
    fn market_text_and_json_share_the_same_fields() {
        let market = Market {
            chain_id: word_from_u64(31_337),
            midnight: [0x11; 20],
            loan_token: [0x22; 20],
            collateral_params: vec![CollateralParams {
                token: [0x33; 20],
                lltv: word_from_u128(770_000_000_000_000_000),
                liquidation_cursor: word_from_u128(300_000_000_000_000_000),
                oracle: [0x44; 20],
            }],
            maturity: word_from_u64(4_000_000_000),
            rcf_threshold: word_from_u64(1_000),
            enter_gate: [0x55; 20],
            liquidator_gate: [0x66; 20],
        };

        let value = market_json(&market);
        assert_eq!(value["chainId"], "31337");
        assert_eq!(value["maturity"], "4000000000");
        assert_eq!(value["collateralParams"][0]["lltv"], "770000000000000000");

        let text = market_text(&market);
        assert_eq!(text.chain_id, "31337");
        assert_eq!(text.maturity, "4000000000 (2096-10-02 07:06:40 UTC)");
        assert_eq!(text.collateral_params[0], "token=0x3333333333333333333333333333333333333333 lltv=770000000000000000 cursor=300000000000000000 oracle=0x4444444444444444444444444444444444444444");
    }

    #[test]
    fn bundle_json_uses_shared_side_renderers() {
        let withdrawal = CollateralWithdrawal {
            collateral_index: U256::from(3u64),
            assets: U256::from(4_000u64),
        };
        let permit = TokenPermit {
            kind: 1,
            data: vec![0xaa],
        };
        let buy = BundleCall {
            kind: BundleKind::BuyWithUnitsTarget,
            target: U256::from(1u64),
            limit: U256::from(2u64),
            taker: [0x11; 20],
            reduce_only: false,
            side: BundleSide::Buy {
                loan_token_permit: permit.clone(),
                collateral_withdrawals: vec![withdrawal],
                collateral_receiver: [0x22; 20],
            },
            fills: Vec::new(),
            referral_fee_pct: U256::ZERO,
            referral_fee_recipient: [0u8; 20],
            max_continuous_fee: U256::ZERO,
            deadline: U256::from(10u64),
        };
        let buy_json = bundle_json(&buy);
        assert_eq!(
            buy_json["side"]["loanTokenPermit"],
            json!({"kind": 1, "data": "0xaa"})
        );
        assert_eq!(
            buy_json["side"]["collateralWithdrawals"][0],
            json!({"collateralIndex": "3", "assets": "4000"})
        );

        let sell = BundleCall {
            kind: BundleKind::SellWithUnitsTarget,
            side: BundleSide::Sell {
                receiver: [0x33; 20],
                collateral_supplies: vec![CollateralSupply {
                    collateral_index: U256::from(5u64),
                    assets: U256::from(6_000u64),
                    permit,
                }],
            },
            ..buy
        };
        let sell_json = bundle_json(&sell);
        assert_eq!(
            sell_json["side"]["collateralSupplies"][0],
            json!({
                "collateralIndex": "5",
                "assets": "6000",
                "permit": {"kind": 1, "data": "0xaa"},
            })
        );
    }
}
