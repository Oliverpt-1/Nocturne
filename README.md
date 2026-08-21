<p align="center">
  <a href="https://github.com/Oliverpt-1/Nocturne/actions/workflows/ci.yml"><img src="https://github.com/Oliverpt-1/Nocturne/actions/workflows/ci.yml/badge.svg" alt="CI"></a>
  <a href="#license"><img src="https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue.svg" alt="License: MIT OR Apache-2.0"></a>
  <a href="https://t.me/+tc58eLgH-dU1ZTJh"><img src="https://img.shields.io/badge/Telegram-chat-2CA5E0?logo=telegram&logoColor=white" alt="Telegram Chat"></a>
</p>

<p align="center"><em>Fast off-chain signing and offers for the Morpho Midnight protocol.</em></p>

<p align="center">
  <img src="assets/nocturne.png" alt="Nocturne - made in Rust" width="100%">
</p>

## What is Nocturne?

**Nocturne** is a high-performance Rust library for building, signing, validating, publishing, and
executing offers on the [Morpho Midnight](https://github.com/morpho-org/midnight) protocol.

Designed for market makers, trading firms, integrators, and protocol developers, Nocturne provides
the infrastructure needed to quote markets, generate Merkle trees, sign offers, simulate execution,
read the public order book, and submit wallet-ready transactions without switching languages.

Every hash, signature, price calculation, and calldata layout is checked byte-for-byte against
vectors derived from the protocol contracts (`cargo test`), and the full offer lifecycle is
exercised against the real contracts on a live Anvil deployment (`crates/nocturne/e2e/`).

Built and maintained by Oliver Tipton, and licensed under the Apache-2.0 and MIT licenses.

> **Note:** This project is **not** endorsed by, nor affiliated with, Morpho. See the full
> [disclaimer](DISCLAIMER.md).

## Status

This is an early (v0.1.0) release. Nocturne is checked byte-for-byte against vectors derived from
the Midnight contracts and exercised end-to-end against the real contracts on a live anvil
deployment, but it has **not** been independently security-audited. Verify against your own deployment before relying on it with real value, and see
[SECURITY.md](SECURITY.md) to report issues.

## Getting help

- Join the [Telegram chat](https://t.me/+tc58eLgH-dU1ZTJh) for questions and discussion.
- Open a [GitHub issue](https://github.com/Oliverpt-1/Nocturne/issues) for bugs and features.

## Choose what you need

Nocturne has one Rust library and one CLI built on that library:

| Goal | Use |
|---|---|
| Build, sign, query, publish, or execute Midnight integrations | [`nocturne-midnight`](crates/nocturne/README.md), imported in Rust as `nocturne` |
| Inspect and independently verify an existing payload | [`nocturne-verify`](crates/nocturne-verify/README.md) |

Add the SDK to a Rust project:

```toml
[dependencies]
nocturne = { package = "nocturne-midnight", version = "0.1.0" }
```

Install the optional verifier CLI when you need to review opaque calldata or typed data:

```sh
cargo install nocturne-verify

nocturne-verify decode 0x6a14c9ef...                    # opaque calldata -> readable terms
nocturne-verify verify 0x6a14c9ef... --chain-id 31337   # reproduce the root, check the signature
nocturne-verify digest offer.json  --chain-id 31337 \
    --expect 0xWalletDigest                             # cross-check the wallet digest vs your terms
```

The [SDK guide](crates/nocturne/README.md) contains the copy-paste quickstart and runnable examples.
The [verifier guide](crates/nocturne-verify/README.md) explains its full offline review workflow.

## Security

`nocturne` produces signatures and calldata that move real value. See [SECURITY.md](SECURITY.md)
for how to report vulnerabilities privately.

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

Independent, personal open source project - not created, endorsed, audited, or supported by Morpho.
Provided "AS IS" with no warranty and no liability, and it is not financial, investment, or legal
advice. Interacting with blockchain protocols and automated trading carries significant risk,
including total loss of funds. See [DISCLAIMER.md](DISCLAIMER.md) for the full text.

## License

Licensed under either of [Apache License, Version 2.0](LICENSE-APACHE) or
[MIT license](LICENSE-MIT) at your option.
