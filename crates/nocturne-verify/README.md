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
agree. Output ends with one of three verdicts:

- `RESULT: PASS` (exit 0) — every check passed, including signature recovery.
- `RESULT: FAIL` (exit 1) — do not trust the payload; the failing checks are listed.
- `RESULT: PARTIAL` (exit 2) — terms and Merkle membership verified, but the payload uses the
  **SetterRatifier**, which carries no signature: the maker authorizes the whole tree on-chain
  via `setIsRootRatified` instead. Contract storage cannot be read offline, so the tool prints
  the exact `cast call <ratifier> "isRootRatified(address,bytes32)(bool)" <maker> <root>`
  command per fill to complete the verification against an RPC.

Both known ratifier-data layouts are detected automatically: the EcrecoverRatifier's
`(Signature, root, leafIndex, proof)` and the SetterRatifier's `(root, leafIndex, proof)`.

**Maker side — verifying a `setIsRootRatified` transaction.** A SetterRatifier maker authorizes
a whole offer tree by confirming a transaction whose calldata is just `(maker, root, true)` — a
bare 32-byte root the wallet cannot explain. Verify it against your own intended terms:

```sh
# PASS proves the root commits to exactly these offers, and nothing else:
nocturne-verify verify 0x2fd0e45d... --offers offer1.json offer2.json ...
# equivalent root-only check from the digest side:
nocturne-verify digest offer1.json offer2.json --expect-root 0xRootFromWallet
```

Without `--offers` the payload is decoded and reported PARTIAL — a bare root cannot be verified
by itself. Offer order matters: it sets the leaf indices.

### `verify-typed` — check the typed data a wallet asks a maker to sign

With the app's "Allow off-chain signing" toggle on, a maker signs an `eth_signTypedData_v4`
document over the entire offer tree. Save that JSON and verify it before signing:

```sh
nocturne-verify verify-typed payload.json
# additionally assert the leaves are exactly your intended offers:
nocturne-verify verify-typed payload.json --offers offer1.json offer2.json ...
```

Every offer is decoded to readable terms (zero-offer padding leaves are summarized — they can
never be taken), and the load-bearing check regenerates the canonical document from the parsed
offers and requires it to equal the input: that proves there are no hidden fields and no
tampered `types` table, so the DIGEST printed is exactly what the wallet will hash. A tampered
*value* still reads as canonical — that is what your own eyes, or `--offers`, are for.

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
  tree*. Save the typed-data JSON the app/wallet shows to a file (any name — `payload.json`)
  and run `nocturne-verify verify-typed payload.json` before approving. Unlike take/bundle
  hex, this payload is a file argument, not pasted inline.
- **Toggle off (default)** — you confirm a `setIsRootRatified` transaction carrying a bare
  32-byte root the wallet cannot explain. Copy its calldata and run
  `nocturne-verify verify 0x2fd0e45d... --offers <your offer JSONs>`, or reproduce the root
  from your terms with `nocturne-verify digest ... --expect-root`. Never approve a root you
  haven't reproduced.

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
