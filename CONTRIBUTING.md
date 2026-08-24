# Contributing

Contributions are welcome. Bug reports, documentation improvements, tests, and focused code
changes are all useful.

## Before you start

- Search the existing issues before opening a new one.
- Open an issue before starting a substantial feature or public API change so the scope can be
  discussed first. Small fixes may go directly to a pull request.
- Report security vulnerabilities privately as described in [SECURITY.md](SECURITY.md). Do not
  open a public issue for a suspected vulnerability.

## Development

Nocturne requires Rust 1.90 or newer. Create a focused branch from `main`, make your change, and
run the same checks used by CI:

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-targets --all-features
cargo test --workspace --doc --all-features
cargo doc --workspace --no-deps --all-features
```

The live Anvil harness is maintainer-oriented and requires Foundry plus compatible Midnight and
MidnightBundles contract checkouts. See the [E2E guide](crates/nocturne/e2e/README.md) when a
change affects transaction encoding, requirements, state transitions, or contract behavior.

Never commit private keys, RPC credentials, API keys, `.env` files, or live-test artifacts.

## Code and testing

- Keep changes small and limited to one purpose.
- Match the existing style and prefer clear, explicit code in protocol-critical paths.
- Prefer deterministic offchain implementations for hashing, validation, sizing, simulation, and
  transaction construction wherever practical. Contributors should be able to exercise most SDK
  behavior without funding a wallet or depending on a hosted service.
- Keep local simulation aligned with the contracts. Use Anvil or live-chain testing for integration
  boundaries that cannot be proven offchain, rather than making funded-wallet tests a normal
  development requirement.
- Preserve public API compatibility unless the change has been discussed as a breaking change.
- Add tests for new behavior and regressions. Protocol math, hashes, signatures, calldata, and
  payload changes should include contract or SDK parity evidence where possible.
- Include malformed, boundary, and overflow cases for parsers and arithmetic.
- Update documentation and runnable examples when user-facing behavior changes.
- Avoid unrelated formatting, dependency, or refactoring changes in the same pull request.

## Pull requests

A pull request should explain:

- What changed and why.
- Any security, compatibility, or protocol assumptions involved.
- How the change was tested.
- Whether it changes public APIs, encoded output, transaction behavior, or minimum supported Rust
  version.

Keep commits readable and logically separated. All CI checks must pass before merge. Review may
request additional parity vectors or Anvil coverage for changes that affect signed or onchain
data.

## License

By contributing, you agree that your contribution will be licensed under the repository's dual
[Apache-2.0](LICENSE-APACHE) or [MIT](LICENSE-MIT) terms.

## Questions

Open a [GitHub issue](https://github.com/Oliverpt-1/Nocturne/issues) or join the
[Telegram chat](https://t.me/+tc58eLgH-dU1ZTJh).
