# nocturne

Rust SDK for building, signing, publishing, discovering, simulating, and taking
[Morpho Midnight](https://github.com/morpho-org/midnight) offers.

## Install

The crates.io package is `nocturne-midnight`; Rust imports it as `nocturne`:

```toml
[dependencies]
nocturne = { package = "nocturne-midnight", version = "0.1.0" }
```

Add `features = ["alloy-wallet"]` when converting publication requests directly into Alloy
transactions.

## Quickstart

Build one lend offer, sign its Merkle tree, and verify it locally:

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
        &tree.proof(0)?,
        &signature,
        chain_id,
        &ratifier,
        &maker,
    ));
    Ok(())
}
```

Run the same example from this repository:

```sh
cargo run -p nocturne-midnight --example quickstart
```

The quickstart is offline and uses dummy addresses and key material.

## Complete flows

### Publish maker offers

1. Build checked markets and offers with `MarketBuilder` and `OfferBuilder`.
2. Create canonical groups, leaves, and proofs with `OfferGroup` and `OfferTree`.
3. Sign with `LocalSigner` or `ExternalSigner`, or approve the root through SetterRatifier.
4. Encode `PayloadItem` values with `Payload::encode`.
5. Optionally validate the exact payload with `MidnightApi::validate_payload`.
6. Build the wallet request with `mempool_submission` and submit it.

[`publish_offer.rs`](https://github.com/Oliverpt-1/Nocturne/blob/main/crates/nocturne/examples/publish_offer.rs)
runs this entire flow through an Alloy wallet.

### Find and take offers

1. Fetch a book or executable quote with `MidnightApi`.
2. Size and locally check the fill with `get_consumable_units`, `take_amounts`, or
   `simulate_take`.
3. Build `take` calldata with `encode_take_calldata`.
4. Submit the calldata through the Rust wallet client of your choice.

[`take_offer.rs`](https://github.com/Oliverpt-1/Nocturne/blob/main/crates/nocturne/examples/take_offer.rs)
shows validation, sizing, simulation, and encoding.

### Manage a position

Use `supply_collateral`, `take_lend`, `take_borrow`, `supply_collateral_take_borrow`,
`repay_withdraw_collateral`, and `redeem` to build wallet-ready transactions. Before bundle calls,
build a `RequirementPlan` and call `discover_requirements` to resolve only the ERC-20 approvals and
Midnight authorization that are still missing. This RPC-backed helper requires the `alloy-wallet`
feature; the transaction builders themselves are wallet-agnostic.

## Main APIs

| Need | Start with |
|---|---|
| Markets and offers | `MarketBuilder`, `OfferBuilder` |
| Groups, roots, and proofs | `OfferGroup`, `OfferTree` |
| Local or external signing | `LocalSigner`, `ExternalSigner` |
| Books, quotes, and API validation | `MidnightApi` |
| Publication payloads | `Payload`, `mempool_submission` |
| Taking offers and managing positions | `take_lend`, `take_borrow`, `supply_collateral`, `repay_withdraw_collateral`, `redeem` |
| Required approvals and authorization | `RequirementPlan`, `discover_requirements` |
| Capacity and notional sizing | `get_consumable_units`, `buyer_assets_to_units`, `seller_assets_to_units` |
| Prices, APRs, fees, and simulation | `tick_to_price`, `tick_to_apr`, `take_amounts`, `simulate_take` |
| Calldata and state codecs | `encode_*`, `decode_*` |
| Delegated signing | `Authorization`, `authorization_digest` |

The [complete API](https://docs.rs/nocturne-midnight) is organized into modules by role. Raw
hashing, EIP-712, ABI, conversion, and recovery functions remain available for custom
infrastructure.

## Examples

| Example | Purpose |
|---|---|
| `quickstart` | Build, sign, and verify one offer |
| `publish_offer` | Publish through an Alloy wallet |
| `quote_book` | Build and re-quote an APR ladder |
| `take_offer` | Validate, size, simulate, and encode a take |
| `read_state` | Decode production calldata into typed views |
| `bench` | Measure the local re-quote pipeline |

The [examples index](https://github.com/Oliverpt-1/Nocturne/blob/main/crates/nocturne/examples/README.md)
contains exact commands and network requirements.

## Security

Nocturne can produce signatures and calldata that move real value. It is an early v0.1.0 release
and has not been independently audited. Review wallet requests, keep mutable chain state current,
and read the repository's
[security policy](https://github.com/Oliverpt-1/Nocturne/blob/main/SECURITY.md).

## License

MIT OR Apache-2.0.
