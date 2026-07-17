// SPDX-License-Identifier: GPL-2.0-or-later
// Fixture generator for nocturne decoder parity.
//
// This is NOT part of `cargo test`. It builds a concrete Offer with the exact IMidnight
// structs, prints `abi.encode(offer)` as hex, and bakes that hex into the FIXTURE_OFFER_HEX
// constant in tests/decode.rs so `decode_offer` is checked against the real ABI encoding.
//
// Regenerate (from the midnight contracts repo, pinned at the rev in tests/decode.rs):
//   cp crates/nocturne/fixtures/GenDecode.t.sol <midnight>/test/GenDecode.t.sol
//   cd <midnight> && forge test --match-contract GenDecode -vv
//   # copy the printed hex into FIXTURE_OFFER_HEX in tests/decode.rs, then delete the copy
pragma solidity 0.8.34;

import {Test, console2} from "forge-std/Test.sol";
import {Offer, Market, CollateralParams} from "../src/interfaces/IMidnight.sol";

contract GenDecodeTest is Test {
    // A repeated-byte address (all 20 bytes = `b`), avoiding checksum'd literals.
    function rep(uint8 b) internal pure returns (address) {
        uint160 x;
        for (uint256 i = 0; i < 20; i++) {
            x = (x << 8) | uint160(b);
        }
        return address(x);
    }

    function makeOffer() internal pure returns (Offer memory o) {
        CollateralParams[] memory cps = new CollateralParams[](2);
        cps[0].token = rep(0x33);
        cps[0].lltv = 860_000_000_000_000_000;
        cps[0].liquidationCursor = 1;
        cps[0].oracle = rep(0x44);
        cps[1].token = rep(0x66);
        cps[1].lltv = 900_000_000_000_000_000;
        cps[1].liquidationCursor = 2;
        cps[1].oracle = rep(0x77);

        Market memory m;
        m.chainId = 1;
        m.midnight = rep(0x11);
        m.loanToken = rep(0x22);
        m.collateralParams = cps;
        m.maturity = 1_800_000_000;
        m.rcfThreshold = 1000;
        m.enterGate = rep(0xA1);
        m.liquidatorGate = rep(0xA2);

        o.market = m;
        o.buy = true;
        o.maker = rep(0xAB);
        o.start = 0;
        o.expiry = 2_000_000_000;
        o.tick = 3372;
        o.group = bytes32(uint256(7));
        o.callback = rep(0x44);
        o.callbackData = hex"deadbeef";
        o.receiverIfMakerIsSeller = rep(0x55);
        o.ratifier = rep(0xBB);
        o.reduceOnly = false;
        o.maxUnits = 1_000_000;
        o.maxAssets = 999;
        o.continuousFeeCap = 42;
    }

    function test_generate() public pure {
        Offer memory o = makeOffer();
        console2.log("OFFER_ABI_ENCODE:");
        console2.logBytes(abi.encode(o));
    }
}
