#!/usr/bin/env bash
# Market-making loop against a REAL Midnight on anvil, driven entirely by the SDK.
# Proves: grid quoting (one signed tree, many rungs), full + partial fills, fair-value move ->
# re-quote (new tree/root/sig), cancel-and-replace (old root dead, new live), and inventory
# accounting — every fill checked against the SDK's simulate/take_amounts predictions.
set -euo pipefail

RPC=http://127.0.0.1:8545
PK0=0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80   # taker
PK1=0x59c6995e998f97a5a0044966f0945389dc9e86dae88c7a8412f4603b6b78690d   # maker
ACCOUNT0=0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266

CRATE="$(cd "$(dirname "$0")/.." && pwd)"
MID="${MIDNIGHT_REPO:?set MIDNIGHT_REPO to a morpho-org/midnight checkout at rev f47568c9}"
PASS=0
ok()   { echo "  PASS: $1"; PASS=$((PASS+1)); }
fail() { echo "  FAIL: $1"; exit 1; }
eq()   { local a="${2%% *}" b="${3%% *}"; [ "$a" = "$b" ] && ok "$1 ($a)" || fail "$1: got '$a' expected '$b'"; }
n()    { awk '{print $1}'; }
loanbal() { cast call "$LOAN" 'balanceOf(address)(uint256)' "$1" --rpc-url "$RPC" | n; }
credit()  { cast call "$MIDNIGHT" 'credit(bytes32,address)(uint128)' "$MARKET_ID" "$1" --rpc-url "$RPC" | n; }
debt()    { cast call "$MIDNIGHT" 'debt(bytes32,address)(uint128)' "$MARKET_ID" "$1" --rpc-url "$RPC" | n; }
consumed(){ cast call "$MIDNIGHT" 'consumed(address,bytes32)(uint128)' "$1" "$(printf '0x%064x' "$2")" --rpc-url "$RPC" | n; }
send()    { cast send "$1" "$2" --rpc-url "$RPC" --private-key "$3" >/dev/null; }

echo "== deploy REAL Midnight for the MM loop =="
pkill -f "anvil --port 8545" 2>/dev/null || true; sleep 1
# fixed genesis timestamp so "1 year to maturity" is exact -> realistic APR quoting
anvil --port 8545 --timestamp 1000000000 --silent >/tmp/nocturne-mm-anvil.log 2>&1 &
sleep 3
cp "$CRATE/e2e/DeployMM.s.sol" "$MID/script/DeployMM.s.sol"
D=$(cd "$MID" && forge script script/DeployMM.s.sol --tc DeployMM --rpc-url "$RPC" --broadcast --private-key "$PK0" 2>&1)
rm -f "$MID/script/DeployMM.s.sol"
grep -q "ONCHAIN EXECUTION COMPLETE & SUCCESSFUL" <<<"$D" || fail "deploy"
ga() { grep -E "^  $1 " <<<"$D" | awk '{print $2}'; }
export MIDNIGHT=$(ga MIDNIGHT) RATIFIER=$(ga RATIFIER) LOAN=$(ga LOAN) COLLATERAL=$(ga COLLATERAL) ORACLE=$(ga ORACLE)
MARKET_ID=$(grep -E "^  MARKET_ID " <<<"$D" | awk '{print $2}')
ok "deployed real Midnight ($MIDNIGHT)"

run_grid() { cd "$CRATE" && cargo run --quiet --example mm_loop -- grid "$1" "$2"; }

echo "== round 1: quote a grid BY APR (one signed tree, 4 rungs) @ fair 10% =="
G1=$(run_grid 10 100)
g1() { grep -E "^$1 " <<<"$G1" | awk '{print $2}'; }
MAKER=$(g1 MAKER)
ok "grid quoted by APR, root $(g1 ROOT)"
for i in 0 1 2 3; do echo "     rung$i: asked $(g1 R${i}_APR)% -> tick $(g1 R${i}_TICK) -> realized $(g1 R${i}_REALIZED_APR)%"; done
# realized APR must be <= asked (apr_to_tick snaps to a not-worse price for the maker)
awk -v a="$(g1 R0_APR)" -v r="$(g1 R0_REALIZED_APR)" 'BEGIN{ exit !(r<=a+0.0001) }' \
  && ok "realized APR <= asked (rung0)" || fail "realized APR > asked"

echo "== round 1: full fill on rung0, partial (x2) on rung1 =="
LB=$(loanbal "$ACCOUNT0"); send "$MIDNIGHT" "$(g1 R0_TAKE_FULL)" "$PK0"
eq "rung0 full: seller assets" "$(( $(loanbal "$ACCOUNT0") - LB ))" "$(g1 R0_FULL_SELLER_ASSETS)"
eq "rung0 consumed" "$(consumed "$MAKER" 100)" "$(g1 FULL)"
send "$MIDNIGHT" "$(g1 R1_TAKE_PARTIAL)" "$PK0"; send "$MIDNIGHT" "$(g1 R1_TAKE_PARTIAL)" "$PK0"
eq "rung1 consumed after 2 partials" "$(consumed "$MAKER" 101)" "800000"
eq "maker credit (inventory)" "$(credit "$MAKER")" "1800000"
eq "taker debt (inventory)"   "$(debt "$ACCOUNT0")" "1800000"

echo "== round 2: fair value moves (10% -> 8%) -> re-quote, cancel & replace =="
G2=$(run_grid 8 200)
g2() { grep -E "^$1 " <<<"$G2" | awk '{print $2}'; }
send "$RATIFIER" "$(g1 CANCEL)" "$PK1"
if cast call "$MIDNIGHT" "$(g1 R2_TAKE_FULL)" --from "$ACCOUNT0" --rpc-url "$RPC" >/dev/null 2>&1; then
  fail "stale grid-1 rung should revert after cancel"
else
  ok "stale grid-1 rung reverts after cancelRoot (RootCanceled)"
fi
LB=$(loanbal "$ACCOUNT0"); send "$MIDNIGHT" "$(g2 R0_TAKE_FULL)" "$PK0"
eq "new grid rung0: seller assets" "$(( $(loanbal "$ACCOUNT0") - LB ))" "$(g2 R0_FULL_SELLER_ASSETS)"
eq "maker credit after re-quote" "$(credit "$MAKER")" "2800000"
eq "taker debt after re-quote"   "$(debt "$ACCOUNT0")" "2800000"

pkill -f "anvil --port 8545" 2>/dev/null || true
echo ""
echo "MM LOOP: ALL $PASS CHECKS PASSED against real Midnight on anvil."
