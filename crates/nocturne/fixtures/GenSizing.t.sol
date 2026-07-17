// SPDX-License-Identifier: GPL-2.0-or-later
// Fixture generator for nocturne sizing parity.
//
// Calls the real TakeAmountsLib.buyerAssetsToUnits / sellerAssetsToUnits and
// ConsumableUnitsLib.consumableUnits against a live market (rev in tests/sizing.rs) and prints the
// results. The values are baked into tests/sizing.rs, which asserts buyer_assets_to_units /
// seller_assets_to_units / consumable_units reproduce them - promoting the inverse sizing math from
// hand-verified to contract-anchored, the same standard as sim_take_parity.rs.
//
// Extends the contracts' own BaseTest to inherit market setup, tokens, and the DummyRatifier.
//
// Regenerate (from the midnight contracts repo, rev in sizing.rs):
//   cp crates/nocturne/fixtures/GenSizing.t.sol <midnight>/test/GenSizing.t.sol
//   cd <midnight> && forge test --match-contract GenSizing -vv
//   # copy the printed constants into tests/sizing.rs, then delete the copy
pragma solidity ^0.8.0;

import {console2} from "../lib/forge-std/src/Test.sol";
import {BaseTest, LLTV, LIQUIDATION_CURSOR} from "./BaseTest.sol";
import {Market, Offer, CollateralParams} from "../src/interfaces/IMidnight.sol";
import {TakeAmountsLib} from "../src/periphery/TakeAmountsLib.sol";
import {ConsumableUnitsLib} from "../src/periphery/ConsumableUnitsLib.sol";

contract GenSizingTest is BaseTest {
    uint256 constant CBP = 1e12;
    uint256 constant TICK = 3372; // price 0.5 WAD
    uint256 constant NOW = 1_000_000;
    uint256 constant TTM = 45 days; // interpolates between the 30d and 90d fee breakpoints
    uint256 constant TARGET = 500_000; // target asset amount for the *_assets_to_units calls
    uint128 constant MAX_ASSETS = 1_000_000; // assets cap for the consumableUnits assets-capped cases
    uint128 constant MAX_UNITS_CAP = 4_000_000; // units cap for the consumableUnits units-capped case
    uint32 constant CONT_FEE = 100_000_000;
    uint16[7] CBPS = [uint16(14), 14, 98, 417, 1250, 2500, 5000];

    bytes32 constant GROUP = keccak256("nocturne-sizing");

    function market() internal view returns (Market memory m) {
        CollateralParams[] memory cps = new CollateralParams[](1);
        cps[0] = CollateralParams(address(collateralToken1), LLTV, LIQUIDATION_CURSOR, address(oracle1));
        m.chainId = block.chainid;
        m.midnight = address(midnight);
        m.loanToken = address(loanToken);
        m.collateralParams = cps;
        m.maturity = NOW + TTM;
    }

    function baseOffer(Market memory m) internal view returns (Offer memory o) {
        o.market = m;
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
        uint256 fee = midnight.settlementFee(id, TTM);

        Offer memory buyOffer = baseOffer(m);
        buyOffer.buy = true;

        Offer memory sellOffer = baseOffer(m);
        sellOffer.buy = false;

        // assets -> units for both sides of both offer directions.
        uint256 buyBuyerUnits = TakeAmountsLib.buyerAssetsToUnits(address(midnight), id, buyOffer, TARGET);
        uint256 buySellerUnits = TakeAmountsLib.sellerAssetsToUnits(address(midnight), id, buyOffer, TARGET);
        uint256 sellBuyerUnits = TakeAmountsLib.buyerAssetsToUnits(address(midnight), id, sellOffer, TARGET);
        uint256 sellSellerUnits = TakeAmountsLib.sellerAssetsToUnits(address(midnight), id, sellOffer, TARGET);

        // consumableUnits (consumed = 0 on a fresh market).
        Offer memory unitsCapped = baseOffer(m);
        unitsCapped.buy = true;
        unitsCapped.maxUnits = MAX_UNITS_CAP;
        uint256 consUnitsCapped = ConsumableUnitsLib.consumableUnits(address(midnight), id, unitsCapped);

        Offer memory assetsCappedBuy = baseOffer(m);
        assetsCappedBuy.buy = true;
        assetsCappedBuy.maxUnits = 0;
        assetsCappedBuy.maxAssets = MAX_ASSETS;
        uint256 consAssetsBuy = ConsumableUnitsLib.consumableUnits(address(midnight), id, assetsCappedBuy);

        Offer memory assetsCappedSell = baseOffer(m);
        assetsCappedSell.buy = false;
        assetsCappedSell.maxUnits = 0;
        assetsCappedSell.maxAssets = MAX_ASSETS;
        uint256 consAssetsSell = ConsumableUnitsLib.consumableUnits(address(midnight), id, assetsCappedSell);

        console2.log("now                 ", NOW);
        console2.log("maturity            ", m.maturity);
        console2.log("tick                ", TICK);
        console2.log("settlement_fee      ", fee);
        console2.log("target              ", TARGET);
        console2.log("max_assets          ", uint256(MAX_ASSETS));
        console2.log("max_units_cap       ", uint256(MAX_UNITS_CAP));
        console2.log("buy_buyerAssetsUnits", buyBuyerUnits);
        console2.log("buy_sellerAssetsUnit", buySellerUnits);
        console2.log("sell_buyerAssetsUnit", sellBuyerUnits);
        console2.log("sell_sellerAssetsUni", sellSellerUnits);
        console2.log("cons_units_capped   ", consUnitsCapped);
        console2.log("cons_assets_buy     ", consAssetsBuy);
        console2.log("cons_assets_sell    ", consAssetsSell);
    }
}
