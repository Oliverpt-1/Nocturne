//! EIP-712 `Authorization` signing for hot-key delegation.
//!
//! Byte-for-byte mirror of the digest built in `EcrecoverAuthorizer.setIsAuthorized`
//! (`src/periphery/EcrecoverAuthorizer.sol`). A maker (the `authorizer`) signs an
//! [`Authorization`] granting/revoking a hot signing key or ratifier (the `authorized`);
//! anyone can then submit `setIsAuthorized(authorization, signature)` and the contract
//! recovers the signer and checks it equals `authorization.authorizer` (or is authorized
//! by it on Midnight).
//!
//! The EIP-712 domain's `verifyingContract` is the `EcrecoverAuthorizer` address itself
//! (`address(this)`), which is why it is passed explicitly here.

use crate::{keccak, Address, Sig, Word, U256};
use k256::ecdsa::SigningKey;

/// The `Authorization` struct signed for hot-key delegation, mirroring
/// `IEcrecoverAuthorizer.sol`.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Authorization {
    /// The address granting authorization; must be the recovered signer on-chain.
    pub authorizer: Address,
    /// The address (hot key / ratifier) being authorized or de-authorized.
    pub authorized: Address,
    /// Whether `authorized` is being granted (`true`) or revoked (`false`).
    pub is_authorized: bool,
    /// Per-authorizer replay nonce; must equal `nonce[authorizer]` on-chain.
    pub nonce: U256,
    /// Unix timestamp after which the signature is rejected as `Expired`.
    pub deadline: U256,
}

impl Authorization {
    /// Convenience constructor.
    pub fn new(
        authorizer: Address,
        authorized: Address,
        is_authorized: bool,
        nonce: U256,
        deadline: U256,
    ) -> Self {
        Authorization {
            authorizer,
            authorized,
            is_authorized,
            nonce,
            deadline,
        }
    }
}

/// The EIP-712 type string for [`Authorization`].
pub const AUTHORIZATION_TYPE: &str =
    "Authorization(address authorizer,address authorized,bool isAuthorized,uint256 nonce,uint256 deadline)";

/// `keccak256(AUTHORIZATION_TYPE)` - matches the on-chain `AUTHORIZATION_TYPEHASH` constant.
/// Tests assert [`authorization_typehash`] reproduces this value.
pub const AUTHORIZATION_TYPEHASH: Word = [
    0x81, 0xd0, 0x28, 0x4f, 0xb0, 0xe2, 0xcd, 0xe1, 0x8d, 0x05, 0x53, 0xb0, 0x61, 0x89, 0xd6, 0xf7,
    0x61, 0x3c, 0x96, 0xa0, 0x1b, 0xb5, 0xb5, 0xe7, 0x82, 0x8e, 0xad, 0xe6, 0xa0, 0xdc, 0xac, 0x91,
];

/// The EIP-712 domain type string, matching the on-chain `EIP712_DOMAIN_TYPEHASH`.
pub const AUTHORIZE_DOMAIN_TYPE: &str = "EIP712Domain(uint256 chainId,address verifyingContract)";

/// Compute the `Authorization` typehash from the type string; tests assert it equals
/// the baked [`AUTHORIZATION_TYPEHASH`] constant (same pattern as the offer typehashes).
pub fn authorization_typehash() -> Word {
    keccak(AUTHORIZATION_TYPE.as_bytes())
}

#[inline]
fn addr_word(a: &Address) -> Word {
    let mut w = [0u8; 32];
    w[12..].copy_from_slice(a);
    w
}

#[inline]
fn bool_word(b: bool) -> Word {
    let mut w = [0u8; 32];
    w[31] = b as u8;
    w
}

fn encode(words: &[Word]) -> Vec<u8> {
    let mut out = Vec::with_capacity(words.len() * 32);
    for w in words {
        out.extend_from_slice(w);
    }
    out
}

/// EIP-712 struct hash of an [`Authorization`]:
/// `keccak256(abi.encode(AUTHORIZATION_TYPEHASH, authorizer, authorized, isAuthorized, nonce, deadline))`.
/// Mirrors `keccak256(abi.encode(AUTHORIZATION_TYPEHASH, authorization))` in the contract.
pub fn hash_authorization(a: &Authorization) -> Word {
    keccak(&encode(&[
        AUTHORIZATION_TYPEHASH,
        addr_word(&a.authorizer),
        addr_word(&a.authorized),
        bool_word(a.is_authorized),
        a.nonce.to_be_bytes::<32>(),
        a.deadline.to_be_bytes::<32>(),
    ]))
}

/// The EIP-712 domain separator:
/// `keccak256(abi.encode(EIP712_DOMAIN_TYPEHASH, chainId, verifyingContract))`, where
/// `verifyingContract` is the `EcrecoverAuthorizer` (`address(this)`).
pub fn authorize_domain_separator(chain_id: Word, authorizer_contract: &Address) -> Word {
    keccak(&encode(&[
        keccak(AUTHORIZE_DOMAIN_TYPE.as_bytes()),
        chain_id,
        addr_word(authorizer_contract),
    ]))
}

/// The full `0x1901` EIP-712 digest the authorizer signs, assembled exactly as
/// `EcrecoverAuthorizer.setIsAuthorized`. `authorizer_contract` is the `EcrecoverAuthorizer`
/// address used as the domain `verifyingContract`.
pub fn authorization_digest(
    a: &Authorization,
    chain_id: Word,
    authorizer_contract: &Address,
) -> Word {
    let hash_struct = hash_authorization(a);
    let domain_separator = authorize_domain_separator(chain_id, authorizer_contract);
    let mut buf = Vec::with_capacity(2 + 64);
    buf.extend_from_slice(&[0x19, 0x01]);
    buf.extend_from_slice(&domain_separator);
    buf.extend_from_slice(&hash_struct);
    keccak(&buf)
}

/// Sign an [`Authorization`] with the authorizer's key (secp256k1, in-process).
pub fn sign_authorization(
    sk: &SigningKey,
    a: &Authorization,
    chain_id: Word,
    authorizer_contract: &Address,
) -> Sig {
    crate::sign_digest(sk, &authorization_digest(a, chain_id, authorizer_contract))
}

/// Recover the signer of an [`Authorization`] signature, mirroring the `ecrecover` step in
/// `EcrecoverAuthorizer.setIsAuthorized`. Returns `None` for a malformed signature (the
/// on-chain equivalent of `ecrecover` yielding `address(0)`).
pub fn recover_authorization(
    a: &Authorization,
    chain_id: Word,
    authorizer_contract: &Address,
    sig: &Sig,
) -> Option<Address> {
    crate::recover(&authorization_digest(a, chain_id, authorizer_contract), sig)
}
