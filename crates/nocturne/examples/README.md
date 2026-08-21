# Examples

Every example runs offline with dummy keys and addresses or checked-in production calldata.
Nothing submits a transaction.

| Goal | Example | Main APIs |
|---|---|---|
| Build, sign, and verify one offer | [`quickstart.rs`](quickstart.rs) | `MarketBuilder`, `OfferBuilder`, `OfferTree`, `tree_digest`, `verify` |
| Quote and re-quote a book by APR | [`quote_book.rs`](quote_book.rs) | `OfferBuilder::apr`, `tick_to_apr`, `encode_cancel_root_calldata` |
| Validate, size, simulate, and encode a take | [`take_offer.rs`](take_offer.rs) | `validate_offer`, `seller_assets_to_units`, `simulate_take`, `encode_take_calldata` |
| Decode calldata into typed views | [`read_state.rs`](read_state.rs) | `decode_bundle_calldata`, `decode_set_is_root_ratified_calldata` |
| Benchmark the re-quote pipeline | [`bench.rs`](bench.rs) | `hash_offer`, `OfferTree`, `sign_digest` |

Run one from the workspace root:

```sh
cargo run -p nocturne-midnight --example quickstart
cargo run -p nocturne-midnight --example quote_book
cargo run -p nocturne-midnight --example take_offer
cargo run -p nocturne-midnight --example read_state
cargo run -p nocturne-midnight --release --example bench
```

The [crate README](../README.md) owns the getting-started guide so the same content appears on
GitHub, crates.io, and docs.rs. The [live Anvil harness](../e2e/README.md) covers real-contract
integration.
