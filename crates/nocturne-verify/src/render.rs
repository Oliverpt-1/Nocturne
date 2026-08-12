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
    word_to_u128(w).map(|v| v as u64)
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
fn apr_str(tick: u64, now: Option<u64>, maturity: u64) -> String {
    match now {
        Some(n) if n < maturity => match tick_to_apr(tick, maturity - n) {
            Ok(a) => format!("{:.4}% (ttm {} days)", a, (maturity - n) / 86_400),
            Err(e) => format!("(apr error: {e})"),
        },
        Some(_) => "(already matured)".to_string(),
        None => "(pass --now <unix> to compute)".to_string(),
    }
}

fn u256_dec(w: &Word) -> String {
    word_to_u256(w).to_string()
}

fn u64_of(w: &Word) -> Option<u64> {
    word_to_u128(w).map(|v| v as u64)
}

// ---- text renderers ----------------------------------------------------------

/// Render a decoded offer as an indented, human-readable block.
pub fn offer_text(o: &Offer, now: Option<u64>) -> String {
    let mut s = String::new();
    let side = if o.buy {
        "BUY (maker lends)"
    } else {
        "SELL (maker borrows)"
    };
    let maturity = u64_of(&o.market.maturity).unwrap_or(0);
    let tick = tick_of(&o.tick);

    s.push_str("  ── security-critical ──────────────────────────────\n");
    s.push_str(&format!(
        "  chain id            : {}\n",
        u256_dec(&o.market.chain_id)
    ));
    s.push_str(&format!(
        "  midnight (contract) : {}\n",
        checksum(&o.market.midnight)
    ));
    s.push_str(&format!("  maker (signer)      : {}\n", checksum(&o.maker)));
    s.push_str(&format!(
        "  ratifier (verifier) : {}\n",
        checksum(&o.ratifier)
    ));
    s.push_str(&format!(
        "  expiry              : {} ({})\n",
        u256_dec(&o.expiry),
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
            u256_dec(&o.tick)
        )),
    }
    s.push_str(&format!(
        "  start               : {} ({})\n",
        u256_dec(&o.start),
        u64_of(&o.start)
            .map(fmt_ts)
            .unwrap_or_else(|| "(large)".into())
    ));
    s.push_str(&format!(
        "  maturity            : {} ({})\n",
        u256_dec(&o.market.maturity),
        u64_of(&o.market.maturity)
            .map(fmt_ts)
            .unwrap_or_else(|| "(large)".into())
    ));
    s.push_str(&format!(
        "  group               : {}\n",
        hex_bytes(&o.group)
    ));
    s.push_str(&format!(
        "  loan token          : {}\n",
        checksum(&o.market.loan_token)
    ));
    s.push_str(&format!("  reduce only         : {}\n", o.reduce_only));
    s.push_str(&format!(
        "  continuous fee cap  : {}\n",
        u256_dec(&o.continuous_fee_cap)
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
        o.market.collateral_params.len()
    ));
    for (i, cp) in o.market.collateral_params.iter().enumerate() {
        s.push_str(&format!(
            "    [{i}] token={} lltv={} cursor={} oracle={}\n",
            checksum(&cp.token),
            u256_dec(&cp.lltv),
            u256_dec(&cp.liquidation_cursor),
            checksum(&cp.oracle),
        ));
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
    s.push_str(&format!("  units               : {}\n", t.units));
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

/// Human label for a `TokenPermit`.
fn permit_str(p: &TokenPermit) -> String {
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

/// A `U256` with a timestamp reading when it fits in `u64`.
fn u256_ts(v: &U256) -> String {
    match u64::try_from(*v) {
        Ok(ts) => format!("{v} ({})", fmt_ts(ts)),
        Err(_) => format!("{v} (large)"),
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
    s.push_str(&format!("  {:<20}: {}\n", b.kind.target_label(), b.target));
    s.push_str(&format!("  {:<20}: {}\n", b.kind.limit_label(), b.limit));
    match &b.side {
        BundleSide::Buy {
            loan_token_permit,
            collateral_withdrawals,
            collateral_receiver,
        } => {
            s.push_str(&format!(
                "  loan token permit   : {}\n",
                permit_str(loan_token_permit)
            ));
            s.push_str(&format!(
                "  collateral withdrawals : {}\n",
                collateral_withdrawals.len()
            ));
            for (i, w) in collateral_withdrawals.iter().enumerate() {
                s.push_str(&format!(
                    "    [{i}] index={} assets={}\n",
                    w.collateral_index, w.assets
                ));
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
                s.push_str(&format!(
                    "    [{i}] index={} assets={} permit={}\n",
                    c.collateral_index,
                    c.assets,
                    permit_str(&c.permit)
                ));
            }
        }
    }
    s.push_str(&format!("  referral fee pct    : {}\n", b.referral_fee_pct));
    if b.referral_fee_pct != U256::ZERO {
        s.push_str(&format!(
            "  referral recipient  : {}\n",
            checksum(&b.referral_fee_recipient)
        ));
    }
    s.push_str(&format!(
        "  max continuous fee  : {}\n",
        b.max_continuous_fee
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
    s.push_str(&format!("  units               : {}\n", fill.units));
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

pub fn offer_json(o: &Offer) -> Value {
    json!({
        "market": {
            "chainId": u256_dec(&o.market.chain_id),
            "midnight": checksum(&o.market.midnight),
            "loanToken": checksum(&o.market.loan_token),
            "maturity": u256_dec(&o.market.maturity),
            "rcfThreshold": u256_dec(&o.market.rcf_threshold),
            "enterGate": checksum(&o.market.enter_gate),
            "liquidatorGate": checksum(&o.market.liquidator_gate),
            "collateralParams": o.market.collateral_params.iter().map(|cp| json!({
                "token": checksum(&cp.token),
                "lltv": u256_dec(&cp.lltv),
                "liquidationCursor": u256_dec(&cp.liquidation_cursor),
                "oracle": checksum(&cp.oracle),
            })).collect::<Vec<_>>(),
        },
        "buy": o.buy,
        "maker": checksum(&o.maker),
        "start": u256_dec(&o.start),
        "expiry": u256_dec(&o.expiry),
        "tick": u256_dec(&o.tick),
        "group": hex_bytes(&o.group),
        "callback": checksum(&o.callback),
        "callbackData": hex_bytes(&o.callback_data),
        "receiverIfMakerIsSeller": checksum(&o.receiver_if_maker_is_seller),
        "ratifier": checksum(&o.ratifier),
        "reduceOnly": o.reduce_only,
        "maxUnits": o.max_units.to_string(),
        "maxAssets": o.max_assets.to_string(),
        "continuousFeeCap": u256_dec(&o.continuous_fee_cap),
    })
}

pub fn ratifier_json(rd: &RatifierData) -> Value {
    json!({
        "type": "ecrecover",
        "signature": { "v": rd.sig.v, "r": hex_bytes(&rd.sig.r), "s": hex_bytes(&rd.sig.s) },
        "root": hex_bytes(&rd.root),
        "leafIndex": rd.leaf_index,
        "treeHeight": rd.proof.len(),
        "proof": rd.proof.iter().map(|p| hex_bytes(p)).collect::<Vec<_>>(),
    })
}

pub fn setter_ratifier_json(rd: &SetterRatifierData) -> Value {
    json!({
        "type": "setter",
        "root": hex_bytes(&rd.root),
        "leafIndex": rd.leaf_index,
        "treeHeight": rd.proof.len(),
        "proof": rd.proof.iter().map(|p| hex_bytes(p)).collect::<Vec<_>>(),
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
        "units": t.units.to_string(),
        "taker": checksum(&t.taker),
        "receiverIfTakerIsSeller": checksum(&t.receiver_if_taker_is_seller),
        "takerCallback": checksum(&t.taker_callback),
        "takerCallbackData": hex_bytes(&t.taker_callback_data),
    })
}

pub fn bundle_json(b: &BundleCall) -> Value {
    let side = match &b.side {
        BundleSide::Buy {
            loan_token_permit,
            collateral_withdrawals,
            collateral_receiver,
        } => json!({
            "loanTokenPermit": { "kind": loan_token_permit.kind, "data": hex_bytes(&loan_token_permit.data) },
            "collateralWithdrawals": collateral_withdrawals.iter().map(|w| json!({
                "collateralIndex": w.collateral_index.to_string(),
                "assets": w.assets.to_string(),
            })).collect::<Vec<_>>(),
            "collateralReceiver": checksum(collateral_receiver),
        }),
        BundleSide::Sell {
            receiver,
            collateral_supplies,
        } => json!({
            "receiver": checksum(receiver),
            "collateralSupplies": collateral_supplies.iter().map(|c| json!({
                "collateralIndex": c.collateral_index.to_string(),
                "assets": c.assets.to_string(),
                "permit": { "kind": c.permit.kind, "data": hex_bytes(&c.permit.data) },
            })).collect::<Vec<_>>(),
        }),
    };
    json!({
        "function": b.kind.function_name(),
        "target": b.target.to_string(),
        "limit": b.limit.to_string(),
        "taker": checksum(&b.taker),
        "reduceOnly": b.reduce_only,
        "side": side,
        "fills": b.fills.iter().map(|f| json!({
            "offer": offer_json(&f.offer),
            "ratifierData": ratifier_payload_json(&f.ratifier_data),
            "units": f.units.to_string(),
        })).collect::<Vec<_>>(),
        "referralFeePct": b.referral_fee_pct.to_string(),
        "referralFeeRecipient": checksum(&b.referral_fee_recipient),
        "maxContinuousFee": b.max_continuous_fee.to_string(),
        "deadline": b.deadline.to_string(),
    })
}

pub fn cancel_json(maker: &Address, root: &Word) -> Value {
    json!({
        "maker": checksum(maker),
        "root": hex_bytes(root),
    })
}

pub fn ratify_json(r: &RatifyCall) -> Value {
    json!({
        "maker": checksum(&r.maker),
        "root": hex_bytes(&r.root),
        "ratified": r.ratified,
    })
}

pub fn market_state_json(m: &MarketStateView) -> Value {
    json!({
        "totalUnits": m.total_units.to_string(),
        "lossFactor": m.loss_factor.to_string(),
        "lossFactorMaxed": m.loss_factor == u128::MAX,
        "withdrawable": m.withdrawable.to_string(),
        "continuousFeeCredit": m.continuous_fee_credit.to_string(),
        "settlementFeeCbp": m.settlement_fee_cbp,
        "continuousFee": m.continuous_fee,
        "tickSpacing": m.tick_spacing,
    })
}

pub fn position_json(p: &PositionView) -> Value {
    json!({
        "credit": p.credit.to_string(),
        "pendingFee": p.pending_fee.to_string(),
        "lastLossFactor": p.last_loss_factor.to_string(),
        "lastAccrual": p.last_accrual.to_string(),
        "debt": p.debt.to_string(),
        "collateralBitmap": p.collateral_bitmap.to_string(),
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
}
