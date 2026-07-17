//! Tests for the `Signer` abstraction and the vendor-agnostic KMS/HSM reconstruction glue.
//!
//! Self-contained: no forge, no network, no cloud SDK. KMS/HSM signing is simulated with an
//! in-process `k256` key that DER-signs the digest, mirroring what `kms:Sign` returns.

use k256::ecdsa::SigningKey;
use nocturne::*;

fn digest(seed: &[u8]) -> Word {
    keccak(seed)
}

fn key(byte: u8) -> SigningKey {
    SigningKey::from_bytes(&[byte; 32].into()).unwrap()
}

#[test]
fn local_signer_signs_and_recovers() {
    let signer = LocalSigner::from_bytes(&[0x42; 32]).unwrap();
    let d = digest(b"local signer: recover to address");
    let sig = signer.sign_digest(&d).unwrap();

    // Recovers to the signer's address, and the address matches `signer_address` of the key.
    assert_eq!(recover(&d, &sig).as_ref(), Some(&signer.address()));
    let sk = key(0x42);
    assert_eq!(signer.address(), signer_address(&sk));

    // `v` is in the on-chain 27/28 range.
    assert!(sig.v == 27 || sig.v == 28);
}

#[test]
fn local_signer_from_bytes_matches_crate_signer() {
    let signer = LocalSigner::from_bytes(&[0x07; 32]).unwrap();
    let sk = key(0x07);
    let d = digest(b"parity with crate::sign_digest..");
    assert_eq!(signer.sign_digest(&d).unwrap(), sign_digest(&sk, &d));
}

#[test]
fn sig_from_rs_recovers_v_across_keys_and_digests() {
    for kb in [0x01u8, 0x11, 0x42, 0x77, 0x9c, 0xfe] {
        let sk = key(kb);
        let expected = signer_address(&sk);
        for ds in [
            b"digest one".as_slice(),
            b"digest two",
            b"another digest payload",
        ] {
            let d = digest(ds);
            let full = sign_digest(&sk, &d);
            // Discard v; reconstruct purely from (r, s) + the known address.
            let rebuilt = sig_from_rs(&d, &full.r, &full.s, &expected).unwrap();
            assert_eq!(recover(&d, &rebuilt).as_ref(), Some(&expected));
            // r/s (already low-s) unchanged; v correctly recovered.
            assert_eq!(rebuilt.r, full.r);
            assert_eq!(rebuilt.s, full.s);
            assert!(rebuilt.v == 27 || rebuilt.v == 28);
        }
    }
}

#[test]
fn sig_from_rs_normalizes_high_s() {
    let sk = key(0x33);
    let expected = signer_address(&sk);
    let d = digest(b"high-s normalization test......");
    let low = sign_digest(&sk, &d);

    // Force the malleable high-s counterpart n - s and feed it in.
    let high_s = high_s_counterpart(&low.s);
    assert_ne!(high_s, low.s, "counterpart must differ from low-s");

    let rebuilt = sig_from_rs(&d, &low.r, &high_s, &expected).unwrap();

    // Returned s must be the low-s value (<= n/2), i.e. back to the original.
    assert_eq!(rebuilt.s, low.s, "s should be normalized back to low-s");
    // And it still recovers the correct signer (v was re-derived after the flip).
    assert_eq!(recover(&d, &rebuilt).as_ref(), Some(&expected));
}

#[test]
fn sig_from_der_matches_rs_path() {
    let sk = key(0x5a);
    let expected = signer_address(&sk);
    let d = digest(b"der path equals rs path.........");

    // Simulate a KMS/HSM: sign the prehash and DER-encode (no recovery id, like a real HSM).
    let (der_sig, _rec) = sk.sign_prehash_recoverable(&d).unwrap();
    let der = der_sig.to_der();

    let from_der = sig_from_der(&d, der.as_bytes(), &expected).unwrap();
    assert_eq!(recover(&d, &from_der).as_ref(), Some(&expected));

    // Same result as going through the (r, s) helper directly.
    let full = sign_digest(&sk, &d);
    let from_rs = sig_from_rs(&d, &full.r, &full.s, &expected).unwrap();
    assert_eq!(from_der, from_rs);
}

#[test]
fn external_signer_matches_local_signer() {
    // The closure captures a k256 key and DER-signs - standing in for a KMS/HSM backend.
    let kb = 0x8d;
    let sk = key(kb);
    let address = signer_address(&sk);

    let external = ExternalSigner::new(address, move |d: &Word| {
        let (sig, _rec) = sk
            .sign_prehash_recoverable(d)
            .map_err(|_| SignerError::SigningFailed)?;
        Ok(sig.to_der().as_bytes().to_vec())
    });

    let local = LocalSigner::from_bytes(&[kb; 32]).unwrap();
    let d = digest(b"external signer vs local signer.");

    let ext_sig = external.sign_digest(&d).unwrap();
    let loc_sig = local.sign_digest(&d).unwrap();

    assert_eq!(external.address(), local.address());
    assert_eq!(
        ext_sig, loc_sig,
        "external and local must produce the same r/s/v"
    );
    assert_eq!(recover(&d, &ext_sig).as_ref(), Some(&address));
}

#[test]
fn sig_from_rs_wrong_signer_errors() {
    let sk = key(0x21);
    let d = digest(b"wrong expected signer address..");
    let full = sign_digest(&sk, &d);

    let wrong = [0x99u8; 20];
    assert_eq!(
        sig_from_rs(&d, &full.r, &full.s, &wrong),
        Err(SignerError::RecoveryMismatch)
    );
}

#[test]
fn sig_from_der_rejects_garbage() {
    let d = digest(b"garbage der bytes...............");
    let addr = [0x00u8; 20];
    assert_eq!(
        sig_from_der(&d, &[0x00, 0x01, 0x02, 0x03], &addr),
        Err(SignerError::BadDer)
    );
}

#[test]
fn local_signer_from_bytes_rejects_zero_key() {
    // All-zero is not a valid secp256k1 private key.
    assert!(matches!(
        LocalSigner::from_bytes(&[0u8; 32]),
        Err(SignerError::BadScalar)
    ));
}
