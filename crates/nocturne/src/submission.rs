//! Wallet-agnostic transaction construction for maker-offer publication.

use crate::{Address, MAX_ATTRIBUTION_SUFFIX_BYTES, U256};

/// Base mainnet chain id.
pub const BASE_CHAIN_ID: u64 = 8_453;
/// Registered Midnight maker mempool on Base mainnet.
pub const BASE_MIDNIGHT_MEMPOOL: Address = [
    0xdd, 0x6d, 0xce, 0x32, 0xe2, 0x1f, 0x7b, 0x02, 0x08, 0x98, 0xa8, 0x25, 0x8d, 0xa3, 0x73, 0x55,
    0xb4, 0x01, 0x79, 0x93,
];
/// Core Midnight contract on Base mainnet.
pub const BASE_MIDNIGHT: Address = [
    0xad, 0xed, 0xd8, 0xab, 0x6d, 0xe8, 0x32, 0x76, 0x6f, 0xed, 0xf0, 0xfa, 0xc4, 0x99, 0x2e, 0x5c,
    0x4d, 0x3e, 0xa1, 0x8a,
];
/// ECDSA-tree ratifier on Base mainnet.
pub const BASE_ECRECOVER_RATIFIER: Address = [
    0xd6, 0xe7, 0x03, 0x65, 0xc8, 0xe8, 0xdd, 0xa9, 0xa4, 0xca, 0x66, 0x2c, 0x07, 0xbb, 0xe6, 0x63,
    0xb0, 0x17, 0x75, 0x5e,
];

/// A transaction request that can be mapped directly into an Ethereum wallet/provider client.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MempoolTransaction {
    /// Registered mempool address receiving the raw payload.
    pub to: Address,
    /// Always zero: publishing offers transfers no native token.
    pub value: U256,
    /// Raw versioned payload, optionally followed by an attribution suffix.
    pub data: Vec<u8>,
}

#[cfg(feature = "alloy-wallet")]
impl From<MempoolTransaction> for alloy::rpc::types::TransactionRequest {
    fn from(transaction: MempoolTransaction) -> Self {
        Self::default()
            .to(alloy::primitives::Address::from_slice(&transaction.to))
            .value(alloy::primitives::U256::from_be_bytes(
                transaction.value.to_be_bytes::<32>(),
            ))
            .input(alloy::primitives::Bytes::from(transaction.data).into())
    }
}

/// Errors constructing a maker-offer publication transaction.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum SubmissionError {
    #[error("Midnight mempool is not registered for chain {0}")]
    UnsupportedChain(u64),
    #[error("attribution suffix exceeds {MAX_ATTRIBUTION_SUFFIX_BYTES} bytes: got {0}")]
    AttributionTooLarge(usize),
}

/// Resolve the registered maker mempool for a supported chain.
pub fn midnight_mempool(chain_id: u64) -> Result<Address, SubmissionError> {
    match chain_id {
        BASE_CHAIN_ID => Ok(BASE_MIDNIGHT_MEMPOOL),
        _ => Err(SubmissionError::UnsupportedChain(chain_id)),
    }
}

/// Build the zero-value transaction that publishes an encoded payload.
///
/// `attribution` is appended verbatim after the length-delimited gzip stream. It is optional and
/// never changes offer semantics.
pub fn mempool_submission(
    chain_id: u64,
    payload: impl AsRef<[u8]>,
    attribution: impl AsRef<[u8]>,
) -> Result<MempoolTransaction, SubmissionError> {
    mempool_submission_to(midnight_mempool(chain_id)?, payload, attribution)
}

/// Build a publication transaction for an explicitly supplied mempool address.
///
/// This variant is useful for local chains and future deployments not yet in the built-in
/// registry.
pub fn mempool_submission_to(
    mempool: Address,
    payload: impl AsRef<[u8]>,
    attribution: impl AsRef<[u8]>,
) -> Result<MempoolTransaction, SubmissionError> {
    let attribution = attribution.as_ref();
    if attribution.len() > MAX_ATTRIBUTION_SUFFIX_BYTES {
        return Err(SubmissionError::AttributionTooLarge(attribution.len()));
    }
    let payload = payload.as_ref();
    let mut data = Vec::with_capacity(payload.len() + attribution.len());
    data.extend_from_slice(payload);
    data.extend_from_slice(attribution);
    Ok(MempoolTransaction {
        to: mempool,
        value: U256::ZERO,
        data,
    })
}
