// SPDX-License-Identifier: GPL-2.0-or-later
// Fixture generator for nocturne-offers ABI calldata codec parity.
//
// This is NOT part of `cargo test`. It is the authoritative oracle for the constants baked into
// `tests/codec.rs`. It builds a concrete Offer + Signature + Merkle proof + take params and prints
// the exact bytes Solidity produces for:
//   - abi.encode(Signature, root, leafIndex, proof)                          (ratifierData)
//   - abi.encodeCall(IMidnight.take, (offer, ratifierData, units, ...))      (full take calldata)
//   - abi.encodeCall(IEcrecoverRatifier.cancelRoot, (maker, root))
//   - the take / cancelRoot 4-byte selectors
//
// The signature/root/proof are arbitrary fixed bytes: the codec only packs them, it does not
// validate them, so no signing or tree building is needed here.
//
// Regenerate (from the midnight contracts repo, rev in tests/codec.rs):
//   cp crates/nocturne-offers/fixtures/GenCodec.t.sol <midnight>/test/GenCodec.t.sol
//   cd <midnight> && forge test --match-contract GenCodec -vv
//   # copy the printed constants into crates/nocturne-offers/tests/codec.rs, then delete the copy
pragma solidity 0.8.34;

import {Test, console2} from "forge-std/Test.sol";
import {IMidnight, Offer, Market, CollateralParams} from "../src/interfaces/IMidnight.sol";
import {IEcrecoverRatifier} from "../src/ratifiers/interfaces/IEcrecoverRatifier.sol";
import {Signature} from "../src/ratifiers/interfaces/IEcrecoverRatifier.sol";

contract GenCodecTest is Test {
    address constant MIDNIGHT = 0x1111111111111111111111111111111111111111;
    address constant LOAN = 0x2222222222222222222222222222222222222222;
    address constant CP0_TOKEN = 0x3333333333333333333333333333333333333333;
    address constant CP0_ORACLE = 0x4444444444444444444444444444444444444444;
    address constant CP1_TOKEN = 0x5555555555555555555555555555555555555555;
    address constant CP1_ORACLE = 0x6666666666666666666666666666666666666666;
    address constant ENTER_GATE = 0x7777777777777777777777777777777777777777;
    address constant LIQ_GATE = 0x8888888888888888888888888888888888888888;
    address constant CALLBACK = 0x9999999999999999999999999999999999999999;
    address constant RECEIVER_MAKER = address(uint160(0x00aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa));
    address constant RATIFIER = address(uint160(0x00bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb));
    address constant TAKER = address(uint160(0x00cccccccccccccccccccccccccccccccccccccccc));
    address constant RECEIVER_TAKER = address(uint160(0x00dddddddddddddddddddddddddddddddddddddddd));
    address constant TAKER_CALLBACK = address(uint160(0x00eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee));
    address constant MAKER = 0x1234567890123456789012345678901234567890;

    uint8 constant SIG_V = 28;
    bytes32 constant SIG_R = 0x1111111111111111111111111111111111111111111111111111111111111111;
    bytes32 constant SIG_S = 0x2222222222222222222222222222222222222222222222222222222222222222;
    bytes32 constant ROOT = 0x3333333333333333333333333333333333333333333333333333333333333333;
    uint256 constant LEAF_INDEX = 2;
    bytes32 constant PROOF0 = 0x4444444444444444444444444444444444444444444444444444444444444444;
    bytes32 constant PROOF1 = 0x5555555555555555555555555555555555555555555555555555555555555555;

    uint256 constant UNITS = 250_000;

    function makeMarket() internal pure returns (Market memory m) {
        CollateralParams[] memory cps = new CollateralParams[](2);
        cps[0].token = CP0_TOKEN;
        cps[0].lltv = 0.86e18;
        cps[0].liquidationCursor = 1;
        cps[0].oracle = CP0_ORACLE;
        cps[1].token = CP1_TOKEN;
        cps[1].lltv = 0.915e18;
        cps[1].liquidationCursor = 2;
        cps[1].oracle = CP1_ORACLE;
        m.chainId = 1;
        m.midnight = MIDNIGHT;
        m.loanToken = LOAN;
        m.collateralParams = cps;
        m.maturity = 1_800_000_000;
        m.rcfThreshold = 1000;
        m.enterGate = ENTER_GATE;
        m.liquidatorGate = LIQ_GATE;
    }

    function makeOffer() internal pure returns (Offer memory o) {
        o.market = makeMarket();
        o.buy = true;
        o.maker = MAKER;
        o.start = 1;
        o.expiry = 2_000_000_000;
        o.tick = 42;
        o.group = bytes32(uint256(7));
        o.callback = CALLBACK;
        o.callbackData = hex"deadbeef";
        o.receiverIfMakerIsSeller = RECEIVER_MAKER;
        o.ratifier = RATIFIER;
        o.reduceOnly = true;
        o.maxUnits = 1_000_000;
        o.maxAssets = 500_000;
        o.continuousFeeCap = 123_456;
    }

    function makeProof() internal pure returns (bytes32[] memory proof) {
        proof = new bytes32[](2);
        proof[0] = PROOF0;
        proof[1] = PROOF1;
    }

    function makeRatifierData() internal pure returns (bytes memory) {
        return abi.encode(Signature(SIG_V, SIG_R, SIG_S), ROOT, LEAF_INDEX, makeProof());
    }

    /// Isolated so the deep ABI-encode of the Offer gets its own stack frame.
    function makeTakeCalldata(bytes memory ratifierData) internal pure returns (bytes memory) {
        return abi.encodeCall(
            IMidnight.take,
            (makeOffer(), ratifierData, UNITS, TAKER, RECEIVER_TAKER, TAKER_CALLBACK, hex"cafe")
        );
    }

    function test_generate() public pure {
        bytes memory ratifierData = makeRatifierData();
        bytes memory takeCalldata = makeTakeCalldata(ratifierData);
        bytes memory cancelCalldata = abi.encodeCall(IEcrecoverRatifier.cancelRoot, (MAKER, ROOT));

        console2.log("ratifierData:");
        console2.logBytes(ratifierData);
        console2.log("takeCalldata:");
        console2.logBytes(takeCalldata);
        console2.log("cancelRootCalldata:");
        console2.logBytes(cancelCalldata);
        console2.log("takeSelector:");
        console2.logBytes32(bytes32(IMidnight.take.selector));
        console2.log("cancelRootSelector:");
        console2.logBytes32(bytes32(IEcrecoverRatifier.cancelRoot.selector));
    }
}
