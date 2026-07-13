# fixtures

Authoritative test vectors for `nocturne-offers`, generated from the Midnight contracts.

- `GenEndToEnd.t.sol` — builds a concrete 4-offer tree, computes the leaf / root / signed
  digest exactly as `EcrecoverRatifier.isRatified`, signs it, and asserts the **real** on-chain
  `isRatified` accepts the signature, then prints every value. These printed values are baked
  into `../tests/parity_e2e.rs`.
- `GenSim.t.sol` — prints `TickLib.tickToPrice` for a spread of ticks; the values are baked into
  `../tests/sim_parity.rs`.

This is intentionally **not** part of `cargo test`: the Rust test carries the constants so the
crate builds and tests standalone (same pattern as the typehash constants in `../tests/parity.rs`).

## Regenerate

Pinned against the Midnight contracts at rev `f47568c9e45a9b70830b82a130b47393dcafec33`.

```sh
cp GenEndToEnd.t.sol <midnight-repo>/test/GenEndToEnd.t.sol
cd <midnight-repo> && forge test --match-contract GenEndToEnd -vv
# paste the printed constants into ../tests/parity_e2e.rs, then delete the temporary copy
rm <midnight-repo>/test/GenEndToEnd.t.sol
```

If the contracts change the offer layout, the digest changes and `parity_e2e.rs` will fail —
that's the signal to regenerate (and bump the pinned rev here and in `parity_e2e.rs`).
