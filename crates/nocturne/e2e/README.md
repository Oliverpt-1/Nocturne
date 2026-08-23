# Maintainer E2E harness

This directory is Nocturne's live protocol-compatibility harness. **Integrators do not need it to
use the SDK.** It exists for maintainers changing hashes, codecs, math, validation, submission, or
contract-facing workflows.

The harness deploys real `Midnight`, `MidnightBundlesV1`, `EcrecoverRatifier`,
`EcrecoverAuthorizer`, ERC-20, and oracle contracts to a fresh Anvil node. No contract stub accepts
Nocturne output on faith: the deployed contracts execute the SDK-produced signatures, proofs, and
calldata.

## Requirements

- Rust 1.90 or newer.
- Foundry commands `anvil`, `forge`, `cast` on `PATH`.
- An otherwise idle local port `8545`.
- A compatible `morpho-org/midnight` checkout. The last validated revision is `e6f2bf28`.
- A compatible `morpho-org/bundler3` checkout. The last validated revision is `9c457e9`.

The scripts temporarily copy one deployment script into `<MIDNIGHT_REPO>/script`, refuse to
overwrite an existing file, and remove the copy on exit. Generated Foundry broadcast data is
written under this workspace's `target/` directory.

## Full offer lifecycle

From the Nocturne repository root:

```sh
MIDNIGHT_REPO=/path/to/midnight \
BUNDLES_REPO=/path/to/bundler3 \
crates/nocturne/e2e/run.sh
```

The lifecycle proves:

1. `MarketBuilder` output creates a market on the deployed Midnight contract.
2. Rust builds an offer, canonical tree, Merkle proof, Ecrecover signature, authorization, and
   `take` calldata.
3. `sign_authorization` output is accepted by the real `EcrecoverAuthorizer`.
4. The real `Midnight.take` accepts the SDK-built offer, ratifier data, and calldata.
5. Credit, debt, consumption, token transfers, and a non-zero settlement fee match
   `simulate_take` predictions.
6. A take sized with `seller_assets_to_units` yields the predicted target assets.
7. An inaccessible tick is rejected both by `validate_offer` and the contract.
8. Live RPC discovery finds missing token approval and bundle authorization, then clears both after
   the generated prerequisite transactions execute.
9. Direct collateral supply and atomic collateral-supply-plus-borrow calldata execute with exact
   expected token, debt, and collateral deltas.
10. A full repayment withdraws all borrower collateral, and redemption transfers all maker credit.
11. Decoded market and position views match live contract getters.
12. SDK-built cancellation calldata invalidates the root and prevents another take.

The runner prints one `PASS` per assertion and exits non-zero on the first mismatch.

## Market-maker cancel-and-replace lifecycle

```sh
MIDNIGHT_REPO=/path/to/midnight crates/nocturne/e2e/mm.sh
```

This harness fixes Anvil's timestamp so the market has exactly one year to maturity, then:

1. Quotes a four-rung lend grid from APR targets.
2. Covers the full grid with one signed tree.
3. Executes full and partial fills and checks consumption and inventory.
4. Moves fair value and creates a second tree.
5. Cancels the first root and proves stale quotes revert.
6. Fills the replacement grid and checks every amount against SDK math.

## Files

| File | Role |
|---|---|
| `DeployE2E.s.sol` | Deploys contracts and creates the lifecycle market |
| `e2e.rs` | Generates signed lifecycle artifacts and decodes live return data |
| `run.sh` | Orchestrates deployment, submission, and assertions |
| `DeployMM.s.sol` | Deploys the fixed-time market-maker environment |
| `mm_loop.rs` | Generates APR grids, proofs, takes, and cancellations |
| `mm.sh` | Orchestrates cancel-and-replace and inventory assertions |

## What the harness does not prove

- It is not a professional security audit or formal verification.
- It does not test the public Midnight API, Base mempool availability, RPC reliability, wallet UI,
  reorganization behavior, gas policy, or production key custody.
- It validates the named compatible contracts revision; later contract changes require rerunning
  and reviewing the parity suite.
- Its deterministic Anvil accounts and private keys are public test fixtures and must never hold
  real value.

Unit, property, differential, and fixture tests remain the faster first line of defense. Run those
before the live harness with `cargo test --workspace --all-targets`.
