# Maintainer lifecycle tests

This harness tests complete SDK workflows. Use the single `lifecycle` entrypoint from the
repository root:

```sh
cargo run -p nocturne-midnight --features alloy-wallet --example lifecycle -- <scenario>
```

| Scenario | What it proves |
|---|---|
| `anvil` | Build, sign, authorize, take, supply, borrow, repay, redeem, decode, and cancel against freshly deployed Midnight contracts |
| `market-maker` | Quote a multi-rung grid, fill it, cancel it, reject stale quotes, and fill its replacement |
| `base-taker` | Fetch a hosted quote and complete a guarded taker borrow/repay lifecycle on Base |
| `base-maker` | Validate and publish an offer, observe router indexing, take it from a second wallet, repay, redeem, and cancel on Base |
| `resume` | Inspect the saved journal and reconcile an interrupted Base lifecycle |
| `cleanup` | Revoke the Base maker's Midnight allowance and ratifier authorization |

Every Base transaction is simulated first. The Base scenarios record confirmed transaction
hashes in the ignored `.nocturne-e2e-journal.json` file, refuse to overwrite an unfinished run,
and check shared lifecycle invariants after borrowing and after reconciliation.

## Local Anvil runs

Requirements: Rust 1.90+, Foundry, a compatible `morpho-org/midnight` checkout (last validated at
`e6f2bf28`), and a compatible `morpho-org/bundles` checkout (last validated at `9c457e9`).

```sh
MIDNIGHT_REPO=/path/to/midnight \
BUNDLES_REPO=/path/to/bundles \
cargo run -p nocturne-midnight --features alloy-wallet --example lifecycle -- anvil

MIDNIGHT_REPO=/path/to/midnight \
cargo run -p nocturne-midnight --features alloy-wallet --example lifecycle -- market-maker
```

The harness deploys real `Midnight`, `MidnightBundlesV1`, `EcrecoverRatifier`,
`EcrecoverAuthorizer`, ERC-20, and oracle contracts. Its deterministic Anvil keys are public test
fixtures and must never hold real value.

## Guarded Base runs

Load the ignored `.env.local` into your shell, then explicitly acknowledge the transaction mode:

```sh
LIVE_BASE_CONFIRM=I_UNDERSTAND \
cargo run -p nocturne-midnight --features alloy-wallet --example lifecycle -- base-taker

LIVE_MAKER_CONFIRM=I_UNDERSTAND \
cargo run -p nocturne-midnight --features alloy-wallet --example lifecycle -- base-maker
```

`base-taker` uses `RPC_URL` and `PRIVATE_KEY`. `base-maker` uses `RPC_URL`,
`PRIVATE_KEY_BIGGER`, and `PRIVATE_KEY_Wallet_1`. Never commit these values. If a run stops after a
transaction, use `resume`; use `cleanup` to revoke maker permissions.

These tests are strong compatibility evidence, not a security audit. Hosted API availability,
RPC behavior, chain reorganizations, gas policy, and production key custody remain external
operational risks.
