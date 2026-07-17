// SPDX-License-Identifier: GPL-2.0-or-later
// E2E deploy: a REAL Morpho Midnight environment on anvil - real Midnight, EcrecoverRatifier,
// EcrecoverAuthorizer, a real (mintable) ERC20 loan+collateral, and an Oracle. No stubs.
// Fees left at zero so the Rust tools' amount predictions are deterministic regardless of anvil
// timing. Deploys, configures, funds, creates the market, and logs every address for the driver.
//
// Run: copied into <midnight>/script/, then
//   forge script script/DeployE2E.s.sol --rpc-url http://127.0.0.1:8545 --broadcast \
//     --private-key 0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80
pragma solidity ^0.8.0;

import {Script, console2} from "../lib/forge-std/src/Script.sol";
import {Midnight} from "../src/Midnight.sol";
import {Market, CollateralParams} from "../src/interfaces/IMidnight.sol";
import {EcrecoverRatifier} from "../src/ratifiers/EcrecoverRatifier.sol";
import {EcrecoverAuthorizer} from "../src/periphery/EcrecoverAuthorizer.sol";
import {WAD, ORACLE_PRICE_SCALE} from "../src/libraries/ConstantsLib.sol";
import {IdLib} from "../src/libraries/IdLib.sol";
import {Oracle} from "../test/helpers/Oracle.sol";

contract MintableERC20 {
    string public name;
    string public symbol;
    uint8 public constant decimals = 18;
    uint256 public totalSupply;
    mapping(address => uint256) public balanceOf;
    mapping(address => mapping(address => uint256)) public allowance;

    constructor(string memory _n, string memory _s) { name = _n; symbol = _s; }
    function mint(address to, uint256 a) external { balanceOf[to] += a; totalSupply += a; }
    function approve(address s, uint256 a) external returns (bool) { allowance[msg.sender][s] = a; return true; }
    function transfer(address to, uint256 a) external returns (bool) {
        balanceOf[msg.sender] -= a; balanceOf[to] += a; return true;
    }
    function transferFrom(address f, address to, uint256 a) external returns (bool) {
        allowance[f][msg.sender] -= a; balanceOf[f] -= a; balanceOf[to] += a; return true;
    }
}

contract DeployE2E is Script {
    // anvil default accounts
    uint256 constant PK0 = 0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80;
    uint256 constant PK1 = 0x59c6995e998f97a5a0044966f0945389dc9e86dae88c7a8412f4603b6b78690d;
    address constant ACCOUNT0 = 0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266; // taker / seller / borrower
    address constant ACCOUNT1 = 0x70997970C51812dc3A010C7d01b50e0d17dc79C8; // maker / buyer / lender

    uint256 constant LLTV = 0.77e18;
    uint256 constant CURSOR = 0.3e18;
    uint256 constant MATURITY = 4_000_000_000; // fixed, far future
    uint256 constant UNITS = 1_000_000;

    function run() external {
        vm.startBroadcast(PK0);
        Midnight midnight = new Midnight();
        EcrecoverRatifier ratifier = new EcrecoverRatifier(address(midnight));
        EcrecoverAuthorizer authorizer = new EcrecoverAuthorizer(address(midnight));
        MintableERC20 loan = new MintableERC20("loan", "LOAN");
        MintableERC20 collat = new MintableERC20("collat", "COLL");
        Oracle oracle = new Oracle();
        oracle.setPrice(ORACLE_PRICE_SCALE); // par

        midnight.setFeeSetter(ACCOUNT0);
        midnight.setTickSpacingSetter(ACCOUNT0);
        midnight.enableLiquidationCursor(CURSOR);
        midnight.enableLltv(LLTV);
        // non-zero settlement-fee curve (on-chain max per index) so the fee-bearing take math and
        // sizing are exercised live; continuous fee left at zero.
        uint256 CBP = 1e12;
        uint16[7] memory cbps = [uint16(14), 14, 98, 417, 1250, 2500, 5000];
        for (uint256 i = 0; i <= 6; i++) {
            midnight.setDefaultSettlementFee(address(loan), i, uint256(cbps[i]) * CBP);
        }

        CollateralParams[] memory cp = new CollateralParams[](1);
        cp[0] = CollateralParams({token: address(collat), lltv: LLTV, liquidationCursor: CURSOR, oracle: address(oracle)});
        Market memory market = Market({
            chainId: block.chainid,
            midnight: address(midnight),
            loanToken: address(loan),
            collateralParams: cp,
            maturity: MATURITY,
            rcfThreshold: 0,
            enterGate: address(0),
            liquidatorGate: address(0)
        });

        // fund + collateralize the taker (seller/borrower); create the market via supplyCollateral
        uint256 collAmt = (((UNITS * WAD + LLTV - 1) / LLTV) * ORACLE_PRICE_SCALE + ORACLE_PRICE_SCALE - 1)
            / ORACLE_PRICE_SCALE * 4; // ~4x the healthy minimum (covers multiple takes)
        collat.mint(ACCOUNT0, collAmt);
        collat.approve(address(midnight), collAmt);
        midnight.supplyCollateral(market, 0, collAmt, ACCOUNT0);
        vm.stopBroadcast();

        // fund + approve the maker (buyer/lender), and enable the signature-authorizer contract
        // as an operator (one-time bootstrap so EcrecoverAuthorizer can act on the maker's behalf)
        vm.startBroadcast(PK1);
        loan.mint(ACCOUNT1, UNITS * 10);
        loan.approve(address(midnight), type(uint256).max);
        midnight.setIsAuthorized(address(authorizer), true, ACCOUNT1);
        vm.stopBroadcast();

        console2.log("MIDNIGHT", address(midnight));
        console2.log("RATIFIER", address(ratifier));
        console2.log("AUTHORIZER", address(authorizer));
        console2.log("LOAN", address(loan));
        console2.log("COLLATERAL", address(collat));
        console2.log("ORACLE", address(oracle));
        console2.log("MARKET_ID", vm.toString(IdLib.toId(market)));
    }
}
