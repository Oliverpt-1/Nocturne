// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright (c) 2026 Morpho Association
pragma solidity ^0.8.0;

import {Test, console} from "forge-std/Test.sol";
import {TickLib, MAX_TICK} from "../src/libraries/TickLib.sol";

/// Generates contract-parity fixtures for the Rust `price_to_tick` / `apr_to_tick` / `tick_to_price`
/// chain. Copy into the midnight repo's `test/` dir and run:
///   forge test --match-contract GenApr -vv
/// then bake the printed values into `tests/apr.rs`.
contract GenAprTest is Test {
    uint256 constant SPACING = 4;

    function testGenPriceToTick() public pure {
        // A spread of WAD prices (including exact and near-par values) → priceToTick(price, 4).
        uint256[10] memory prices = [
            uint256(1e17), // 0.1
            2e17, // 0.2
            5e17, // 0.5
            7e17, // 0.7
            9e17, // 0.9
            95e16, // 0.95
            99e16, // 0.99
            999e15, // 0.999
            1e18, // 1.0 (par)
            333333333333333333 // ~1/3
        ];
        console.log("=== priceToTick(price, 4) ===");
        for (uint256 i = 0; i < prices.length; i++) {
            uint256 t = TickLib.priceToTick(prices[i], SPACING);
            console.log("price=%s tick=%s price_back=%s", prices[i], t, TickLib.tickToPrice(t));
        }
    }

    function testGenAprChain() public pure {
        // The (apr_pct_scaled_1e6, ttm_secs) inputs the Rust test uses. apr is scaled by 1e6 to
        // print as an integer, then divided in-Rust. We replicate apr_to_tick's price math here so
        // the printed tick/price match the Rust chain exactly.
        // apr_pct in percent, ttm in seconds.
        uint256 SECONDS_PER_YEAR = 31_536_000;
        // (apr_bps_of_percent, ttm) pairs. apr_pct = col0 / 100 (so 500 => 5.00%).
        uint256[2][6] memory cases = [
            [uint256(500), 31_536_000], // 5.00% APR, 1y
            [uint256(1000), 31_536_000], // 10.00% APR, 1y
            [uint256(720), 15_768_000], // 7.20% APR, 0.5y
            [uint256(250), 7_884_000], // 2.50% APR, 0.25y
            [uint256(1500), 2_592_000], // 15.00% APR, 30d
            [uint256(300), 63_072_000] // 3.00% APR, 2y
        ];
        console.log("=== apr -> price -> tick -> price ===");
        for (uint256 i = 0; i < cases.length; i++) {
            uint256 aprHundredths = cases[i][0]; // apr_pct * 100
            uint256 ttm = cases[i][1];
            // term_rate = (apr_pct/100) * (ttm/YEAR); apr_pct = aprHundredths/100.
            // term_rate_wad = aprHundredths * 1e18 * ttm / (100 * 100 * YEAR).
            uint256 termRateWad = (aprHundredths * 1e18 * ttm) / (10000 * SECONDS_PER_YEAR);
            // price_frac = 1/(1+term_rate); price_wad = 1e18 * 1e18 / (1e18 + term_rate_wad).
            uint256 priceWad = (1e18 * 1e18) / (1e18 + termRateWad);
            uint256 tick = TickLib.priceToTick(priceWad, SPACING);
            console.log(
                "aprHundredths=%s ttm=%s priceWad=%s", aprHundredths, ttm, priceWad
            );
            console.log("   tick=%s tickPrice=%s", tick, TickLib.tickToPrice(tick));
        }
    }
}
