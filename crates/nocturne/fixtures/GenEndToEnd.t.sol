// SPDX-License-Identifier: GPL-2.0-or-later
// Fixture generator for nocturne end-to-end parity (Tier 0).
//
// This is NOT part of `cargo test`. It is the authoritative oracle that produces the
// constants baked into `tests/parity_e2e.rs`. It builds a concrete 4-offer tree, computes
// the leaf / root / signed digest exactly as `EcrecoverRatifier.isRatified` does, signs it,
// and asserts the real on-chain `isRatified` accepts the signature — then prints every value
// the Rust side must reproduce.
//
// Regenerate (from the midnight contracts repo, pinned at the rev in parity_e2e.rs):
//   cp crates/nocturne/fixtures/GenEndToEnd.t.sol <midnight>/test/GenEndToEnd.t.sol
//   cd <midnight> && forge test --match-contract GenEndToEnd -vv
//   # copy the printed constants into crates/nocturne/tests/parity_e2e.rs, then delete the copy
pragma solidity 0.8.34;

import {Test, console2} from "forge-std/Test.sol";
import {EcrecoverRatifier} from "../src/ratifiers/EcrecoverRatifier.sol";
import {Signature} from "../src/ratifiers/interfaces/IEcrecoverRatifier.sol";
import {HashLib} from "../src/ratifiers/libraries/HashLib.sol";
import {Offer, Market, CollateralParams} from "../src/interfaces/IMidnight.sol";
import {CALLBACK_SUCCESS} from "../src/libraries/ConstantsLib.sol";

contract GenEndToEndTest is Test {
    uint256 constant PK = 0xA11CE;
    uint256 constant N = 4; // 4 leaves -> height 2

    EcrecoverRatifier ratifier;
    address maker;

    function makeMarket() internal view returns (Market memory m) {
        CollateralParams[] memory cps = new CollateralParams[](1);
        cps[0].token = address(0x3333333333333333333333333333333333333333);
        cps[0].lltv = 0.86e18;
        cps[0].liquidationCursor = 1;
        cps[0].oracle = address(0x4444444444444444444444444444444444444444);
        m.chainId = block.chainid;
        m.midnight = address(0x1111111111111111111111111111111111111111);
        m.loanToken = address(0x2222222222222222222222222222222222222222);
        m.collateralParams = cps;
        m.maturity = 1_800_000_000;
        m.rcfThreshold = 1000;
        m.enterGate = address(0);
        m.liquidatorGate = address(0);
    }

    function makeOffer(uint256 i) internal view returns (Offer memory o) {
        o.market = makeMarket();
        o.buy = i % 2 == 0;
        o.maker = maker;
        o.start = 0;
        o.expiry = 2_000_000_000;
        o.tick = i;
        o.group = bytes32(i);
        o.callback = address(0);
        o.callbackData = "";
        o.receiverIfMakerIsSeller = address(0);
        o.ratifier = address(ratifier);
        o.reduceOnly = false;
        o.maxUnits = uint128(1_000_000 + i);
        o.maxAssets = 0;
        o.continuousFeeCap = 0;
    }

    /// Perfect binary tree over the 4 offers. Returns (leaf0, root) and fills `proof` for leaf 0.
    function buildTree(bytes32[] memory proof) internal view returns (bytes32 leaf0, bytes32 root) {
        bytes32[] memory leaves = new bytes32[](N);
        for (uint256 i = 0; i < N; i++) {
            leaves[i] = HashLib.hashOffer(makeOffer(i));
        }
        bytes32 n23 = HashLib.hashNode(leaves[2], leaves[3]);
        root = HashLib.hashNode(HashLib.hashNode(leaves[0], leaves[1]), n23);
        // Proof for leaf 0: sibling per level, low bit first (matches HashLib.isLeaf).
        proof[0] = leaves[1];
        proof[1] = n23;
        leaf0 = leaves[0];
    }

    /// Digest assembled exactly as EcrecoverRatifier.isRatified.
    function computeDigest(bytes32 root, uint256 proofLen) internal view returns (bytes32) {
        bytes32 structHash = keccak256(abi.encode(HashLib.offerTreeTypeHash(proofLen), root));
        bytes32 domainSeparator = keccak256(
            abi.encode(
                0x47e79534a245952e8b16893a336b85a3d9ea9fa8c573f3d803afb92a79469218, // EIP712Domain typehash
                block.chainid,
                address(ratifier)
            )
        );
        return keccak256(bytes.concat("\x19\x01", domainSeparator, structHash));
    }

    /// Isolated so the (deep) ABI-encode of the Offer for the external call gets its own frame.
    function assertRatified(bytes32 root, bytes32[] memory proof, uint8 v, bytes32 r, bytes32 s) internal view {
        bytes memory ratifierData = abi.encode(Signature(v, r, s), root, uint256(0), proof);
        require(
            ratifier.isRatified(makeOffer(0), ratifierData, address(0xca11e7)) == CALLBACK_SUCCESS,
            "ratifier must accept the generated signature"
        );
    }

    function test_generate() public {
        maker = vm.addr(PK);
        ratifier = new EcrecoverRatifier(address(0xdead));

        bytes32[] memory proof = new bytes32[](2);
        (bytes32 leaf0, bytes32 root) = buildTree(proof);
        bytes32 digest = computeDigest(root, proof.length);

        (uint8 v, bytes32 r, bytes32 s) = vm.sign(PK, digest);

        // Prove the real ratifier accepts this signature for offer 0.
        assertRatified(root, proof, v, r, s);

        console2.log("chainId          ", block.chainid);
        console2.log("maker            ", maker);
        console2.log("ratifier         ", address(ratifier));
        console2.logString("leaf0 / root / digest / r / s below:");
        console2.logBytes32(leaf0);
        console2.logBytes32(root);
        console2.logBytes32(digest);
        console2.logBytes32(r);
        console2.logBytes32(s);
        console2.log("v                ", v);
    }
}
