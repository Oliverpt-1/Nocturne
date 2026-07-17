# e2e — live anvil test against real Midnight

Deploys a **real** Midnight environment on anvil (no stubs: real `Midnight`,
`EcrecoverRatifier`, `EcrecoverAuthorizer`, real ERC20s, `Oracle`) and drives the full offer
lifecycle using only the `nocturne` tools, asserting the chain accepts every artifact and
that on-chain state matches the tools' predictions.

```sh
# needs anvil + cast + forge (Foundry) on PATH
crates/nocturne/e2e/run.sh
```

Every tool is exercised through the live node (15 checks):

1. deploy real Midnight + ratifier + authorizer + tokens; create the market (**builder**)
2. tools build the offer, sign the tree, sign an `Authorization`, encode calldata (`e2e.rs`)
3. `sign_authorization` → real `EcrecoverAuthorizer.setIsAuthorized` accepts it (**authorize**)
4. tool-built `take` calldata → real `Midnight.take` accepts the signed offer (**hash/tree/proof/sign/verify/codec**)
5. on-chain credit / debt / consumed / transferred assets == predictions, with a **non-zero
   settlement fee** applied (**simulate_take**, fee-bearing)
6. size a take to a target asset amount → the real take yields exactly that (**sizing**)
7. a bad-tick offer is flagged by `validate_offer` and reverts on-chain (**validate**)
8. `decode_market_state` / `decode_position` match the live getters (**decode**)
9. `encode_cancel_root_calldata` → real `cancelRoot`, after which a re-take reverts (**cancel**)

Files: `DeployE2E.s.sol` (Solidity deploy, copied into the contracts repo to run),
`../examples/e2e.rs` (the tool-driven artifact generator), `run.sh` (orchestrator).
The settlement fee is non-zero but the market maturity is far enough out that the fee is flat
(the 360-day breakpoint), so predictions stay deterministic regardless of anvil timing.

## Market-making loop

```sh
crates/nocturne/e2e/mm.sh
```

A full MM lifecycle against real Midnight, driven by the SDK (`../examples/mm_loop.rs`,
`DeployMM.s.sol`): quote a grid of lend offers **by APR** as one signed tree, full + partial
fills, then a fair-value move → re-quote (new tree/root/sig) → cancel-and-replace (old root
reverts `RootCanceled`, new grid fills at the new price), with inventory checked each round and
every fill matched to `take_amounts`. Anvil is pinned to a fixed timestamp so maturity is exactly
one year out, making APR quoting realistic while keeping the fee flat.
