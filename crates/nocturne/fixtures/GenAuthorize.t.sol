// SPDX-License-Identifier: GPL-2.0-or-later
// Fixture generator for nocturne Authorization parity.
//
// This is NOT part of `cargo test`. It is the authoritative oracle that produces the
// constants baked into `tests/authorize.rs`. It builds a concrete Authorization, computes
// the hashStruct / digest exactly as `EcrecoverAuthorizer.setIsAuthorized` does, signs it,
// and asserts the real on-chain `setIsAuthorized` accepts the signature — then prints every
// value the Rust side must reproduce.
//
// No stubs: the authorizer is backed by the REAL `Midnight` contract, and its forwarded
// `IMidnight(MIDNIGHT).setIsAuthorized(...)` call really executes against it.
//
// Regenerate (from the midnight contracts repo, pinned at the rev in authorize.rs):
//   cp crates/nocturne/fixtures/GenAuthorize.t.sol <midnight>/test/GenAuthorize.t.sol
//   cd <midnight> && forge test --match-contract GenAuthorize -vv
//   # copy the printed constants into crates/nocturne/tests/authorize.rs, then delete the copy
pragma solidity 0.8.34;

import {Test, console2} from "forge-std/Test.sol";
import {EcrecoverAuthorizer} from "../src/periphery/EcrecoverAuthorizer.sol";
import {Midnight} from "../src/Midnight.sol";
import {
    Authorization,
    Signature,
    AUTHORIZATION_TYPEHASH,
    EIP712_DOMAIN_TYPEHASH
} from "../src/periphery/interfaces/IEcrecoverAuthorizer.sol";

contract GenAuthorizeTest is Test {
    uint256 constant PK = 0xA11CE;

    EcrecoverAuthorizer authorizer_;
    address authorizer;

    function makeAuthorization() internal view returns (Authorization memory a) {
        a.authorizer = authorizer;
        a.authorized = address(0x2222222222222222222222222222222222222222);
        a.isAuthorized = true;
        a.nonce = 0;
        a.deadline = 2_000_000_000;
    }

    /// Digest assembled exactly as EcrecoverAuthorizer.setIsAuthorized.
    function computeDigest(Authorization memory a) internal view returns (bytes32 hashStruct, bytes32 digest) {
        hashStruct = keccak256(abi.encode(AUTHORIZATION_TYPEHASH, a));
        bytes32 domainSeparator =
            keccak256(abi.encode(EIP712_DOMAIN_TYPEHASH, block.chainid, address(authorizer_)));
        digest = keccak256(bytes.concat("\x19\x01", domainSeparator, hashStruct));
    }

    function test_generate() public {
        authorizer = vm.addr(PK);
        // Real EcrecoverAuthorizer backed by the real Midnight (no stub). Deployment order is
        // Midnight first, authorizer second — identical to the previous (stub-first, authorizer-
        // second) layout, so the authorizer address (the EIP-712 verifyingContract) is unchanged.
        Midnight midnight = new Midnight();
        authorizer_ = new EcrecoverAuthorizer(address(midnight));

        Authorization memory a = makeAuthorization();
        (bytes32 hashStruct, bytes32 digest) = computeDigest(a);

        (uint8 v, bytes32 r, bytes32 s) = vm.sign(PK, digest);

        // The authorizer forwards to Midnight.setIsAuthorized on behalf of `a.authorizer`, which
        // requires the authorizer contract to be authorized by that account on Midnight. Grant it
        // (as the account itself) so the real forwarded call succeeds. This is post-deployment and
        // deploys nothing, so it does not affect the authorizer address or the EIP-712 digest.
        vm.prank(a.authorizer);
        midnight.setIsAuthorized(address(authorizer_), true, a.authorizer);

        // Prove the real authorizer accepts this signature (signer == authorization.authorizer).
        authorizer_.setIsAuthorized(a, Signature(v, r, s));

        console2.log("chainId          ", block.chainid);
        console2.log("authorizerContract", address(authorizer_));
        console2.log("authorizer       ", a.authorizer);
        console2.log("authorized       ", a.authorized);
        console2.log("isAuthorized     ", a.isAuthorized);
        console2.log("nonce            ", a.nonce);
        console2.log("deadline         ", a.deadline);
        console2.logString("hashStruct / digest / r / s below:");
        console2.logBytes32(hashStruct);
        console2.logBytes32(digest);
        console2.logBytes32(r);
        console2.logBytes32(s);
        console2.log("v                ", v);
    }
}
