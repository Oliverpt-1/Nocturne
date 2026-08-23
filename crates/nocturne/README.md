# nocturne

Rust SDK for building complete [Morpho Midnight](https://github.com/morpho-org/midnight)
integrations: construct and publish maker offers, fetch books and quotes, size and simulate fills,
and produce transactions that a Rust wallet can submit.

`nocturne-verify` is a separate, optional command-line tool for inspecting data produced by an
application or wallet. It is not required to use this SDK.

## Installation and names

The published package is named `nocturne-midnight`, while Rust code imports it as `nocturne`:

```toml
[dependencies]
nocturne = { package = "nocturne-midnight", version = "0.1.0" }
```

Enable the optional Alloy conversion when the final publication request should be handed directly
to an Alloy provider:

```toml
nocturne = { package = "nocturne-midnight", version = "0.1.0", features = ["alloy-wallet"] }
```

The SDK itself is wallet-agnostic. Only the conversion from `MempoolTransaction` to Alloy's
`TransactionRequest` is feature-gated.

## Five-minute quickstart

This offline example builds a lend offer, assigns its canonical group, constructs its one-leaf
tree, signs the EIP-712 tree digest, and verifies the proof and signature locally:

```rust
use nocturne::*;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let signer = LocalSigner::from_bytes(&[0x42; 32])?;
    let maker = signer.address();
    let ratifier = [0xbb; 20];
    let chain_id = word_from_u64(1);

    let market = MarketBuilder::new(1, [0x11; 20], [0x22; 20])
        .collateral(
            [0x33; 20],
            U256::from(770_000_000_000_000_000u64),
            U256::from(1u64),
            [0x44; 20],
        )
        .maturity(2_000_000_000)
        .build_checked()?;

    let offer = OfferBuilder::new(market, maker)
        .lend()
        .apr(7.2, 1_700_000_000)
        .expiry(2_000_000_000)
        .ratifier(ratifier)
        .max_units(1_000_000)
        .build_checked()?;

    let descriptor = OfferTree::from_entries([offer])?;
    let offer = &descriptor.offers[0];
    let tree = descriptor.tree;
    let digest = tree_digest(tree.root(), tree.height(), chain_id, &ratifier);
    let signature = signer.sign_digest(&digest)?;
    let proof = tree.proof(0)?;

    assert!(verify(
        offer,
        &tree.root(),
        0,
        &proof,
        &signature,
        chain_id,
        &ratifier,
        &maker,
    ));

    Ok(())
}
```

Run the same code from a source checkout:

```sh
cargo run -p nocturne-midnight --example quickstart
```

The example uses dummy keys and addresses and never touches a network.

## Mental model

A Midnight offer is not published by calling a normal Solidity function. The maker commits one or
more offers into a Merkle tree, authorizes that tree through a ratifier, encodes each offer with its
proof data, and sends the raw payload to the Midnight mempool address.

```text
maker terms
  → MarketBuilder / OfferBuilder
  → local validation
  → OfferGroup / OfferTree
  → Ecrecover or Setter ratification
  → Payload::encode
  → optional API policy validation
  → mempool_submission
  → Rust wallet

Midnight API
  → book / quote / takeable offers
  → local sizing and simulation
  → encode_take_calldata
  → Rust wallet
```

## Make and publish offers

### 1. Build terms

Use `MarketBuilder::build_checked` and `OfferBuilder::build_checked` for local construction. A
checked offer requires a side, tick or APR, expiry, ratifier, and exactly one consumption cap.
`OfferBuilder::try_build` additionally accepts a `ValidateCtx` when current time or market state is
available.

### 2. Group and commit offers

- `OfferGroup::create` groups offers that share one consumption cap.
- `OfferTree::from_entries` assigns canonical group identifiers, pads the tree, rejects duplicate
  leaves or mixed signing domains, and returns the final offers in leaf order.
- `OfferTree::proof(index)` returns the proof that travels with that leaf.

Always publish the offers returned in `OfferTreeDescriptor::offers`; their canonical group fields
may differ from the builder inputs.

### 3. Ratify the tree

Nocturne supports both Midnight ratifier routes:

| Route | Maker action | Ratifier data in each payload item |
|---|---|---|
| Ecrecover | Sign the EIP-712 tree digest with the maker or an authorized signer | `encode_ratifier_data(signature, root, index, proof)` |
| Setter | Approve the root on-chain with `setIsRootRatified` | `encode_setter_ratifier_data(root, index, proof)` |

`LocalSigner` is the reference in-process signer. For a KMS, HSM, remote wallet, or custody
service, use `ExternalSigner`; it converts DER ECDSA output into an Ethereum recoverable,
low-`s` signature. `Authorization` helpers cover EIP-712 delegation to a separate signing key.

### 4. Encode and validate the exact payload

Create one `PayloadItem` per final offer and call `Payload::encode`. Before requesting a signature,
validate offer terms locally. Before publishing, `MidnightApi::validate_items` or
`MidnightApi::validate_payload` can apply current API policy to the exact encoded bytes.

Local validation and API validation answer different questions:

- Local validation checks deterministic protocol structure and supplied market state.
- API validation checks the current service policy and information available to the API.
- Neither replaces wallet review or an on-chain simulation against the target deployment.

### 5. Publish through a wallet

`mempool_submission` wraps the encoded payload in a wallet-agnostic, zero-value transaction to the
registered mempool. Use `mempool_submission_to` for local or custom deployments. With the
`alloy-wallet` feature, convert the result directly into an Alloy transaction request.

The runnable
[`publish_offer.rs`](https://github.com/Oliverpt-1/Nocturne/blob/main/crates/nocturne/examples/publish_offer.rs)
performs the entire sequence: build, sign, encode, optionally validate, submit through Alloy, and
read the mined transaction back to confirm its destination, value, and data.

## Find and take offers

### 1. Discover executable liquidity

`MidnightApi` provides:

- `fetch_books` and `fetch_book` for markets and top-of-book data.
- `fetch_price_levels` for aggregated bid or ask levels.
- `fetch_quote` for target-aware, bundle-ready quotes.
- `fetch_book_takeable_offers` for executable offers on one side.
- `fetch_takeable_offers` for one maker's active offers.

An ask is a maker-sell offer; a bid is a maker-buy offer. API takeable offers already contain the
typed `Offer`, ratifier data, and maximum executable units needed to build `IMidnight.take` input.
The client binds returned offers to the requested market, side, maker, and group and sorts them in
execution order.

### 2. Size and simulate

Before encoding a transaction:

- `get_consumable_units` applies time, fee-cap, and consumed-cap limits.
- `buyer_assets_to_units` and `seller_assets_to_units` size a fill by notional assets.
- `take_amounts` computes buyer, seller, and settlement-fee amounts.
- `simulate_take` applies market and position context and reports exact deltas and revert reasons.

Simulation is only as current as the `SimCtx` supplied by the caller. Refresh mutable on-chain
state immediately before execution when stale state would affect safety.

### 3. Encode and submit

Clamp each fill to the smaller of the quote cap and the remaining target, then call
`encode_take_calldata`. Submit the returned bytes to the offer's Midnight contract through the
wallet client of your choice. Multi-offer integrations can use `encode_bundle_calldata` when they
already have a complete bundle description.

The offline
[`take_offer.rs`](https://github.com/Oliverpt-1/Nocturne/blob/main/crates/nocturne/examples/take_offer.rs)
walks through validation, notional sizing, simulation, calldata encoding, and a decode round-trip.
API-backed applications should begin with `MidnightApi::fetch_quote` and then apply the same sizing
and simulation steps to its `takeable_offers`.

## Read-only integrations

An integration does not need a wallet to:

- Fetch books, price levels, quotes, and maker offers with `MidnightApi`.
- Decode `Market`, `Offer`, market-state, position, consumption, ratifier, take, cancel, and bundle
  bytes with the `decode_*` functions.
- Convert ticks to prices or APRs and calculate settlement fees.
- Verify a Merkle proof or recover an EIP-712 signer.

[`read_state.rs`](https://github.com/Oliverpt-1/Nocturne/blob/main/crates/nocturne/examples/read_state.rs)
decodes production bundle and ratification calldata into typed views and connects the fill's root
to the maker's ratification transaction.

## Signing and wallets

| Need | API | Key handling |
|---|---|---|
| Local development or reference signing | `LocalSigner` | Raw key lives in process |
| KMS, HSM, custody, or remote backend | `ExternalSigner` | Caller supplies a DER-signing closure |
| Delegate a hot signer | `Authorization`, `authorization_digest` | Maker authorizes a separate address on-chain |
| Submit with Alloy | `alloy-wallet` feature | Converts the final request; Alloy owns RPC and wallet behavior |

Never log private keys or signer internals. `LocalSigner` deliberately omits key material from its
`Debug` output, but production applications should prefer external key custody.

## Task-to-API map

| Task | Primary APIs |
|---|---|
| Construct a market | `MarketBuilder` |
| Construct an offer | `OfferBuilder` |
| Validate an offer | `validate_offer`, `ValidateCtx` |
| Group offers | `OfferGroup` |
| Build roots and proofs | `OfferTree` |
| Sign locally | `LocalSigner` |
| Use an external wallet or HSM | `ExternalSigner` |
| Delegate a signer | `Authorization`, `authorization_digest` |
| Encode publication payloads | `Payload`, `PayloadItem` |
| Validate current mempool policy | `MidnightApi::validate_items` |
| Build publication transactions | `mempool_submission` |
| Fetch books and quotes | `MidnightApi` |
| Size available liquidity | `get_consumable_units` |
| Simulate execution | `simulate_take`, `take_amounts` |
| Build take, cancel, or ratification calldata | `encode_*` helpers |
| Decode calldata and state | `decode_*` helpers |
| Independently inspect external data | separate `nocturne-verify` CLI |

## Errors and safety boundaries

| Error family | Meaning |
|---|---|
| `BuildError`, `MarketBuildError`, `GroupError`, `OfferTreeError` | Local terms or collection structure are invalid |
| `OfferError` | One or more deterministic or context-dependent offer checks failed |
| `ApiError`, `ValidationIssue` | HTTP, response-shape, or current API-policy failure |
| `SimError`, `SizingError` | Invalid math input, checked overflow, or unavailable sizing result |
| `PayloadError`, `SubmissionError`, `DecodeError` | Bytes cannot be safely encoded, decoded, or submitted |
| Wallet or RPC error | The external signer, node, chain state, or transaction rejected the operation |

The SDK returns typed errors where callers can recover and vectors of `OfferError` where reporting
multiple invalid terms at once is more useful than failing on the first one.

## Low-level APIs

Most integrations should begin with the builders, `OfferTree`, `MidnightApi`, `Payload`, sizing,
and simulation APIs above. The crate also exposes protocol primitives for specialized systems:

- EIP-712 typehash, struct-hash, domain-separator, and digest construction.
- Raw Merkle leaf and node hashing.
- ABI encoders and decoders for Midnight and bundle calls.
- `Word`, `Address`, and `U256` conversion helpers.
- Raw signature recovery and low-`s` normalization.

These functions are intentionally available for audit tooling and custom infrastructure, but they
require the caller to preserve the same domain, ordering, and on-chain context enforced by the
higher-level workflows.

See the
[examples index](https://github.com/Oliverpt-1/Nocturne/blob/main/crates/nocturne/examples/README.md)
for exact Cargo commands and
[docs.rs](https://docs.rs/nocturne-midnight) for every public type and function.

## Terminology

- **Build:** create local market or offer terms.
- **Validate:** check local invariants or current API policy.
- **Sign:** authorize an EIP-712 digest.
- **Ratify:** produce the ratifier-specific proof used when an offer is taken.
- **Encode:** construct payload or calldata bytes.
- **Publish:** submit maker-offer payload bytes to the Midnight mempool.
- **Take:** execute an offer against the Midnight contract.
- **Verify:** independently reproduce and inspect what will be signed or submitted.

## SDK and verifier boundary

The SDK creates integration artifacts and wallet-ready transactions. The separate
[`nocturne-verify`](https://github.com/Oliverpt-1/Nocturne/tree/main/crates/nocturne-verify) CLI is
read-only: it inspects existing bytes or typed data and never creates or submits a transaction.

## Security

This crate produces signatures and calldata that can move real value. It is an early v0.1.0
release and has not been independently audited. Verify behavior against the intended deployment,
keep mutable state current, review wallet requests, and report vulnerabilities through the
repository's [security policy](https://github.com/Oliverpt-1/Nocturne/blob/main/SECURITY.md).

## License

Licensed under either Apache-2.0 or MIT at your option.
