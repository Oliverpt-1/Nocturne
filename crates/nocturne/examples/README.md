# Runnable examples

These examples are the executable counterparts to the
[SDK integration guide](../README.md). Run every command from the repository root.

## First offer

| Example | Lifecycle stage | Network | Command |
|---|---|---:|---|
| [`quickstart.rs`](quickstart.rs) | Build → group → sign → locally verify one maker offer | No | `cargo run -p nocturne-midnight --example quickstart` |

Start here. It uses deterministic dummy data and does not read environment variables.

## Maker workflows

| Example | Lifecycle stage | Network | Command |
|---|---|---:|---|
| [`quote_book.rs`](quote_book.rs) | Build an APR ladder → sign one tree → cancel and re-quote | No | `cargo run -p nocturne-midnight --example quote_book` |
| [`publish_offer.rs`](publish_offer.rs) | Build → sign → encode → optionally API-validate → submit through Alloy | Yes | `cargo run -p nocturne-midnight --features alloy-wallet --example publish_offer` |

`publish_offer` requires `RPC_URL` and `PRIVATE_KEY`. Set `VALIDATE_PAYLOAD=1` to call the public
Midnight validation API before submission. Base addresses are built in; custom deployments may
set `MIDNIGHT_ADDRESS`, `RATIFIER_ADDRESS`, and `MEMPOOL_ADDRESS`.

Never use a valuable private key with an example program.

## Taker and read-only workflows

| Example | Lifecycle stage | Network | Command |
|---|---|---:|---|
| [`take_offer.rs`](take_offer.rs) | Validate → size → simulate → encode and decode `take` calldata | No | `cargo run -p nocturne-midnight --example take_offer` |
| [`read_state.rs`](read_state.rs) | Decode production bundle and ratification fixtures into typed views | No | `cargo run -p nocturne-midnight --example read_state` |

`take_offer` begins with a locally constructed signed offer so the complete file remains offline.
In a live integration, replace that setup with takeable offers from `MidnightApi::fetch_quote` or
`MidnightApi::fetch_book_takeable_offers`, refresh chain context, and submit the encoded calldata
through a wallet.

## Performance

| Example | What it measures | Network | Command |
|---|---|---:|---|
| [`bench.rs`](bench.rs) | Hash leaves → build tree → generate every proof → sign root | No | `cargo run -p nocturne-midnight --release --example bench` |

The benchmark reports the best of repeated runs for 1,024, 4,096, and 16,384 offers. It is a local
engineering tool, not a protocol performance guarantee.

## Live contract coverage

The examples above teach individual SDK workflows. The separate
[maintainer E2E harness](../e2e/README.md) deploys real Midnight contracts to Anvil and proves that
the complete artifacts are accepted on-chain. Integrators do not need the harness to use the SDK.
