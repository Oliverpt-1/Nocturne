# fixtures

Authoritative test vectors for `nocturne`, generated from the Midnight contracts.

- `GenEndToEnd.t.sol` - builds a concrete 4-offer tree, computes the leaf / root / signed
  digest exactly as `EcrecoverRatifier.isRatified`, signs it, and asserts the **real** on-chain
  `isRatified` accepts the signature, then prints every value. These printed values are baked
  into `../tests/parity_e2e.rs`.
- `GenSim.t.sol` - prints `TickLib.tickToPrice` for a spread of ticks; the values are baked into
  `../tests/sim_parity.rs`.
- `GenTake.t.sol` - extends the contracts' `BaseTest` and runs a **real** `Midnight.take`, reading
  back the resulting amounts and position deltas; baked into `../tests/sim_take_parity.rs`. Because
  it imports `BaseTest.sol`, drop it in the contracts' `test/` dir to regenerate (as above).

- `GenCodec.t.sol` - emits Solidity's own `abi.encode` / `abi.encodeCall` output for a fixed
  Offer + Signature + proof + take params; baked into `../tests/codec.rs`. The same golden blobs
  also anchor the **decoders** (`decode_take_calldata` / `decode_ratifier_data` /
  `decode_cancel_root_calldata`): `../tests/codec.rs` decodes them back and asserts the exact
  inputs, so the decoders are contract-anchored, not merely inverses of the Rust encoders.

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

If the contracts change the offer layout, the digest changes and `parity_e2e.rs` will fail -
that's the signal to regenerate (and bump the pinned rev here and in `parity_e2e.rs`).
