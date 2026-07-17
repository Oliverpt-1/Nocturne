// SPDX-License-Identifier: GPL-2.0-or-later
// Fixture generator for nocturne differential fuzz parity.
//
// Deterministically generates N=32 pseudo-random offers + ticks from the index i and prints,
// per i, HashLib.hashOffer(offer_i) and TickLib.tickToPrice(tick_i). The exact generation rule
// below is mirrored byte-for-byte in `tests/fuzz.rs`, which bakes the printed pairs and asserts
// the Rust `hash_offer` / `tick_to_price` reproduce them all. This broadens the point-parity in
// `parity_e2e.rs` / `sim_parity.rs` to 32 random inputs. Not part of `cargo test`.
//
// Generation rule (identical on both sides):
//   seed  = keccak256(abi.encode(uint256(i)))        // 32-byte big-endian of i, then keccak
//   seed2 = keccak256(abi.encode(seed))              // keccak of the 32 bytes of seed
//   tick  = uint256(seed) % (MAX_TICK + 1)
//   market.chainId          = i + 1
//   market.midnight         = address(uint160(uint256(seed)))
//   market.loanToken        = address(uint160(uint256(seed2)))
//   collateralParams[0].token             = address(uint160(uint256(seed) >> 96))
//   collateralParams[0].lltv              = uint256(seed)
//   collateralParams[0].liquidationCursor = i
//   collateralParams[0].oracle            = address(uint160(uint256(seed2) >> 96))
//   market.maturity        = uint256(seed2) % 4_000_000_000
//   market.rcfThreshold    = i * 1000
//   market.enterGate = market.liquidatorGate = address(0)
//   offer.buy       = (i % 2 == 0)
//   offer.maker     = address(uint160(uint256(seed2) >> 8))
//   offer.start     = 0
//   offer.expiry    = 2_000_000_000 + i
//   offer.tick      = tick
//   offer.group     = seed
//   offer.callback  = address(0)
//   offer.callbackData = first (i % 40) bytes of abi.encodePacked(seed, seed2)  // 64 bytes source
//   offer.receiverIfMakerIsSeller = address(0)
//   offer.ratifier  = address(0)
//   offer.reduceOnly = (i % 3 == 0)
//   offer.maxUnits  = uint128(uint256(seed))
//   offer.maxAssets = 0
//   offer.continuousFeeCap = uint256(seed2)
//
// Regenerate (from the midnight contracts repo, rev f47568c9e45a9b70830b82a130b47393dcafec33):
//   cp crates/nocturne/fixtures/GenFuzz.t.sol <midnight>/test/GenFuzz.t.sol
//   cd <midnight> && forge test --match-contract GenFuzz -vv
//   # copy the printed HASH/PRICE lines into tests/fuzz.rs, then delete the copy
pragma solidity 0.8.34;

import {Test, console2} from "forge-std/Test.sol";
import {HashLib} from "../src/ratifiers/libraries/HashLib.sol";
import {TickLib, MAX_TICK} from "../src/libraries/TickLib.sol";
import {Offer, Market, CollateralParams} from "../src/interfaces/IMidnight.sol";

contract GenFuzzTest is Test {
    uint256 constant N = 32;

    function seedOf(uint256 i) internal pure returns (bytes32) {
        return keccak256(abi.encode(uint256(i)));
    }

    function tickOf(uint256 i) internal pure returns (uint256) {
        return uint256(seedOf(i)) % (MAX_TICK + 1);
    }

    // Split market construction into its own frame to avoid "stack too deep".
    function makeMarket(uint256 i) internal pure returns (Market memory m) {
        bytes32 seed = seedOf(i);
        bytes32 seed2 = keccak256(abi.encode(seed));

        CollateralParams[] memory cps = new CollateralParams[](1);
        cps[0].token = address(uint160(uint256(seed) >> 96));
        cps[0].lltv = uint256(seed);
        cps[0].liquidationCursor = i;
        cps[0].oracle = address(uint160(uint256(seed2) >> 96));

        m.chainId = i + 1;
        m.midnight = address(uint160(uint256(seed)));
        m.loanToken = address(uint160(uint256(seed2)));
        m.collateralParams = cps;
        m.maturity = uint256(seed2) % 4_000_000_000;
        m.rcfThreshold = i * 1000;
        m.enterGate = address(0);
        m.liquidatorGate = address(0);
    }

    function makeCallbackData(uint256 i, bytes32 seed, bytes32 seed2) internal pure returns (bytes memory cd) {
        bytes memory both = abi.encodePacked(seed, seed2); // 64 bytes
        uint256 len = i % 40; // <= 39 < 64
        cd = new bytes(len);
        for (uint256 j = 0; j < len; j++) {
            cd[j] = both[j];
        }
    }

    function makeOffer(uint256 i) internal pure returns (Offer memory o) {
        bytes32 seed = seedOf(i);
        bytes32 seed2 = keccak256(abi.encode(seed));

        o.market = makeMarket(i);
        o.buy = i % 2 == 0;
        o.maker = address(uint160(uint256(seed2) >> 8));
        o.start = 0;
        o.expiry = 2_000_000_000 + i;
        o.tick = tickOf(i);
        o.group = seed;
        o.callback = address(0);
        o.callbackData = makeCallbackData(i, seed, seed2);
        o.receiverIfMakerIsSeller = address(0);
        o.ratifier = address(0);
        o.reduceOnly = i % 3 == 0;
        o.maxUnits = uint128(uint256(seed));
        o.maxAssets = 0;
        o.continuousFeeCap = uint256(seed2);
    }

    function test_generate() public view {
        for (uint256 i = 0; i < N; i++) {
            console2.log("HASH", vm.toString(HashLib.hashOffer(makeOffer(i))));
            console2.log("PRICE", vm.toString(TickLib.tickToPrice(tickOf(i))));
        }
    }
}
