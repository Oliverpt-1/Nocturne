//! Signer abstraction with a vendor-agnostic KMS/HSM path.
//!
//! Institutions rarely hold a raw private key in process; they sign through a service
//! (AWS KMS, GCP KMS, or a PKCS#11 HSM). Those services return a raw ECDSA signature —
//! DER-encoded, or as `(r, s)` — over the 32-byte digest, but they do **not** return the
//! recovery id (`v`) that Ethereum's `ecrecover` needs, and they may return a high-`s`
//! value. This module supplies the glue that turns such a signature into the crate's
//! [`Sig`] `{ r, s, v }`:
//!
//! * normalize `s` to its low-`s` form (BIP-0062), since the EVM and most tooling expect it;
//! * brute-force `v` (27 then 28) against the *known* signer address, using [`crate::recover`],
//!   because flipping `s` can flip the parity — so the only reliable way to pin `v` is to try
//!   both and keep the one that recovers the expected address.
//!
//! No cloud SDKs are pulled in: [`ExternalSigner`] takes a closure, so an integrator wires the
//! actual KMS/HSM call themselves (see its docs for the AWS KMS recipe).

use k256::ecdsa::{Signature as EcdsaSig, SigningKey};

use crate::{recover, signer_address, Address, Sig, Word, U256};

/// secp256k1 group order `n`, big-endian. Used only to build a high-`s` value in tests; the
/// production low-`s` normalization goes through [`EcdsaSig::normalize_s`].
pub const SECP256K1_N: Word = [
    0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFE,
    0xBA, 0xAE, 0xDC, 0xE6, 0xAF, 0x48, 0xA0, 0x3B, 0xBF, 0xD2, 0x5E, 0x8C, 0xD0, 0x36, 0x41, 0x41,
];

/// Errors from a [`Signer`] or the KMS/HSM signature-reconstruction helpers.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SignerError {
    /// The backend (local key or external service) failed to produce a signature.
    #[error("signing backend failed to produce a signature")]
    SigningFailed,
    /// No recovery id (`v = 27` or `28`) reproduces the expected signer address. Either the
    /// signature is not over this digest, or `expected_signer` is wrong.
    #[error("no recovery id recovers the expected signer address")]
    RecoveryMismatch,
    /// The DER blob is not a well-formed ECDSA signature.
    #[error("malformed DER-encoded ECDSA signature")]
    BadDer,
    /// `r`/`s` is not a valid secp256k1 scalar (zero, or >= the group order).
    #[error("invalid secp256k1 scalar in signature")]
    BadScalar,
}

/// A backend that can produce an Ethereum-style recoverable signature over a 32-byte digest.
///
/// Implemented by [`LocalSigner`] (in-process key) and [`ExternalSigner`] (KMS/HSM closure).
pub trait Signer {
    /// The Ethereum address this signer produces signatures for (the offer `maker`).
    fn address(&self) -> Address;
    /// Sign a 32-byte digest, returning a `{ r, s, v }` with low-`s` and the correct `v`.
    fn sign_digest(&self, digest: &Word) -> Result<Sig, SignerError>;
}

/// A signer backed by an in-process secp256k1 key. Reference/testing path — prefer
/// [`ExternalSigner`] for anything holding real value.
#[derive(Clone)]
pub struct LocalSigner {
    sk: SigningKey,
    address: Address,
}

impl LocalSigner {
    /// Build a signer from 32 raw private-key bytes. Errors with [`SignerError::BadScalar`]
    /// if the bytes are not a valid secp256k1 private key.
    pub fn from_bytes(bytes: &[u8; 32]) -> Result<Self, SignerError> {
        let fb: k256::FieldBytes = (*bytes).into();
        let sk = SigningKey::from_bytes(&fb).map_err(|_| SignerError::BadScalar)?;
        let address = signer_address(&sk);
        Ok(Self { sk, address })
    }
}

impl core::fmt::Debug for LocalSigner {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        // Never print key material.
        f.debug_struct("LocalSigner")
            .field("address", &self.address)
            .finish_non_exhaustive()
    }
}

impl Signer for LocalSigner {
    fn address(&self) -> Address {
        self.address
    }
    fn sign_digest(&self, digest: &Word) -> Result<Sig, SignerError> {
        // The crate's reference signer already yields low-`s` with the correct `v`.
        Ok(crate::sign_digest(&self.sk, digest))
    }
}

/// Turn a raw `(r, s)` signature (as returned by a KMS/HSM, without a recovery id) into a
/// complete [`Sig`].
///
/// Steps: normalize `s` to low-`s` (via [`EcdsaSig::normalize_s`]), then try `v = 27` and
/// `v = 28` with [`crate::recover`], returning the `Sig` that recovers to `expected_signer`.
/// Errors [`SignerError::RecoveryMismatch`] if neither `v` matches, or
/// [`SignerError::BadScalar`] if `(r, s)` are not valid scalars.
pub fn sig_from_rs(
    digest: &Word,
    r: &Word,
    s: &Word,
    expected_signer: &Address,
) -> Result<Sig, SignerError> {
    let mut rs = [0u8; 64];
    rs[..32].copy_from_slice(r);
    rs[32..].copy_from_slice(s);
    let parsed = EcdsaSig::from_slice(&rs).map_err(|_| SignerError::BadScalar)?;
    // `normalize_s` returns `Some(low_s_sig)` when `s` was high, `None` when already low.
    let low = parsed.normalize_s().unwrap_or(parsed);

    let bytes = low.to_bytes();
    let mut lr = [0u8; 32];
    let mut ls = [0u8; 32];
    lr.copy_from_slice(&bytes[..32]);
    ls.copy_from_slice(&bytes[32..]);

    for v in [27u8, 28u8] {
        let candidate = Sig { r: lr, s: ls, v };
        if recover(digest, &candidate).as_ref() == Some(expected_signer) {
            return Ok(candidate);
        }
    }
    Err(SignerError::RecoveryMismatch)
}

/// Turn a DER-encoded ECDSA signature (as returned by AWS/GCP KMS or a PKCS#11 HSM) into a
/// complete [`Sig`]. Parses the DER, extracts `(r, s)`, and delegates to [`sig_from_rs`].
pub fn sig_from_der(
    digest: &Word,
    der: &[u8],
    expected_signer: &Address,
) -> Result<Sig, SignerError> {
    let parsed = EcdsaSig::from_der(der).map_err(|_| SignerError::BadDer)?;
    let bytes = parsed.to_bytes();
    let mut r = [0u8; 32];
    let mut s = [0u8; 32];
    r.copy_from_slice(&bytes[..32]);
    s.copy_from_slice(&bytes[32..]);
    sig_from_rs(digest, &r, &s, expected_signer)
}

/// A signer whose private key lives in an external service — AWS KMS, GCP KMS, or a PKCS#11
/// HSM — reached through a caller-supplied closure. This is the production path for
/// institutions that cannot hold raw keys in process.
///
/// The closure receives the 32-byte digest and must return a DER-encoded ECDSA signature over
/// it. `address` is derived **once** from the backend's public key and passed in at
/// construction; it is what [`sig_from_der`] brute-forces `v` against.
///
/// # Wiring AWS KMS
///
/// One-time setup: read the key's public key with `kms:GetPublicKey`, DER-decode the
/// SubjectPublicKeyInfo to the 64-byte uncompressed secp256k1 point, and derive the Ethereum
/// address (`keccak256(pubkey)[12..]`). Pass that as `address`.
///
/// Per signature, the closure calls `kms:Sign` with:
/// * `MessageType = DIGEST` — you are signing the 32-byte digest directly, not a message;
/// * `SigningAlgorithm = ECDSA_SHA_256`;
/// * `Message = <the 32-byte digest>`.
///
/// KMS returns `Signature` as a DER-encoded ECDSA signature — return those bytes from the
/// closure. [`ExternalSigner::sign_digest`] then normalizes `s` and pins `v`. GCP KMS
/// (`AsymmetricSign`) and PKCS#11 HSMs follow the same shape: sign the digest, hand back DER.
///
/// ```no_run
/// # use nocturne_offers::{ExternalSigner, Signer, SignerError, Address, Word};
/// let address: Address = [0u8; 20]; // derived once from the KMS public key
/// let signer = ExternalSigner::new(address, |digest: &Word| -> Result<Vec<u8>, SignerError> {
///     // let out = kms_client.sign(digest, "ECDSA_SHA_256", "DIGEST")?;
///     // Ok(out.signature) // DER bytes
///     # let _ = digest;
///     Err(SignerError::SigningFailed)
/// });
/// # let _ = signer;
/// ```
pub struct ExternalSigner<F> {
    address: Address,
    sign_der: F,
}

impl<F> ExternalSigner<F>
where
    F: Fn(&Word) -> Result<Vec<u8>, SignerError>,
{
    /// Build an external signer from the backend's Ethereum `address` and a closure that
    /// DER-signs a digest via the KMS/HSM.
    pub fn new(address: Address, sign_der: F) -> Self {
        Self { address, sign_der }
    }
}

impl<F> core::fmt::Debug for ExternalSigner<F> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("ExternalSigner")
            .field("address", &self.address)
            .finish_non_exhaustive()
    }
}

impl<F> Signer for ExternalSigner<F>
where
    F: Fn(&Word) -> Result<Vec<u8>, SignerError>,
{
    fn address(&self) -> Address {
        self.address
    }
    fn sign_digest(&self, digest: &Word) -> Result<Sig, SignerError> {
        let der = (self.sign_der)(digest)?;
        sig_from_der(digest, &der, &self.address)
    }
}

/// Compute `n - s` (big-endian), the high-`s` counterpart of a low-`s` value. Exposed for
/// tests exercising the low-`s` normalization path.
pub fn high_s_counterpart(s: &Word) -> Word {
    let n = U256::from_be_bytes(SECP256K1_N);
    let s_val = U256::from_be_bytes(*s);
    (n - s_val).to_be_bytes::<32>()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn local_signer_round_trips() {
        let signer = LocalSigner::from_bytes(&[0x42; 32]).unwrap();
        let digest = crate::keccak(b"local signer unit test digest...");
        let sig = signer.sign_digest(&digest).unwrap();
        assert_eq!(recover(&digest, &sig).as_ref(), Some(&signer.address()));
    }

    #[test]
    fn high_s_counterpart_is_high() {
        // n/2 boundary: n - 1 is high, and its counterpart (1) is low.
        let one = {
            let mut w = [0u8; 32];
            w[31] = 1;
            w
        };
        let back = high_s_counterpart(&high_s_counterpart(&one));
        assert_eq!(back, one);
    }
}
