# nocturne-verify

Offline, read-only inspection for Morpho Midnight calldata, maker payloads, and EIP-712 signing
requests. It turns opaque bytes into readable terms and reproduces the hashes, Merkle roots,
proofs, and digests that those bytes commit to.

## What this tool is

Use `nocturne-verify` when an application or wallet gives you data that is difficult to review:

- Taker `take` or bundle calldata.
- Maker `setIsRootRatified` calldata.
- EIP-712 typed data for an off-chain offer-tree signature.
- A compressed maker-offer mempool payload.
- Raw getter return bytes for market or position state.

The CLI decodes the input, prints its terms, and checks every property available from those bytes.
Pass your own offer JSON with `--offers` when the result must also match separately recorded intent.

## What this tool is not

- It does not build offers or transactions; use the `nocturne` SDK for integration work.
- It never holds a key, contacts an RPC node, or submits anything.
- `PASS` does not mean a trade is economically desirable or safe.
- It cannot prove mutable on-chain state while offline. Such checks produce `PARTIAL`, with a
  `cast call` command when an RPC check can complete the review.
- It is independent from the application or wallet being inspected, but it shares protocol
  primitives with the `nocturne` Rust library. It is not a separately authored cryptographic
  implementation.

The strongest workflow starts from offer JSON in your own records and uses `digest` to predict the
root or wallet digest without trusting the application's displayed interpretation.

## Installation

Install the published binary:

```sh
cargo install nocturne-verify
```

Run it from a source checkout:

```sh
cargo run -p nocturne-verify -- <command>
```

Prebuilt binaries are attached to each
[GitHub release](https://github.com/Oliverpt-1/Nocturne/releases).

## Verify transaction calldata

`verify` accepts raw calldata and identifies supported calls from their selector:

```sh
nocturne-verify verify 0x<calldata>
```

Copy the complete value from the wallet's Data/Hex view or a block explorer's original input-data
view. Truncated data is rejected.

### Taker take or bundle

Every offer and ratifier payload is decoded. Ecrecover signatures, Merkle membership, signing
domains, leaf bounds, and bundle fills are checked. One bad fill fails the complete bundle.

Useful assertions:

```sh
nocturne-verify verify 0x<calldata> \
  --expected-maker 0x<maker> \
  --chain-id 8453 \
  --now 1780000000
```

`--now` makes expiry and displayed APR calculations explicit.

### Maker root ratification

A SetterRatifier transaction approves one opaque root. Supply the maker's intended offers in leaf
order to prove that the root commits to exactly those terms:

```sh
nocturne-verify verify 0x<setIsRootRatified-calldata> \
  --offers offer1.json offer2.json
```

Without `--offers`, the root can be decoded but its intended leaves cannot be established offline,
so the result is `PARTIAL`.

## Verify a typed-data signing request

`verify-typed` accepts the complete `eth_signTypedData_v4` JSON document as a file:

```sh
nocturne-verify verify-typed typed-data.json
nocturne-verify verify-typed typed-data.json --offers offer1.json offer2.json
```

It prints every real leaf, identifies zero-offer padding, verifies the EIP-712 type table and
canonical document shape, rebuilds the Merkle tree, and prints the exact digest a conforming wallet
will hash. `--offers` additionally asserts that the real leaves equal independently recorded offer
terms in the same order.

## Reproduce a digest from intended terms

`digest` begins with offer JSON rather than application-generated calldata or typed data:

```sh
nocturne-verify digest offer1.json offer2.json
nocturne-verify digest offer1.json offer2.json --expect 0x<wallet-digest>
nocturne-verify digest offer1.json offer2.json --expect-root 0x<wallet-root>
nocturne-verify digest offer1.json offer2.json --eip712
```

| Option | Result |
|---|---|
| no assertion | Print the canonical root and EIP-712 digest |
| `--expect` | Fail unless the computed digest matches |
| `--expect-root` | Fail unless the computed root matches |
| `--eip712` | Emit the complete canonical typed-data JSON document |

For maximum separation, create the offer files from your own records and run this command on a
different machine from the application requesting the signature.

## Decode without verifying

`decode` is the inspection-only path:

```sh
nocturne-verify decode 0x<payload>
nocturne-verify decode 0x<getter-return> --type market-state
nocturne-verify decode 0x<payload> --json
```

Supported explicit types are `take`, `bundle`, `offer`, `ratifier`, `cancel`, `ratify`,
`market-state`, and `position`. Selector-bearing calldata is normally detected automatically;
getter return data needs `--type` because it has no selector.

## Exit codes

| Code | Meaning |
|---:|---|
| `0` | `PASS`: every requested and locally available check passed |
| `1` | `FAIL`: at least one check failed or the input was invalid |
| `2` | `PARTIAL`: nothing failed, but an intent or on-chain-state check remains unavailable offline |

Automation should treat both `1` and `2` as incomplete approval unless its policy explicitly
handles the missing `PARTIAL` checks.

## Trust and security boundaries

The verifier proves that the bytes it parsed correspond to the terms it prints. Only the signer or
taker can decide whether those terms match their intent. Supplying `--offers` or an expected digest
makes that intent comparison mechanical instead of visual.

The CLI is parity-tested against contract-generated fixtures and production payload shapes, but it
is an early v0.1.0 release and is not independently security-audited. It is not an on-chain
simulation of `take`; use the SDK's simulation APIs and current chain state for execution analysis.

## License

MIT OR Apache-2.0, same as the workspace.
