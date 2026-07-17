// SPDX-License-Identifier: GPL-2.0-or-later
// Fixture generator for nocturne-sim take parity.
//
// Runs a real `Midnight.take` through the full contract and reads back the resulting amounts and
// position deltas. The values are baked into tests/sim_take_parity.rs, which asserts
// take_amounts / simulate_take reproduce them - promoting the take math from hand-verified to
// contract-anchored.
//
// Extends the contracts' own BaseTest to inherit market setup, tokens, the DummyRatifier, and the
// collateralize / take helpers.
//
// Regenerate (from the midnight contracts repo, rev in sim_take_parity.rs):
//   cp crates/nocturne/fixtures/GenTake.t.sol <midnight>/test/GenTake.t.sol
//   cd <midnight> && forge test --match-contract GenTake -vv
//   # copy the printed constants into tests/sim_take_parity.rs, then delete the copy
pragma solidity ^0.8.0;

import {console2} from "../lib/forge-std/src/Test.sol";
import {BaseTest, LLTV, LIQUIDATION_CURSOR} from "./BaseTest.sol";
import {Market, Offer, CollateralParams} from "../src/interfaces/IMidnight.sol";

contract GenTakeTest is BaseTest {
    uint256 constant CBP = 1e12;
    uint256 constant UNITS = 1_000_000;
    uint256 constant TICK = 3372; // price 0.5 WAD, multiple of DEFAULT_TICK_SPACING
    uint256 constant NOW = 1_000_000;
    uint256 constant TTM = 45 days; // interpolates between the 30d and 90d fee breakpoints
    uint32 constant CONT_FEE = 100_000_000; // < MAX_CONTINUOUS_FEE
    // Settlement fee curve = the on-chain per-index maxima (all valid), in cbp.
    uint16[7] CBPS = [uint16(14), 14, 98, 417, 1250, 2500, 5000];

    bytes32 constant GROUP = keccak256("nocturne-sim-take");

    function market() internal view returns (Market memory m) {
        CollateralParams[] memory cps = new CollateralParams[](1);
        cps[0] = CollateralParams(address(collateralToken1), LLTV, LIQUIDATION_CURSOR, address(oracle1));
        m.chainId = block.chainid;
        m.midnight = address(midnight);
        m.loanToken = address(loanToken);
        m.collateralParams = cps;
        m.maturity = NOW + TTM;
    }

    function offer(Market memory m) internal view returns (Offer memory o) {
        o.market = m;
        o.buy = true;
        o.maker = lender;
        o.tick = TICK;
        o.group = GROUP;
        o.ratifier = address(dummyRatifier);
        o.maxUnits = type(uint128).max;
        o.continuousFeeCap = type(uint256).max;
        o.expiry = NOW + 200;
    }

    function test_generate() public {
        vm.warp(NOW);
        for (uint256 i = 0; i <= 6; i++) {
            midnight.setDefaultSettlementFee(address(loanToken), i, uint256(CBPS[i]) * CBP);
        }
        midnight.setDefaultContinuousFee(address(loanToken), CONT_FEE);

        Market memory m = market();
        bytes32 id = midnight.touchMarket(m);
        Offer memory o = offer(m);

        // Fund the buyer (maker/lender) and collateralize the seller (taker/borrower).
        deal(address(loanToken), lender, type(uint128).max);
        collateralize(m, borrower, UNITS * 2); // over-collateralize so the seller stays healthy

        uint256 lenderBefore = loanToken.balanceOf(lender);
        uint256 borrowerBefore = loanToken.balanceOf(borrower);

        take(UNITS, borrower, o);

        uint256 buyerAssets = lenderBefore - loanToken.balanceOf(lender);
        uint256 sellerAssets = loanToken.balanceOf(borrower) - borrowerBefore;

        (uint128 buyerCredit, uint128 buyerPendingFee,,,,) = midnight.position(id, lender);
        (uint128 sellerCredit, uint128 sellerPendingFee,, , uint128 sellerDebt,) = midnight.position(id, borrower);
        uint256 newConsumed = midnight.consumed(lender, GROUP);
        uint256 fee = midnight.settlementFee(id, TTM);

        console2.log("now              ", NOW);
        console2.log("maturity         ", m.maturity);
        console2.log("units            ", UNITS);
        console2.log("tick             ", TICK);
        console2.log("continuous_fee   ", uint256(CONT_FEE));
        console2.log("settlement_fee   ", fee);
        console2.log("maker (lender)   ", lender);
        console2.log("buyer_assets     ", buyerAssets);
        console2.log("seller_assets    ", sellerAssets);
        console2.log("buyer_credit     ", uint256(buyerCredit));
        console2.log("buyer_pending_fee", uint256(buyerPendingFee));
        console2.log("seller_credit    ", uint256(sellerCredit));
        console2.log("seller_debt      ", uint256(sellerDebt));
        console2.log("seller_pending   ", uint256(sellerPendingFee));
        console2.log("new_consumed     ", newConsumed);
        console2.logBytes32(GROUP);
    }
}
