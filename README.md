# Nocturne

Off-chain Rust tooling for [Morpho Midnight](https://github.com/morpho-org/midnight).

`nocturne-offers` is a byte-for-byte mirror of Midnight's EIP-712 offer-tree signing
(`HashLib.sol` + `EcrecoverRatifier`): build offers, hash them, build the Merkle tree and
proofs, sign the root, recover/verify signatures, validate offers against the `take` rules,
and simulate a take locally — all in native code, across cores.

```toml
[dependencies]
nocturne-offers = { path = "crates/nocturne-offers" }
```

```rust
use nocturne_offers::*;
```

Types: `Word = [u8; 32]`, `Address = [u8; 20]`. On the wire, `uint256` fields (`chain_id`,
`tick`, `maturity`, …) are `Word` (big-endian) so hashing is byte-exact; the builder and
simulator work in `U256` (re-exported from `ruint`, the primitive alloy/reth/foundry use) and
convert for you (`u256_to_word` / `word_to_u256` / `word_to_u128`). The tools below assume
`offers: Vec<Offer>`, `sk: SigningKey`, `chain_id: Word`, and `ratifier: Address` are in scope.

---

## Building offers

`OfferBuilder` / `MarketBuilder` take typed inputs (`u64`/`u128`/`U256`/`Address`), apply
defaults, and pack the wire `Word`s. `try_build` runs `validate_offer` so a malformed offer
never leaves the builder.

```rust
let market = MarketBuilder::new(1, midnight_addr, loan_token)
    .collateral(collat_token, U256::from(860_000_000_000_000_000u64), U256::from(1u64), oracle)
    .maturity(2_000_000_000)
    .build();

let offer = OfferBuilder::new(market, maker)
    .buy()                     // or .sell()
    .tick(8)
    .expiry(2_000_000_000)
    .ratifier(ratifier)
    .max_units(1_000_000)      // exactly one of max_units / max_assets
    .try_build(&ctx)?;         // Result<Offer, Vec<OfferError>>; or .build() to skip validation
```

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
`ContinuousFeeAboveOfferCap`, `ConsumedUnits`, `ConsumedAssets`, `SelfTake`,
`CannotIncreaseDebtPostMaturity`, `MakerCreditOrDebtIncreased`.

## Take simulation

"If a taker lifts this offer for N units, what executes?" — a local port of the `Midnight.take`
math (`TickLib.tickToPrice`, `settlementFee`, buyer/seller assets, position deltas). Use
`tick_to_price` / `settlement_fee` / `take_amounts` for just the pricing, or `simulate_take` for
the full outcome plus locally computable revert reasons.

```rust
let price = tick_to_price(8)?;                       // WAD, == TickLib.tickToPrice
let amounts = take_amounts(&offers[0], U256::from(1_000u64), now_ts, cbps)?;
// amounts.buyer_assets / seller_assets / settlement_fee_assets

let out = simulate_take(&offers[0], U256::from(1_000u64), &SimCtx {
    now: now_ts,
    market: SimMarket { tick_spacing: 4, continuous_fee: 100, settlement_fee_cbp: cbps, loss_factor_maxed: false },
    consumed: consumed_so_far,
    maker_position: Position::default(),
    taker_position: Position::default(),
    taker_is_maker: false,
})?;
assert!(out.reverts.is_empty());                     // else: why `take` would revert
// out.buyer_credit_increase / seller_debt_increase / new_consumed / ...
```

Scope: the deterministic economic outcome and the take-time reverts. Out of scope (needs live
chain reads / external calls): gate checks, borrower health, ratifier/authorization, and position
slashing + fee accrual — positions passed in are assumed already updated.

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

Everything is checked against the contract (`cargo test`):

1. **Typehashes** — `tests/parity.rs`: each computed typehash equals the `HashLib.sol` constant
   (`COLLATERAL_PARAMS`, `MARKET`, `OFFER`, all 21 `offerTreeTypeHash` heights).
2. **End-to-end signing** — `tests/parity_e2e.rs`: a 4-offer tree's leaf / root / digest /
   recovered signer all match the on-chain values from `fixtures/GenEndToEnd.t.sol`, which drives
   the real `EcrecoverRatifier.isRatified` and confirms it *accepts* the signature.
3. **Tick prices** — `tests/sim_parity.rs`: `tick_to_price` matches `TickLib.tickToPrice`
   (vectors from `fixtures/GenSim.t.sol`) across the tick range.
4. **Take math** — `tests/sim_take_parity.rs`: `settlement_fee` / `take_amounts` / `simulate_take`
   reproduce the amounts and position deltas of a **real** `Midnight.take`, run end-to-end through
   the full contract by `fixtures/GenTake.t.sol`.
5. **ethers** — the bench and the ethers baseline print the same root to the byte.

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
  src/convert.rs             # Word <-> U256/u128/u64
  src/builder.rs             # OfferBuilder / MarketBuilder
  src/validate.rs            # validate_offer + consumption helpers
  src/sim.rs                 # tick_to_price, settlement_fee, take_amounts, simulate_take
  src/bin/bench.rs           # benchmark
  tests/parity.rs            # typehash parity vs HashLib.sol
  tests/parity_e2e.rs        # end-to-end signing parity vs the real EcrecoverRatifier
  tests/sim_parity.rs        # tick_to_price parity vs TickLib
  tests/sim_take_parity.rs   # take math parity vs a real Midnight.take
  tests/builder.rs           # builder coverage
  tests/validate.rs          # validate_offer coverage
  tests/sim.rs               # simulator coverage
  fixtures/GenEndToEnd.t.sol # Solidity generator for the signing vectors
  fixtures/GenSim.t.sol      # Solidity generator for the tick-price vectors
  fixtures/GenTake.t.sol     # Solidity generator (real take) for the take-math vectors
bench-js/                    # ethers v6 baseline
```
