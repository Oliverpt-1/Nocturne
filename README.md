<p align="center">
  <a href="https://github.com/Oliverpt-1/midnight-rust/actions/workflows/ci.yml"><img src="https://github.com/Oliverpt-1/midnight-rust/actions/workflows/ci.yml/badge.svg" alt="CI"></a>
  <a href="#license"><img src="https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue.svg" alt="License: MIT OR Apache-2.0"></a>
</p>

<p align="center"><em>Fast off-chain signing and offers for the Morpho Midnight protocol.</em></p>

<p align="center">
  <img src="assets/nocturne.png" alt="Nocturne — made in Rust" width="640">
</p>

## What is Nocturne?

**Nocturne** is a production-ready signing and offer library for the
[Morpho Midnight](https://github.com/morpho-org/midnight) protocol, focused on speed and execution.
It is compatible with all deployments of Midnight. Midnight is an onchain protocol by design;
however, offers are explicitly handled offchain — which is where Nocturne operates. Every hash,
signature, price, and calldata layout is parity-checked byte-for-byte against the contracts
(`cargo test`), and the full lifecycle is exercised against a real deployment on anvil.

Built and operated by Oliver Tipton, and licensed under the Apache-2.0 and MIT licenses.

> **Note:** This project is **not** endorsed by, nor affiliated with, Morpho Labs or its
> subsidiaries.

```toml
[dependencies]
nocturne = { path = "crates/nocturne" }
```

```rust
use nocturne::*;
```

`Word = [u8; 32]`, `Address = [u8; 20]`. On-chain `uint256`s are big-endian `Word`s; typed APIs
take `U256`/`u64`/`u128` and convert for you. Snippets below assume `sk`, `chain_id: Word`, and
`ratifier: Address` are in scope.

---

## Build offers

Typed builders that pack the raw wire fields for you. Quote in ticks/units, or in **APR and
notional** — `.lend()/.borrow()`, `.apr()`, `.assets()` convert for you (APR↔tick is parity-checked
against `TickLib`).

```rust
let market = MarketBuilder::new(1, midnight_addr, loan_token)
    .collateral(collat, U256::from(770_000_000_000_000_000u64), U256::from(1u64), oracle)
    .maturity(2_000_000_000)
    .build();

// low-level: ticks + units
let offer = OfferBuilder::new(market.clone(), maker)
    .buy().tick(8).expiry(2_000_000_000).ratifier(ratifier).max_units(1_000_000)
    .build();                       // or .try_build(&ctx)? to validate first

// human: APR + assets (converts APR->tick, assets->units)
let offer = OfferBuilder::new(market, maker)
    .lend().apr(7.2, now).assets(1_000_000, now, cbps)   // "lend at 7.2% for 1M"
    .expiry(2_000_000_000).ratifier(ratifier)
    .build_checked()?;              // surfaces conversion errors

// or the raw conversions
let tick = apr_to_tick(7.2, ttm_secs, DEFAULT_TICK_SPACING as u64)?;
let apr  = tick_to_apr(tick, ttm_secs)?;   // simple annualized %, snaps to accessible ticks
```

## Hash · tree · proofs

Hash offers into leaves, build the Merkle tree, get per-leaf proofs.

```rust
let leaves: Vec<Word> = offers.iter().map(hash_offer).collect();
let tree = OfferTree::build(leaves)?;      // Result — errors on non-power-of-two
let (root, proof) = (tree.root(), tree.proof(0));
```

## Sign

One signature covers the whole tree. Use a raw key, or the `Signer` trait for KMS/HSM.

```rust
let digest = tree_digest(tree.root(), tree.height(), chain_id, &ratifier);
let sig = sign_digest(&sk, &digest);

// institutional: LocalSigner, or ExternalSigner wrapping a KMS/HSM DER-signing closure
let signer = LocalSigner::from_bytes(&key_bytes)?;
let sig = signer.sign_digest(&digest)?;
let kms = ExternalSigner::new(kms_address, |d| Ok(kms_sign_der(d)));
let sig = kms.sign_digest(&digest)?;
```

## Authorize a hot key

Delegate signing to a hot key (or authorize a ratifier) without your cold account — the signed
`Authorization` that `EcrecoverAuthorizer.setIsAuthorized` consumes.

```rust
let auth = Authorization::new(authorizer, hot_key_addr, true, nonce, deadline);
let sig = sign_authorization(&cold_sk, &auth, chain_id, &authorizer_contract);
```

## Verify · recover

Off-chain mirror of `EcrecoverRatifier.isRatified`.

```rust
let signer = recover(&digest, &sig);            // Option<Address>
let ok = verify(&offers[0], &tree.root(), 0, &tree.proof(0), &sig, chain_id, &ratifier, &maker);
```

## Validate

Will `take` accept this offer? Returns every problem, not just the first.

```rust
let errors = validate_offer(&offers[0], &ValidateCtx {
    chain_id: Some(1),
    now: Some(now_ts),
    market: Some(MarketSnapshot { tick_spacing: 4, loss_factor_maxed: false, continuous_fee: 100 }),
    ..Default::default()
});
```

## Simulate

"If a taker lifts this for N units, what executes?"

```rust
let price = tick_to_price(8)?;                          // WAD
let amounts = take_amounts(&offers[0], U256::from(1_000u64), now_ts, cbps)?;
let out = simulate_take(&offers[0], U256::from(1_000u64), &ctx)?;
// out.buyer_assets / seller_assets / *_credit_increase / new_consumed / reverts
```

Size in notional (assets) instead of units, or find remaining capacity:

```rust
let units = buyer_assets_to_units(&offers[0], U256::from(500_000u64), now_ts, cbps)?;
let left  = consumable_units(&offers[0], consumed, now_ts, cbps)?;
```

## Decode on-chain data

Turn raw bytes (an offer, or an `eth_call` return) into typed Rust.

```rust
let offer  = decode_offer(&abi_bytes)?;
let market = decode_market_state(&ret)?;    // .to_sim_market() / .to_market_snapshot()
let pos    = decode_position(&ret)?;        // .to_sim_position()
let used   = decode_consumed(&ret)?;
```

## Encode calldata

Build the `take` / `cancelRoot` transactions.

```rust
let ratifier_data = encode_ratifier_data(&sig, &tree.root(), 0, &tree.proof(0));
let take_call     = encode_take_calldata(&offers[0], &ratifier_data, units, &taker, &receiver, &cb, &cb_data);
let cancel_call   = encode_cancel_root_calldata(&maker, &tree.root());
```

## Benchmark

Times the re-quote pipeline (hash → tree → proofs → sign) single-threaded and across cores,
against the ethers baseline.

```sh
cargo run --release --example bench
```

---

```sh
cargo test              # everything, incl. parity vs the Midnight contracts
```

## Security

`nocturne` produces signatures and calldata that move real value. See [SECURITY.md](SECURITY.md)
for how to report vulnerabilities privately.

## License

Licensed under either of [Apache License, Version 2.0](LICENSE-APACHE) or
[MIT license](LICENSE-MIT) at your option.
