<p align="center">
  <a href="https://github.com/Oliverpt-1/Nocturne/actions/workflows/ci.yml"><img src="https://github.com/Oliverpt-1/Nocturne/actions/workflows/ci.yml/badge.svg" alt="CI"></a>
  <a href="#license"><img src="https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue.svg" alt="License: MIT OR Apache-2.0"></a>
  <a href="https://t.me/+tc58eLgH-dU1ZTJh"><img src="https://img.shields.io/badge/Telegram-chat-2CA5E0?logo=telegram&logoColor=white" alt="Telegram Chat"></a>
</p>

<p align="center"><em>Fast off-chain signing and offers for the Morpho Midnight protocol.</em></p>

<p align="center">
  <a href="crates/nocturne/README.md">SDK Guide</a> |
  <a href="https://docs.rs/nocturne-midnight">API Docs</a> |
  <a href="crates/nocturne/examples">Examples</a> |
  <a href="crates/nocturne-verify/README.md">Verifier</a>
</p>

<p align="center">
  <img src="assets/nocturne.png" alt="Nocturne - made in Rust" width="100%">
</p>

## What is Nocturne?

Nocturne is a Rust SDK for integrating with the
[Morpho Midnight](https://github.com/morpho-org/midnight) protocol. It builds and signs offers,
constructs Merkle trees, simulates execution, reads the public order book, encodes calldata and
mempool payloads, and produces transactions that Rust wallet clients can submit.

## Status

Nocturne is an early v0.1.0 release and has not been independently security-audited. See
[SECURITY.md](SECURITY.md) before using it with real value.

## For Integrators

The SDK package is `nocturne-midnight` and is imported in Rust as `nocturne`:

```toml
[dependencies]
nocturne = { package = "nocturne-midnight", version = "0.1.0" }
```

Start with the [SDK guide](crates/nocturne/README.md) for a copy-paste quickstart, publication flow,
and common workflows. The [examples](crates/nocturne/examples) are runnable with Cargo, and the
[API documentation](https://docs.rs/nocturne-midnight) covers the complete public surface.

`nocturne-verify` is a separate CLI for inspecting an existing payload or typed-data request
without creating or submitting transactions. Its [verifier guide](crates/nocturne-verify/README.md)
contains installation and usage instructions.

## For Developers

The minimum supported Rust version is 1.90. Run the local test suite with:

```sh
cargo test --workspace --all-targets
```

Protocol-critical outputs are checked against contract and SDK vectors. The
[live Anvil harness](crates/nocturne/e2e) exercises the complete lifecycle against deployed
Midnight contracts and requires Foundry plus a compatible contracts checkout.

## Getting help

- Join the [Telegram chat](https://t.me/+tc58eLgH-dU1ZTJh) for questions and discussion.
- Open a [GitHub issue](https://github.com/Oliverpt-1/Nocturne/issues) for bugs and features.

## Security

Nocturne produces signatures and calldata that can move real value. Report vulnerabilities
privately using the process in [SECURITY.md](SECURITY.md).

## Acknowledgements

- [Morpho](https://github.com/morpho-org/midnight) - the Midnight protocol and contracts this
  library mirrors.
- [ruint](https://github.com/recmo/uint) / [alloy](https://github.com/alloy-rs) - the `U256`
  primitive.
- [RustCrypto `k256`](https://github.com/RustCrypto/elliptic-curves) - secp256k1 signing and
  recovery.
- [`tiny-keccak`](https://github.com/debris/tiny-keccak), [`rayon`](https://github.com/rayon-rs/rayon).
- [Foundry](https://github.com/foundry-rs/foundry) - `anvil` / `forge` / `cast` power the parity
  fixtures and the live e2e harness.

## Disclaimer

Independent, personal open source project—not created, endorsed, or supported by Morpho.
Provided "AS IS" with no warranty and no liability, and it is not financial, investment, or legal
advice. Interacting with blockchain protocols and automated trading carries significant risk,
including total loss of funds. See [DISCLAIMER.md](DISCLAIMER.md) for the full text.

## License

Licensed under either of [Apache License, Version 2.0](LICENSE-APACHE) or
[MIT license](LICENSE-MIT) at your option.
