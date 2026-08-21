use nocturne::*;

#[test]
fn base_submission_is_a_raw_zero_value_transaction() {
    let payload = [1, 2, 3, 4];
    let tx = mempool_submission(BASE_CHAIN_ID, payload, []).unwrap();
    assert_eq!(tx.to, BASE_MIDNIGHT_MEMPOOL);
    assert_eq!(tx.value, U256::ZERO);
    assert_eq!(tx.data, payload);
}

#[test]
fn metadata_is_appended_without_changing_payload() {
    let tx = mempool_submission(BASE_CHAIN_ID, [1, 2], [0xa1, 0xb2, 0xc3, 0xd4]).unwrap();
    assert_eq!(tx.data, [1, 2, 0xa1, 0xb2, 0xc3, 0xd4]);
}

#[test]
fn custom_mempool_supports_local_and_future_deployments() {
    let custom = [0x42; 20];
    let tx = mempool_submission_to(custom, [0xaa], []).unwrap();
    assert_eq!(tx.to, custom);
}

#[test]
fn unsupported_chain_and_large_suffix_are_rejected() {
    assert_eq!(
        mempool_submission(1, [], []).unwrap_err(),
        SubmissionError::UnsupportedChain(1)
    );
    assert_eq!(
        mempool_submission_to([0; 20], [], [0; 257]).unwrap_err(),
        SubmissionError::AttributionTooLarge(257)
    );
}

#[cfg(feature = "alloy-wallet")]
#[test]
fn alloy_request_preserves_destination_value_and_input() {
    let transaction = mempool_submission_to([0x42; 20], [1, 2, 3], []).unwrap();
    let request: alloy::rpc::types::TransactionRequest = transaction.into();
    assert_eq!(
        request.to,
        Some(alloy::primitives::TxKind::Call(
            alloy::primitives::Address::from_slice(&[0x42; 20])
        ))
    );
    assert_eq!(request.value, Some(alloy::primitives::U256::ZERO));
    assert_eq!(request.input.input.unwrap().as_ref(), [1, 2, 3]);
}
