# Security Policy

`nocturne` produces signatures and calldata that move real value on Morpho Midnight. Please treat
any correctness or signing issue as potentially security-relevant.

## Reporting a vulnerability

**Do not open a public issue for security problems.** Instead, report privately:

- Use GitHub's **[Report a vulnerability](https://github.com/Oliverpt-1/midnight-rust/security/advisories/new)**
  (Security → Advisories), or
- email **oliverptipton@gmail.com** with details and, if possible, a reproduction.

Please include the affected version/commit, the impact, and steps to reproduce. We aim to
acknowledge within a few business days and will coordinate a fix and disclosure timeline with you.

## Scope

In scope — anything that could cause a maker to sign or submit something other than intended, or a
consumer to mis-read on-chain state:

- incorrect EIP-712 hashing / digest assembly (an offer or authorization that verifies differently
  than intended, or fails to verify)
- incorrect signature recovery, low-`s` normalization, or `v` handling
- incorrect ABI encoding of `take` / `cancelRoot` calldata
- incorrect decoding of offers or on-chain state
- incorrect tick/price/APR, settlement-fee, sizing, or take-simulation math that diverges from the
  contracts
- panics reachable from untrusted input

## Out of scope

- The Midnight smart contracts themselves (report those to the
  [contracts repository](https://github.com/morpho-org/midnight)).
- Issues in dependencies (report upstream), unless this crate uses them unsafely.
- Trading strategy, quoting logic, or key management on the consumer's side.

## Verification

Every hashing, signing, pricing, and encoding path is parity-checked byte-for-byte against the
Midnight contracts (`cargo test`), and the full lifecycle is exercised against a real deployment on
anvil (`crates/nocturne/e2e/`). A report that shows a divergence from the contracts is exactly the
kind of thing we want to hear about.
