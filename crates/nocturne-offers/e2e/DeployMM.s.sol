// SPDX-License-Identifier: GPL-2.0-or-later
// Deploy for the market-making loop test: real Midnight + EcrecoverRatifier + real ERC20s + Oracle.
// The maker directly authorizes the ratifier (this harness tests quoting/requoting/cancel-replace,
// not the authorization tool), and the taker is heavily over-collateralized for many fills.
pragma solidity ^0.8.0;

import {Script, console2} from "../lib/forge-std/src/Script.sol";
import {Midnight} from "../src/Midnight.sol";
import {Market, CollateralParams} from "../src/interfaces/IMidnight.sol";
import {EcrecoverRatifier} from "../src/ratifiers/EcrecoverRatifier.sol";
import {WAD, ORACLE_PRICE_SCALE} from "../src/libraries/ConstantsLib.sol";
import {IdLib} from "../src/libraries/IdLib.sol";
import {Oracle} from "../test/helpers/Oracle.sol";

contract MintableERC20 {
    string public name; string public symbol; uint8 public constant decimals = 18;
    uint256 public totalSupply; mapping(address => uint256) public balanceOf;
    mapping(address => mapping(address => uint256)) public allowance;
    constructor(string memory _n, string memory _s) { name = _n; symbol = _s; }
    function mint(address to, uint256 a) external { balanceOf[to] += a; totalSupply += a; }
    function approve(address s, uint256 a) external returns (bool) { allowance[msg.sender][s] = a; return true; }
    function transfer(address to, uint256 a) external returns (bool) { balanceOf[msg.sender] -= a; balanceOf[to] += a; return true; }
    function transferFrom(address f, address to, uint256 a) external returns (bool) { allowance[f][msg.sender] -= a; balanceOf[f] -= a; balanceOf[to] += a; return true; }
}

contract DeployMM is Script {
    uint256 constant PK0 = 0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80;
    uint256 constant PK1 = 0x59c6995e998f97a5a0044966f0945389dc9e86dae88c7a8412f4603b6b78690d;
    address constant ACCOUNT0 = 0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266; // taker / borrower
    address constant ACCOUNT1 = 0x70997970C51812dc3A010C7d01b50e0d17dc79C8; // maker / lender

    uint256 constant LLTV = 0.77e18;
    uint256 constant CURSOR = 0.3e18;
    uint256 constant MATURITY = 4_000_000_000;
    uint256 constant UNITS = 1_000_000;

    function run() external {
        vm.startBroadcast(PK0);
        Midnight midnight = new Midnight();
        EcrecoverRatifier ratifier = new EcrecoverRatifier(address(midnight));
        MintableERC20 loan = new MintableERC20("loan", "LOAN");
        MintableERC20 collat = new MintableERC20("collat", "COLL");
        Oracle oracle = new Oracle();
        oracle.setPrice(ORACLE_PRICE_SCALE);

        midnight.setFeeSetter(ACCOUNT0);
        midnight.setTickSpacingSetter(ACCOUNT0);
        midnight.enableLiquidationCursor(CURSOR);
        midnight.enableLltv(LLTV);
        uint256 CBP = 1e12;
        uint16[7] memory cbps = [uint16(14), 14, 98, 417, 1250, 2500, 5000];
        for (uint256 i = 0; i <= 6; i++) midnight.setDefaultSettlementFee(address(loan), i, uint256(cbps[i]) * CBP);

        CollateralParams[] memory cp = new CollateralParams[](1);
        cp[0] = CollateralParams({token: address(collat), lltv: LLTV, liquidationCursor: CURSOR, oracle: address(oracle)});
        Market memory market = Market({
            chainId: block.chainid, midnight: address(midnight), loanToken: address(loan),
            collateralParams: cp, maturity: MATURITY, rcfThreshold: 0, enterGate: address(0), liquidatorGate: address(0)
        });

        // heavily over-collateralize the taker for many fills; create the market
        uint256 collAmt = (((UNITS * WAD + LLTV - 1) / LLTV) * ORACLE_PRICE_SCALE + ORACLE_PRICE_SCALE - 1)
            / ORACLE_PRICE_SCALE * 50;
        collat.mint(ACCOUNT0, collAmt);
        collat.approve(address(midnight), collAmt);
        midnight.supplyCollateral(market, 0, collAmt, ACCOUNT0);
        vm.stopBroadcast();

        // maker: fund + approve loan, and directly authorize the ratifier
        vm.startBroadcast(PK1);
        loan.mint(ACCOUNT1, UNITS * 1000);
        loan.approve(address(midnight), type(uint256).max);
        midnight.setIsAuthorized(address(ratifier), true, ACCOUNT1);
        vm.stopBroadcast();

        console2.log("MIDNIGHT", address(midnight));
        console2.log("RATIFIER", address(ratifier));
        console2.log("LOAN", address(loan));
        console2.log("COLLATERAL", address(collat));
        console2.log("ORACLE", address(oracle));
        console2.log("MARKET_ID", vm.toString(IdLib.toId(market)));
    }
}
