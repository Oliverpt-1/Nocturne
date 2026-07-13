# Nocturne

Off-chain Rust tooling for [Morpho Midnight](https://github.com/morpho-org/midnight).

`nocturne-offers` is a byte-for-byte mirror of Midnight's EIP-712 offer-tree signing
(`HashLib.sol` + `EcrecoverRatifier`): hash offers, build the Merkle tree and proofs, sign
the root, recover/verify signatures, and validate offers against the `take` rules — all
locally, in native code, across cores.

```toml
[dependencies]
nocturne-offers = { path = "crates/nocturne-offers" }
```

```rust
use nocturne_offers::*;
```

Types: `Word = [u8; 32]`, `Address = [u8; 20]`. `uint256` fields (`chain_id`, `tick`,
`maturity`, …) are `Word` (big-endian). The tools below assume `offers: Vec<Offer>`,
`sk: SigningKey`, `chain_id: Word`, and `ratifier: Address` are in scope.

---

## Offer hashing

EIP-712 struct hashes for `Offer` / `Market` / `CollateralParams`. `hash_offer` is the Merkle
leaf. Typehash helpers (`offer_typehash()`, `market_typehash()`, …) compute the on-chain
`HashLib` constants from the type strings.

```rust
let leaf: Word = hash_offer(&offers[0]);           // == HashLib.hashOffer(offer)
let market_hash: Word = hash_market(&offers[0].market);
```

## Offer tree (Merkle)

Perfect binary tree over offer leaves, with per-leaf proofs in `HashLib.isLeaf` order. Takers
need a leaf's proof to lift that offer.

```rust
let leaves: Vec<Word> = offers.iter().map(hash_offer).collect();
let tree = OfferTree::build(leaves);               // leaf count must be a power of two

let root = tree.root();
let height = tree.height();
let proof = tree.proof(0);                          // proof for leaf 0

assert!(verify_leaf(&root, &hash_offer(&offers[0]), 0, &proof));
```

## Signing

Builds the digest the maker signs — one signature covers the whole tree — exactly as
`EcrecoverRatifier` reassembles it (`domain separator` + `offerTreeTypeHash` + `0x1901`).

```rust
let digest = tree_digest(tree.root(), tree.height(), chain_id, &ratifier);
let sig: Sig = sign_digest(&sk, &digest);           // { r, s, v }
let maker: Address = signer_address(&sk);           // the offer.maker to embed
```

## Verify / recover

Off-chain mirror of `EcrecoverRatifier.isRatified`. `recover` returns the signer (`None` on a
malformed signature). `verify` re-hashes the leaf, checks the proof, rebuilds the digest,
recovers, and confirms the maker — i.e. confirms a `take` carrying `(sig, root, index, proof)`
will pass the ratifier.

```rust
assert_eq!(recover(&digest, &sig), Some(maker));

assert!(verify(
    &offers[0], &tree.root(), 0, &tree.proof(0),
    &sig, chain_id, &ratifier, &maker,
));
```

## Policy validation

`validate_offer` mirrors the offer-relevant revert conditions in `Midnight.take` (and the market
checks in `touchMarket`) and returns **every** problem found, so a whole grid can be scrubbed in
one pass. Any `ValidateCtx` field left `None` skips its checks, so it degrades from a stateless
structural check to a full check against a live market snapshot.

```rust
let errors: Vec<OfferError> = validate_offer(&offers[0], &ValidateCtx {
    chain_id: Some(1),
    midnight: Some(midnight_addr),
    now: Some(now_ts),
    market: Some(MarketSnapshot {
        tick_spacing: DEFAULT_TICK_SPACING,
        loss_factor_maxed: false,
        continuous_fee: 100,
    }),
});
assert!(errors.is_empty());               // or: is_valid(&offers[0], &ctx)
```

Consumption caps depend on take size, so they're separate helpers:

```rust
let cap = active_cap(&offers[0]);                              // Cap::Units | Cap::Assets, None if caps invalid
let left = consumption_headroom(&offers[0], consumed_so_far);  // remaining in the group
let ok = can_consume(&offers[0], consumed_so_far, amount);     // does `amount` stay within the cap?
```

`OfferError` variants map to `IMidnight` errors: `InvalidOfferCaps`, `TickOutOfRange`,
`StartAfterExpiry`, `UnusedReceiverMustBeZero`, `NoCollateralParams`, `TooManyCollateralParams`,
`CollateralParamsNotSorted`, `InvalidChainId`, `InvalidMidnight`, `MaturityTooFar`,
`OfferNotStarted`, `OfferExpired`, `TickNotAccessible`, `MarketLossFactorMaxedOut`,
`ContinuousFeeAboveOfferCap`.

## Benchmark (`bin/bench`)

Times one full re-quote cycle — hash `N` leaves → build tree → all proofs → sign root —
single-threaded and parallel (rayon), and cross-checks the root against the ethers baseline.

```sh
cargo run --release --bin bench
```

Apple M5 Pro, 18 cores, best of 20 reps:

| Offers (N) | ethers v6 | Nocturne — 1 core | Nocturne — parallel | speedup |
|-----------:|----------:|------------------:|--------------------:|--------:|
| 1,024      | 302.5 ms  | 4.74 ms           | 1.02 ms             | 297×    |
| 4,096      | 1.22 s    | 18.9 ms           | 3.09 ms             | 396×    |
| 16,384     | 4.90 s    | 75.5 ms           | 9.46 ms             | 517×    |

The signature is O(1) per tree; the cost that scales is keccak hashing every leaf on each
re-quote, which is what the parallel path fans across cores.

---

## Correctness (parity)

Every hash is checked against the contract three ways (`cargo test`):

1. **Typehashes** — `tests/parity.rs`: each computed typehash equals the `HashLib.sol` constant
   (`COLLATERAL_PARAMS`, `MARKET`, `OFFER`, all 21 `offerTreeTypeHash` heights).
2. **End-to-end** — `tests/parity_e2e.rs`: a 4-offer tree's leaf / root / digest / recovered
   signer all match the on-chain values from `fixtures/GenEndToEnd.t.sol`, which drives the real
   `EcrecoverRatifier.isRatified` and confirms it *accepts* the signature.
3. **ethers** — the bench and the ethers baseline print the same root to the byte.

## Run it

```sh
cargo test -q                              # parity + verify + validation
cargo run --release --bin bench            # Nocturne numbers + crosscheck root
cd bench-js && npm i && node bench.js      # ethers baseline (same root)
```

## Layout

```
crates/nocturne-offers/
  src/lib.rs                 # hashing + tree + digest + sign/recover/verify
  src/validate.rs            # validate_offer + consumption helpers
  src/bin/bench.rs           # benchmark
  tests/parity.rs            # typehash parity vs HashLib.sol
  tests/parity_e2e.rs        # end-to-end parity vs the real EcrecoverRatifier
  tests/validate.rs          # validate_offer coverage
  fixtures/GenEndToEnd.t.sol # Solidity generator for the parity_e2e vectors
bench-js/                    # ethers v6 baseline
```
