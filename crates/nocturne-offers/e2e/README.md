# e2e — live anvil test against real Midnight

Deploys a **real** Midnight environment on anvil (no stubs: real `Midnight`,
`EcrecoverRatifier`, `EcrecoverAuthorizer`, real ERC20s, `Oracle`) and drives the full offer
lifecycle using only the `nocturne-offers` tools, asserting the chain accepts every artifact and
that on-chain state matches the tools' predictions.

```sh
# needs anvil + cast + forge (Foundry) on PATH
crates/nocturne-offers/e2e/run.sh
```

What it checks, end to end:

1. deploy real Midnight + ratifier + authorizer + tokens; create the market
2. tools build the offer, sign the tree, sign an `Authorization`, encode calldata (`e2e.rs`)
3. `sign_authorization` → real `EcrecoverAuthorizer.setIsAuthorized` accepts it
4. tool-built `take` calldata → real `Midnight.take` accepts the signed offer
5. on-chain credit / debt / consumed / transferred assets == `simulate_take` predictions
6. `decode_market_state` / `decode_position` decode the live on-chain state correctly
7. `encode_cancel_root_calldata` → real `cancelRoot`, after which a re-take reverts

Files: `DeployE2E.s.sol` (Solidity deploy, copied into the contracts repo to run),
`../examples/e2e.rs` (the tool-driven artifact generator), `run.sh` (orchestrator).
Fees are left at zero so predictions are deterministic regardless of anvil timing; the fee math
itself is parity-checked separately in `tests/sim_take_parity.rs`.
