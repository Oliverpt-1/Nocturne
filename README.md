# Nocturne

Off-chain Rust tooling for [Morpho Midnight](https://github.com/morpho-org/midnight) — a
byte-for-byte mirror of Midnight's EIP-712 offer signing and `take` math. Every hash, signature,
price, and calldata layout is parity-checked against the contracts (`cargo test`).

```toml
[dependencies]
nocturne-offers = { path = "crates/nocturne-offers" }
```

```rust
use nocturne_offers::*;
```

`Word = [u8; 32]`, `Address = [u8; 20]`. On-chain `uint256`s are big-endian `Word`s; typed APIs
take `U256`/`u64`/`u128` and convert for you. Snippets below assume `sk`, `chain_id: Word`, and
`ratifier: Address` are in scope.

---

## Build offers

Typed builders that pack the raw wire fields for you.

```rust
let market = MarketBuilder::new(1, midnight_addr, loan_token)
    .collateral(collat, U256::from(770_000_000_000_000_000u64), U256::from(1u64), oracle)
    .maturity(2_000_000_000)
    .build();

let offer = OfferBuilder::new(market, maker)
    .buy().tick(8).expiry(2_000_000_000).ratifier(ratifier).max_units(1_000_000)
    .build();                       // or .try_build(&ctx)? to validate first
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
cargo run --release --bin bench
```

---

```sh
cargo test              # everything, incl. parity vs the Midnight contracts
```
