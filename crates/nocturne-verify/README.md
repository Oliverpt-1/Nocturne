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

From raw `take` calldata — or a `midnightBundlesV1*` bundle wrapping several takes — reproduce
each signed Merkle root and confirm each signature recovers to its offer's maker. Prints a
per-check PASS/FAIL and exits non-zero if anything fails.

```sh
nocturne-verify verify 0x6a14c9ef... --chain-id 31337
# optionally assert the signer:
nocturne-verify verify 0x6a14c9ef... --chain-id 31337 --expected-maker 0xYourMaker
# bundles are auto-detected; every embedded fill is verified, one bad fill fails the bundle:
nocturne-verify verify 0xa85d52e5...
```

The chain id defaults to the offer's own `market.chainId`; pass `--chain-id` to also assert they
agree. Output ends with either:

```
RESULT: PASS - this signature authorizes exactly the offer shown above.
```

or a `FAIL` with the failing checks listed.

### `decode` — read any payload in plain terms

```sh
nocturne-verify decode 0x6a14c9ef...            # auto-detects take / bundle / offer / cancelRoot
nocturne-verify decode 0x... --type market-state # getter returns have no selector; name the type
nocturne-verify decode 0x... --now 1700000000    # compute APR against a reference time
nocturne-verify decode 0x... --json              # machine-readable output
```

Types: `take`, `bundle`, `offer`, `ratifier`, `cancel`, `market-state`, `position`.

Bundle payloads (`midnightBundlesV1BuyWithUnitsTargetAndWithdrawCollateral`,
`...SupplyCollateralAndSellWithUnitsTarget`, and their `AssetsTarget` variants) decode into the
wrapper's taker-side arguments — targets/limits, permits, collateral moves, referral fee,
deadline — followed by every embedded `(offer, ratifierData, units)` fill. The wrapper arguments
are **not** covered by any maker signature; only the fills are, and each verifies exactly like a
bare take.

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

## Fetching the payload to verify

Where to get the hex this tool consumes, depending on which side of the trade you're on:

**Taker — before submitting a transaction.** When your wallet prompts you to confirm, copy the
raw transaction data instead of trusting the summary: in MetaMask, open the confirmation's
**Data / Hex** tab and copy the full hex; in Rabby and most other wallets, expand the
transaction details to find the raw calldata. Paste it into `nocturne-verify verify`. This works
for both bare `take` calls and the app's `midnightBundlesV1*` bundles.

**Anyone — after the fact.** On the explorer (e.g. Basescan), open the transaction → **Input
Data** → *View Input As → Original*. Copy the entire field — a bundle with several fills runs to
kilobytes of hex, and a partial copy is unverifiable (the tool detects truncation and refuses
rather than reporting on a partial payload; use *Copy*, don't drag-select).

**Maker — before signing offers.** What your wallet shows depends on the app's signing mode,
toggled under **Settings (gear icon) → Allow off-chain signing**:

- **Toggle on** — the app requests an `eth_signTypedData_v4` signature over the *entire offer
  tree*, so the wallet displays every field. Cross-check it with
  `nocturne-verify digest offers... --eip712` and diff the typed data, or `--expect` the digest.
- **Toggle off (default)** — you sign just the Merkle *root* digest: a bare 32-byte value the
  wallet cannot explain. Reproduce it from your intended terms with `nocturne-verify digest`
  and compare before approving. Never sign a root digest you haven't reproduced.

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
