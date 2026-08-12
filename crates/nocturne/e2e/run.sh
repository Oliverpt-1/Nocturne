#!/usr/bin/env bash
# End-to-end test of the nocturne tools against a REAL Midnight deployed on anvil.
# No stubs: real Midnight, EcrecoverRatifier, EcrecoverAuthorizer, real ERC20s.
# Every offer, signature, and calldata comes from the Rust tools; the chain either accepts it
# (proving the tool) or the script fails.
set -euo pipefail

RPC=http://127.0.0.1:8545
PK0=0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80   # taker/seller
PK1=0x59c6995e998f97a5a0044966f0945389dc9e86dae88c7a8412f4603b6b78690d   # maker/lender
ACCOUNT0=0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266
GROUP=0x0000000000000000000000000000000000000000000000000000000000000001

CRATE="$(cd "$(dirname "$0")/.." && pwd)"
MID="${MIDNIGHT_REPO:?set MIDNIGHT_REPO to a morpho-org/midnight checkout at rev f47568c9}"
SCRIPT_DIR="$MID/script"
DEPLOY_SCRIPT="$SCRIPT_DIR/DeployE2E.s.sol"
CREATED_SCRIPT_DIR=0
COPIED_DEPLOY_SCRIPT=0
ANVIL_PID=""
cleanup() {
  if [ "$COPIED_DEPLOY_SCRIPT" -eq 1 ]; then rm -f "$DEPLOY_SCRIPT"; fi
  if [ "$CREATED_SCRIPT_DIR" -eq 1 ]; then rmdir "$SCRIPT_DIR" 2>/dev/null || true; fi
  if [ -n "$ANVIL_PID" ]; then kill "$ANVIL_PID" 2>/dev/null || true; wait "$ANVIL_PID" 2>/dev/null || true; fi
}
trap cleanup EXIT
PASS=0
ok()   { echo "  PASS: $1"; PASS=$((PASS+1)); }
fail() { echo "  FAIL: $1"; exit 1; }
eq()   { local a="${2%% *}" b="${3%% *}"; [ "$a" = "$b" ] && ok "$1 ($a)" || fail "$1: got '$a' expected '$b'"; }

echo "== 1. start anvil + deploy REAL Midnight =="
if [ -e "$DEPLOY_SCRIPT" ]; then fail "refusing to overwrite existing $DEPLOY_SCRIPT"; fi
if (echo >/dev/tcp/127.0.0.1/8545) 2>/dev/null; then fail "port 8545 is already in use"; fi
anvil --port 8545 --silent >/tmp/nocturne-anvil.log 2>&1 &
ANVIL_PID=$!
sleep 3
kill -0 "$ANVIL_PID" 2>/dev/null || fail "anvil failed to start (is port 8545 already in use?)"
if [ ! -d "$SCRIPT_DIR" ]; then mkdir -p "$SCRIPT_DIR"; CREATED_SCRIPT_DIR=1; fi
cp "$CRATE/e2e/DeployE2E.s.sol" "$DEPLOY_SCRIPT"
COPIED_DEPLOY_SCRIPT=1
if ! DEPLOY=$(cd "$MID" && FOUNDRY_BROADCAST="$CRATE/../../target/nocturne-e2e-broadcast" forge script script/DeployE2E.s.sol --tc DeployE2E --rpc-url "$RPC" --broadcast --private-key "$PK0" 2>&1); then
  echo "$DEPLOY" >&2
  fail "deploy"
fi
rm -f "$DEPLOY_SCRIPT"
COPIED_DEPLOY_SCRIPT=0
grep -q "ONCHAIN EXECUTION COMPLETE & SUCCESSFUL" <<<"$DEPLOY" || fail "deploy"
getaddr() { grep -E "^  $1 " <<<"$DEPLOY" | awk '{print $2}'; }
export MIDNIGHT=$(getaddr MIDNIGHT) RATIFIER=$(getaddr RATIFIER) AUTHORIZER=$(getaddr AUTHORIZER)
export LOAN=$(getaddr LOAN) COLLATERAL=$(getaddr COLLATERAL) ORACLE=$(getaddr ORACLE)
MARKET_ID=$(grep -E "^  MARKET_ID " <<<"$DEPLOY" | awk '{print $2}')
ok "deployed real Midnight at $MIDNIGHT (market $MARKET_ID)"

echo "== 2. run the Rust tools (build/sign/authorize/encode/simulate) =="
GEN=$(cd "$CRATE" && cargo run --quiet --example e2e -- gen)
g() { grep -E "^$1 " <<<"$GEN" | awk '{print $2}'; }
MAKER=$(g MAKER); TAKE=$(g TAKE_CALLDATA); CANCEL=$(g CANCEL_CALLDATA)
TAKE2=$(g TAKE2_CALLDATA); BAD_TAKE=$(g BAD_TAKE_CALLDATA)
ok "tools produced offer, signature, and calldata"
eq "validate flagged bad tick (rust)" "$(g VALIDATE_FLAGGED_TICK)" "true"

echo "== 3. hot-key authorization (sign_authorization -> real EcrecoverAuthorizer) =="
AUTH_CALLDATA=$(cast calldata "setIsAuthorized((address,address,bool,uint256,uint256),(uint8,bytes32,bytes32))" \
  "($(g AUTH_AUTHORIZER),$(g AUTH_AUTHORIZED),true,$(g AUTH_NONCE),$(g AUTH_DEADLINE))" \
  "($(g AUTH_V),$(g AUTH_R),$(g AUTH_S))")
cast send "$AUTHORIZER" "$AUTH_CALLDATA" --rpc-url "$RPC" --private-key "$PK0" >/dev/null
eq "ratifier authorized by maker" "$(cast call "$MIDNIGHT" 'isAuthorized(address,address)(bool)' "$MAKER" "$RATIFIER" --rpc-url "$RPC")" "true"

echo "== 4. take the offer with tool-built calldata (real Midnight.take) =="
LOAN_BEFORE=$(cast call "$LOAN" 'balanceOf(address)(uint256)' "$ACCOUNT0" --rpc-url "$RPC" | awk '{print $1}')
cast send "$MIDNIGHT" "$TAKE" --rpc-url "$RPC" --private-key "$PK0" >/dev/null
ok "real take() accepted the tool's signed offer + calldata"

echo "== 5. on-chain state matches simulate_take predictions =="
eq "maker credit"  "$(cast call "$MIDNIGHT" 'credit(bytes32,address)(uint128)' "$MARKET_ID" "$MAKER" --rpc-url "$RPC")"     "$(g PRED_BUYER_CREDIT_INCREASE 2>/dev/null || grep -E '^PRED_BUYER_CREDIT_INCREASE' <<<"$GEN" | awk '{print $2}')"
eq "taker debt"    "$(cast call "$MIDNIGHT" 'debt(bytes32,address)(uint128)' "$MARKET_ID" "$ACCOUNT0" --rpc-url "$RPC")"    "$(grep -E '^PRED_SELLER_DEBT_INCREASE' <<<"$GEN" | awk '{print $2}')"
eq "group consumed" "$(cast call "$MIDNIGHT" 'consumed(address,bytes32)(uint128)' "$MAKER" "$GROUP" --rpc-url "$RPC")"      "$(grep -E '^PRED_NEW_CONSUMED' <<<"$GEN" | awk '{print $2}')"
LOAN_AFTER=$(cast call "$LOAN" 'balanceOf(address)(uint256)' "$ACCOUNT0" --rpc-url "$RPC" | awk '{print $1}')
eq "seller received assets (with fee)" "$((LOAN_AFTER - LOAN_BEFORE))" "$(grep -E '^PRED_SELLER_ASSETS ' <<<"$GEN" | awk '{print $2}')"

echo "== 5b. sizing: take sized to a target asset amount (seller_assets_to_units) =="
SZ_BEFORE=$(cast call "$LOAN" 'balanceOf(address)(uint256)' "$ACCOUNT0" --rpc-url "$RPC" | awk '{print $1}')
cast send "$MIDNIGHT" "$TAKE2" --rpc-url "$RPC" --private-key "$PK0" >/dev/null
SZ_AFTER=$(cast call "$LOAN" 'balanceOf(address)(uint256)' "$ACCOUNT0" --rpc-url "$RPC" | awk '{print $1}')
eq "sized take yielded predicted assets" "$((SZ_AFTER - SZ_BEFORE))" "$(grep -E '^PRED_SELLER_ASSETS2 ' <<<"$GEN" | awk '{print $2}')"

echo "== 5c. validate: a bad-tick offer must revert on-chain (TickNotAccessible) =="
if cast call "$MIDNIGHT" "$BAD_TAKE" --from "$ACCOUNT0" --rpc-url "$RPC" >/dev/null 2>&1; then
  fail "bad-tick take should have reverted"
else
  ok "bad-tick take reverted on-chain (as validate_offer predicted)"
fi

echo "== 6. decode real on-chain state with the decoder tool =="
MS=$(cast call "$MIDNIGHT" 'marketState(bytes32)' "$MARKET_ID" --rpc-url "$RPC")
DM=$(cd "$CRATE" && cargo run --quiet --example e2e -- decode-market "$MS")
eq "decoded tick_spacing"   "$(grep TICK_SPACING <<<"$DM" | awk '{print $2}')" "4"
eq "decoded continuous_fee" "$(grep CONTINUOUS_FEE <<<"$DM" | awk '{print $2}')" "0"
POS=$(cast call "$MIDNIGHT" 'position(bytes32,address)' "$MARKET_ID" "$MAKER" --rpc-url "$RPC")
DP=$(cd "$CRATE" && cargo run --quiet --example e2e -- decode-position "$POS")
CREDIT_ONCHAIN=$(cast call "$MIDNIGHT" 'credit(bytes32,address)(uint128)' "$MARKET_ID" "$MAKER" --rpc-url "$RPC" | awk '{print $1}')
eq "decoded maker credit == getter" "$(grep CREDIT <<<"$DP" | awk '{print $2}')" "$CREDIT_ONCHAIN"

echo "== 7. cancel the root (encode_cancel_root_calldata) then a re-take must revert =="
cast send "$RATIFIER" "$CANCEL" --rpc-url "$RPC" --private-key "$PK1" >/dev/null
if cast call "$MIDNIGHT" "$TAKE" --from "$ACCOUNT0" --rpc-url "$RPC" >/dev/null 2>&1; then
  fail "re-take after cancel should have reverted"
else
  ok "re-take reverted after cancelRoot (RootCanceled)"
fi

echo ""
echo "ALL $PASS CHECKS PASSED against real Midnight on anvil."
