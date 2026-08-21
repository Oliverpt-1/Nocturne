# nocturne-verify

Offline decoder and verifier for Morpho Midnight payloads, so nobody has to blind-sign.

## How it works

A Midnight payload is a wall of hex (or a bare 32-byte digest) that no wallet can explain. This
tool decodes it into plain terms — maker, side, price, size, expiry — and proves the bytes mean
exactly the terms it prints: Merkle roots and EIP-712 digests are recomputed from scratch,
signatures are recovered, and any mismatch is a loud FAIL.

**You verify by decoding, then reading.** The tool proves *what you read is what you sign*; it
cannot know whether those terms are the trade you want — that part is yours. `PASS` never means
"this trade is good." To check intent mechanically, pass `--offers` with offer files from your
own records.

It runs fully offline (no keys, no network) and is an independent reimplementation,
parity-checked against the Midnight contracts and real production payloads. Exit codes: `0`
PASS, `1` FAIL, `2` PARTIAL (see `verify`).

```sh
cargo install nocturne-verify
# From a source checkout: cargo run -p nocturne-verify -- <cmd>
```

Prebuilt binaries are on each [GitHub release](https://github.com/Oliverpt-1/Nocturne/releases).

## `verify` — check calldata before submitting or approving it

Takes raw calldata as hex and auto-detects what it is from the selector.

```sh
nocturne-verify verify 0x<calldata>
```

**Taker (take or bundle transaction).** Copy the raw hex from your wallet's Data/Hex tab (or
Basescan → Input Data → Original; copy the whole field — truncated payloads are refused).
Every offer is decoded and every signature checked; in a bundle, one bad fill fails the whole
thing. Useful flags: `--expected-maker 0x...`, `--chain-id N`, `--now $(date +%s)` for APRs.

**Maker (`setIsRootRatified` transaction).** The default signing mode: you approve a bare
32-byte root. Add your intended offers and PASS proves the root commits to exactly them:

```sh
nocturne-verify verify 0x2fd0e45d... --offers offer1.json offer2.json
```

**PARTIAL (exit 2)** means nothing failed but something is on-chain state the tool cannot read
offline — a SetterRatifier fill's root ratification, or a maker root checked without
`--offers`. The output prints the exact `cast call` to finish the check.

## `verify-typed` — check the typed data a maker signs

The "Allow off-chain signing" mode: the wallet asks for an `eth_signTypedData_v4` signature
over your whole offer tree. Save that JSON to a file (it's a file argument, not inline hex):

```sh
nocturne-verify verify-typed payload.json
nocturne-verify verify-typed payload.json --offers offer1.json offer2.json   # assert intent
```

Prints every offer, proves the document is the canonical encoding of those offers (no hidden
fields, no tampered types — the printed DIGEST is exactly what the wallet will hash), and
flags zero-offer padding leaves as untakeable.

## `digest` — predict the digest from your own terms

The strongest check: it never reads the app's output at all. Feed in the offers you intend
(JSON files, order = leaf order) and compare what the tool computes against what the wallet
shows — ideally on a separate machine.

```sh
nocturne-verify digest offer1.json offer2.json                     # print root + digest
nocturne-verify digest ... --expect 0xWalletDigest                 # assert the digest
nocturne-verify digest ... --expect-root 0xRootFromWallet          # assert just the root
nocturne-verify digest ... --eip712                                # emit the full typed data
```

## `decode` — just read a payload

```sh
nocturne-verify decode 0x<payload>              # auto-detects the type
nocturne-verify decode 0x... --type market-state # getter returns have no selector; name one
nocturne-verify decode 0x... --json             # machine-readable
```

Types: `take`, `bundle`, `offer`, `ratifier`, `cancel`, `ratify`, `market-state`, `position`.

## Scope

Offline, read-only, contract-anchored ([fixtures](../nocturne/fixtures/README.md)). Not an
on-chain simulation of `take`, and not security-audited — verify against your own deployment
before relying on it with real value.

## License

MIT OR Apache-2.0, same as the workspace.
