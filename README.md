# midnight-rust

Off-chain Rust tooling for [Morpho Midnight](https://github.com/morpho-org/midnight).
Internal engine name: **Nocturne**.

First component: `nocturne-offers` — a byte-for-byte mirror of Midnight's EIP-712
offer-tree signing (`HashLib.sol` + `EcrecoverRatifier`), for market makers who
re-price and re-sign large offer grids on every market move.

## Why this exists

Midnight uses Seaport-style bulk signing: a maker builds a Merkle tree of offers and
signs **one** root; takers lift individual offers with a Merkle proof. The ECDSA
signature is O(1) per tree — the cost that scales is the **keccak hashing** of every
nested `Offer` leaf, rebuilt every time the maker re-quotes. That's a pure-CPU,
embarrassingly parallel workload sitting on the competitive critical path (stale-quote
risk / adverse selection). It's exactly where native + multicore beats a JS signer.

## Correctness (parity)

Speed means nothing if the hashes don't match the contract. Parity is proven two ways:

1. **Rust ↔ Solidity** — `tests/parity.rs` asserts every computed typehash equals the
   hardcoded constant in `HashLib.sol` (`COLLATERAL_PARAMS`, `MARKET`, `OFFER`, and all
   21 `offerTreeTypeHash` heights). `cargo test` → 5/5 pass.
2. **Rust ↔ ethers** — the Rust bench and the ethers baseline both print the root of the
   same 4-offer tree, and they are identical to the byte:
   `0x53fe807622c3257be67f3fc456a3585aabc545a739bc99e464ccc52961a68cb8`.

Rust matches ethers, ethers/Rust match the on-chain typehashes — so a signature produced
here will pass `EcrecoverRatifier.isRatified` on-chain.

## Benchmark

### What is measured

One **full re-quote cycle** — the exact work a maker repeats every time the market moves:

1. keccak-hash all `N` `Offer` leaves (EIP-712 struct hashes),
2. build the Merkle tree,
3. generate a Merkle proof for every leaf (takers need these to lift an offer),
4. sign the tree root once (secp256k1).

Both implementations do identical work and produce the identical root (see parity above),
so this is a like-for-like comparison. Timings are the best of 20 reps after warmup.

- **Baseline — ethers v6** (`TypedDataEncoder`), single-threaded. This is how makers sign
  Midnight offers today.
- **Nocturne (this crate)**, reported single-threaded *and* parallel (rayon, across cores).

### Results

Machine: Apple M5 Pro, 18 cores. Reproduce with the commands below.

| Offers (N) | ethers v6 (today) | Nocturne — 1 core | Nocturne — parallel |
|-----------:|------------------:|------------------:|--------------------:|
| 1,024      | 302.5 ms          | 4.74 ms           | 1.02 ms             |
| 4,096      | 1.22 s            | 18.9 ms           | 3.09 ms             |
| 16,384     | 4.90 s            | 75.5 ms           | 9.46 ms             |

**Speedup vs ethers** (higher = faster):

| Offers (N) | Nocturne 1 core | Nocturne parallel |
|-----------:|----------------:|------------------:|
| 1,024      | 64×             | 297×              |
| 4,096      | 65×             | 396×              |
| 16,384     | 65×             | **517×**          |

Per-offer cost: ethers ≈ **299 µs**, Nocturne 1-core ≈ **4.6 µs**, Nocturne parallel ≈ **0.58 µs**.

### How to read this

Refreshing a **16,384-offer** quote grid takes **~4.9 s** with ethers but **~9.5 ms** with
Nocturne. At ~5 s per refresh a maker is signing into prices that have already moved
(adverse selection); at ~10 ms they can re-quote the entire book faster than a block is
produced. That gap is the whole point of the crate.

### Honest caveats

- ethers' `TypedDataEncoder.hashStruct` is the *standard* idiom but not the fastest
  possible JS (it re-resolves types on every call). A hand-optimized JS signer would
  narrow the single-core gap. The **parallel** advantage is structural, though — Node
  can't easily fan keccak across cores, and that's where the ~500× lives.
- ECDSA signing is one call per tree, so it's noise in these numbers; the win is keccak
  hashing throughput.

## Run it

```sh
cargo test -q                              # parity (Rust ↔ Solidity)
cargo run --release --bin bench            # Nocturne numbers + crosscheck root
cd bench-js && npm i && node bench.js      # ethers baseline + crosscheck root (same root)
```

## Layout

```
crates/nocturne-offers/   # EIP-712 offer hashing, Merkle tree/proofs, signing
  src/lib.rs              # the library (mirror of HashLib + EcrecoverRatifier digest)
  src/bin/bench.rs        # Nocturne benchmark
  tests/parity.rs         # typehash parity vs HashLib.sol
bench-js/                 # ethers v6 baseline (how it's done today)
```
