// SPDX-License-Identifier: GPL-2.0-or-later
// Fixture generator for nocturne-sim tick-price parity.
//
// Prints TickLib.tickToPrice for a spread of ticks; the values are baked into
// tests/sim_parity.rs. Not part of `cargo test`.
//
// Regenerate (from the midnight contracts repo, rev in sim_parity.rs):
//   cp crates/nocturne-offers/fixtures/GenSim.t.sol <midnight>/test/GenSim.t.sol
//   cd <midnight> && forge test --match-contract GenSim -vv
//   # copy the printed (tick, price) pairs into tests/sim_parity.rs, then delete the copy
pragma solidity 0.8.34;

import {Test, console2} from "forge-std/Test.sol";
import {TickLib, MAX_TICK} from "../src/libraries/TickLib.sol";

contract GenSimTest is Test {
    function test_generate() public pure {
        uint256[12] memory ticks =
            [uint256(0), 1, 4, 100, 1000, 3371, 3372, 3373, 5000, 6740, 6743, MAX_TICK];
        for (uint256 i = 0; i < ticks.length; i++) {
            console2.log(ticks[i], TickLib.tickToPrice(ticks[i]));
        }
    }
}
