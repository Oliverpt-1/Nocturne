# Examples

Midnight offers live off-chain: a maker builds offers, hashes them into a Merkle tree, and signs
the root once; a taker lifts any offer in the tree by submitting it with the signature and a
proof. This SDK covers everything up to the transaction - building, pricing, signing, validating,
simulating, and encoding/decoding the calldata.

Every example runs offline - dummy keys and addresses, or real production calldata checked into
the test fixtures. Nothing touches a network:

```sh
cargo run --example <name>
```

## Which example do I need?

| I want to... | Example | Key API |
|---|---|---|
| Build, sign, and verify one offer | [`quickstart`](quickstart.rs) | `MarketBuilder`, `OfferBuilder`, `OfferTree`, `tree_digest`, `verify` |
| Quote a book of offers by APR, then re-quote | [`quote_book`](quote_book.rs) | `OfferBuilder::apr`, `tick_to_apr`, `encode_cancel_root_calldata` |
| Take an offer: validate, size, simulate, encode | [`take_offer`](take_offer.rs) | `validate_offer`, `seller_assets_to_units`, `simulate_take`, `encode_take_calldata` |
| Decode on-chain calldata into typed views | [`read_state`](read_state.rs) | `decode_bundle_calldata`, `decode_set_is_root_ratified_calldata` |
| Time the hash → tree → sign pipeline | [`bench`](bench.rs) | `hash_offer`, `OfferTree`, `sign_digest` |

The full API reference is `cargo doc --open`. The live end-to-end harness (real contracts on
anvil) lives in [`../e2e/`](../e2e/), not here.

## Build, sign, and verify one offer

The whole maker path in one screen: a market, a "lend at 7.2% APR" offer, a one-leaf tree, a
signature, and proof that the on-chain ratifier would accept it.

```rust
let market = MarketBuilder::new(1, [0x11; 20], [0x22; 20])
    .collateral([0x33; 20], U256::from(770_000_000_000_000_000u64), U256::from(1u64), [0x44; 20])
    .maturity(2_000_000_000)
    .build();
let offer = OfferBuilder::new(market, maker)
    .lend()
    .apr(7.2, 1_700_000_000)
    .expiry(2_000_000_000)
    .ratifier(ratifier)
    .max_units(1_000_000)
    .build_checked()
    .expect("valid offer");

let tree = OfferTree::build(vec![hash_offer(&offer)]).unwrap();
let digest = tree_digest(tree.root(), tree.height(), chain_id, &ratifier);
let sig = signer.sign_digest(&digest).unwrap();
assert!(verify(&offer, &tree.root(), 0, &tree.proof(0), &sig, chain_id, &ratifier, &maker));
```

```sh
cargo run --example quickstart
```

## Quote a book of offers by APR

A ladder of lend offers priced in APR terms, one tree, **one signature for the whole book** -
and the re-quote when fair value moves: cancel the old root, sign a fresh ladder.

```rust
// One rung: price by APR (snapped to the tick grid), one group per rung.
OfferBuilder::new(market(), maker)
    .lend()
    .apr(fair_apr + APR_STEP * i as f64, NOW)
    .expiry(MATURITY)
    .group_u64(i)
    .ratifier(ratifier)
    .max_units(MAX_UNITS)
    .build_checked()?

// The whole book under one signature.
let tree = OfferTree::build(offers.iter().map(hash_offer).collect()).unwrap();
let sig = signer.sign_digest(&tree_digest(tree.root(), tree.height(), chain_id, &ratifier))?;

// Fair value moved: cancel the old root on-chain, then sign the new ladder.
let cancel = encode_cancel_root_calldata(&maker, &tree.root());
```

```sh
cargo run --example quote_book
```

## Take an offer

The taker path end to end: check the offer would be accepted, size the take in notional terms,
simulate the exact fill (amounts, fees, position deltas, revert reasons), and encode the `take`
calldata.

```rust
// 1. Would take() accept this offer at all?
let problems = validate_offer(&offer, &ctx);
assert!(problems.is_empty());

// 2. How many units to receive exactly 500_000 loan-token assets?
let units = seller_assets_to_units(&offer, U256::from(500_000u64), NOW, CBPS)?;

// 3. What exactly executes? (buyer pays / seller receives / fee / debt deltas)
let outcome = simulate_take(&offer, units, &sim)?;
assert!(outcome.reverts.is_empty());

// 4. The transaction: signature + Merkle proof travel as ratifier data inside take().
let rd = encode_ratifier_data(&sig, &tree.root(), 0, &tree.proof(0));
let calldata = encode_take_calldata(&offer, &rd, units, &TAKER, &TAKER, &[0u8; 20], &[]);
```

```sh
cargo run --example take_offer
```

## Decode on-chain calldata

The read side, on two real Base-mainnet transactions from the test fixtures: a taker's bundle
fill decoded into its offers, and the maker's ratification of the root that covers them.

```rust
let bundle = decode_bundle_calldata(&bundle_tx_bytes)?;
for fill in &bundle.fills {
    // fill.offer, fill.units, fill.ratifier_data.root(), ...
}

let ratify = decode_set_is_root_ratified_calldata(&ratify_tx_bytes)?;
assert_eq!(*bundle.fills[0].ratifier_data.root(), ratify.root);
```

```sh
cargo run --example read_state
```

## Benchmark the re-quote pipeline

Times the loop a market maker repeats on every price move - hash N leaves, build the tree,
generate every proof, sign the root - serial vs `rayon`-parallel.

```sh
cargo run --release --example bench
```
