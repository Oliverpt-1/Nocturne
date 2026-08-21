# nocturne

Rust SDK for building, signing, validating, simulating, publishing, and taking offers on the
Morpho Midnight protocol. It covers the full Rust integration path, including the public order-book
API and wallet-ready Ethereum transaction requests.

If you only need to inspect a payload before signing or submitting it, use the
[`nocturne-verify`](https://github.com/Oliverpt-1/Nocturne/tree/main/crates/nocturne-verify)
command-line tool instead.

## Getting started

Add the crate:

```toml
[dependencies]
nocturne = { package = "nocturne-midnight", version = "0.1.0" }
```

Enable the optional Alloy adapter when this crate should hand the final request directly to an
Alloy provider:

```toml
nocturne = { package = "nocturne-midnight", version = "0.1.0", features = ["alloy-wallet"] }
```

Build a lend offer, sign its one-leaf tree, and verify the result locally:

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

    assert!(verify(
        offer,
        &tree.root(),
        0,
        &tree.proof(0).unwrap(),
        &signature,
        chain_id,
        &ratifier,
        &maker,
    ));

    Ok(())
}
```

This example uses dummy keys and addresses and does not touch a network. Run its executable
version from the workspace with `cargo run -p nocturne-midnight --example quickstart`.

## Validate and publish offers

Maker offers are published by sending the encoded payload itself as a zero-value transaction to
the registered Midnight mempool address. There is no Solidity function selector at this step.

```rust,ignore
use alloy::providers::Provider;
use nocturne::{mempool_submission, MidnightApi, Payload, PayloadItem};

async fn publish<P: Provider>(
    provider: &P,
    items: &[PayloadItem],
) -> Result<(), Box<dyn std::error::Error>> {
    let payload = Payload::encode(items)?;
    let validation = MidnightApi::default()
        .validate_payload(8453, &payload, None)
        .await?;
    if !validation.valid {
        return Err(format!("payload rejected: {:?}", validation.issues).into());
    }

    let request = mempool_submission(8453, payload, [])?;
    provider.send_transaction(request.into()).await?.watch().await?;
    Ok(())
}
```

The runnable [`publish_offer.rs`](https://github.com/Oliverpt-1/Nocturne/blob/main/crates/nocturne/examples/publish_offer.rs)
starts at market construction, creates and signs the tree, builds ratifier data, encodes and
optionally validates the payload, broadcasts it with an Alloy wallet, then reads the mined
transaction back and checks its destination, value, and input.

The API client also exposes books, individual price levels, quotes, and executable takeable offers
through `MidnightApi`. Returned takeable offers are converted directly into Nocturne `Offer`
values and checked against the requested market, side, maker, and group before use.

## Common workflows

| Goal | Main APIs | Example |
|---|---|---|
| Quote and sign an offer book | `OfferBuilder`, `OfferGroup`, `OfferTree`, `tree_digest`, `Signer` | [`quote_book.rs`](https://github.com/Oliverpt-1/Nocturne/blob/main/crates/nocturne/examples/quote_book.rs) |
| Validate, size, and simulate a take | `validate_offer`, `seller_assets_to_units`, `simulate_take` | [`take_offer.rs`](https://github.com/Oliverpt-1/Nocturne/blob/main/crates/nocturne/examples/take_offer.rs) |
| Encode a transaction | `encode_ratifier_data`, `encode_take_calldata`, `encode_bundle_calldata` | [`take_offer.rs`](https://github.com/Oliverpt-1/Nocturne/blob/main/crates/nocturne/examples/take_offer.rs) |
| Read books and validate payloads | `MidnightApi` | API documentation |
| Publish maker offers | `Payload`, `mempool_submission`, optional Alloy adapter | [`publish_offer.rs`](https://github.com/Oliverpt-1/Nocturne/blob/main/crates/nocturne/examples/publish_offer.rs) |
| Decode calldata and contract state | `decode_bundle_calldata`, `decode_market_state`, `decode_position` | [`read_state.rs`](https://github.com/Oliverpt-1/Nocturne/blob/main/crates/nocturne/examples/read_state.rs) |
| Delegate to a signing key | `Authorization`, `sign_authorization` | API documentation |

See the [examples index](https://github.com/Oliverpt-1/Nocturne/tree/main/crates/nocturne/examples)
for runnable commands and the
[API documentation](https://docs.rs/nocturne-midnight) for every public type and function. The
[live Anvil harness](https://github.com/Oliverpt-1/Nocturne/tree/main/crates/nocturne/e2e)
exercises the complete lifecycle against the real Midnight contracts.

## SDK versus verifier

`nocturne` is an integration library: applications use it to create offers and transaction
payloads. `nocturne-verify` is an independent, offline review tool: humans and signing systems use
it to decode an existing payload, recompute its root and digest, and check that it matches their
intent. The verifier never creates or submits a transaction.

## Security

This crate produces signatures and calldata that can move real value. It is not independently
audited. Verify against your deployment before using it with funds and report vulnerabilities as
described in the repository's
[security policy](https://github.com/Oliverpt-1/Nocturne/blob/main/SECURITY.md).

## License

Licensed under either Apache-2.0 or MIT at your option.
