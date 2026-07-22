# nocturne-verify

**An offline decoder and signature/Merkle-root verifier for Morpho Midnight offer payloads.**

Midnight offers are deep, heavily-nested payloads. When you sign one, your wallet shows you a
32-byte digest (or a wall of hex) that you cannot independently check. `nocturne-verify` closes
that gap: entirely offline, it reproduces the two things that matter and shows them to you in
plain terms:

1. **What the bytes say** — decode a `take` payload / offer / ratifier blob into readable fields
   (tick → price & APR, timestamps → dates, checksummed addresses), with the security-critical
   fields (chain id, Midnight contract, maker, ratifier, expiry, caps) surfaced first.
2. **What the signature commits to** — reproduce the Merkle root and the EIP-712 digest, and
   confirm the signature recovers to the intended maker.

It holds **no keys** and makes **no network calls**. It is a deliberately independent
reimplementation, [parity-checked against the Midnight contracts](../nocturne/fixtures/README.md),
so it can catch a bug — or a tampered field — in whatever produced the payload.

## Install

```sh
# from a checkout of this repo
cargo install --path crates/nocturne-verify

# or run without installing
cargo run -p nocturne-verify -- <command> ...
```

Prebuilt macOS/Linux binaries are attached to each [GitHub release](https://github.com/Oliverpt-1/Nocturne/releases) for users without a Rust toolchain.

## Commands

### `verify` — check a payload you're about to submit or that was handed to you

From raw `take` calldata, reproduce the signed Merkle root and confirm the signature recovers to
the offer's maker. Prints a per-check PASS/FAIL and exits non-zero if anything fails.

```sh
nocturne-verify verify 0x6a14c9ef... --chain-id 31337
# optionally assert the signer:
nocturne-verify verify 0x6a14c9ef... --chain-id 31337 --expected-maker 0xYourMaker
```

The chain id defaults to the offer's own `market.chainId`; pass `--chain-id` to also assert they
agree. Output ends with either:

```
RESULT: PASS - this signature authorizes exactly the offer shown above.
```

or a `FAIL` with the failing checks listed.

### `decode` — read any payload in plain terms

```sh
nocturne-verify decode 0x6a14c9ef...            # auto-detects take / offer / cancelRoot
nocturne-verify decode 0x... --type market-state # getter returns have no selector; name the type
nocturne-verify decode 0x... --now 1700000000    # compute APR against a reference time
nocturne-verify decode 0x... --json              # machine-readable output
```

Types: `take`, `offer`, `ratifier`, `cancel`, `market-state`, `position`.

### `digest` — reproduce the digest from the terms *you* intend to sign

Supply the offer(s) you mean to sign as JSON (serialized `Offer`, one file per leaf, in order).
The tool prints each leaf hash, the Merkle root, the EIP-712 domain separator, and the final
digest — so you can compare against what your wallet displays.

```sh
nocturne-verify digest offer.json --chain-id 31337 --ratifier 0xRatifier
# assert the wallet's digest matches your terms (exit non-zero on mismatch):
nocturne-verify digest offer.json --chain-id 31337 --expect 0xWalletDigest
# emit full EIP-712 typed data for an eth_signTypedData_v4 wallet:
nocturne-verify digest offer.json --chain-id 31337 --eip712
```

`--chain-id` and `--ratifier` default to the first offer's `market.chainId` and `ratifier`.

## Two workflows — and which one is stronger

**A. Verify the app's payload (`verify`).** Fast and catches the common failures: a malformed
offer, a signature over different terms, a wrong maker. Use it on every payload.

**B. Independent cross-check (`digest --expect`).** *Stronger.* You feed in the terms you intend to
sign, sourced from **your own records — not the app**, and compare the digest the tool computes
against what your wallet shows. It does not trust the payload producer at all.

> **Why B is stronger:** `verify` re-decodes bytes the app produced. If the app itself is fully
> compromised it could hand a self-consistent lie to both the wallet and the verifier. The
> `digest` cross-check breaks that loop by taking the terms from you, so the only thing that has to
> be correct is your own intent and this tool. For the highest assurance, run `digest` on a
> separate machine from the one building and submitting the payload.

## Guarantees & scope

- **Offline.** No RPC, no network, no key material — safe to run on an air-gapped machine.
- **Read-only.** It never signs and never submits; it only decodes and verifies.
- **Contract-anchored.** Every hash, digest, price, and calldata layout mirrors the Midnight
  contracts and is parity-checked against blobs the contracts emit (see the
  [fixtures](../nocturne/fixtures/README.md)).
- **Not** a substitute for an on-chain simulation of `take`, and **not** security-audited. Verify
  against your own deployment before relying on it with real value.

## License

MIT OR Apache-2.0, same as the workspace.
