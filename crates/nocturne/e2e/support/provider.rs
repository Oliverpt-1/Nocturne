use alloy::{
    primitives::{Address as AlloyAddress, B256},
    providers::Provider,
    rpc::types::TransactionRequest,
};
use nocturne::{
    discover_requirements, ActionRequirement, Address, MidnightTransaction, RequirementPlan,
};

use super::{BoxError, JournalEntry, LifecycleJournal};

pub fn alloy_address(address: Address) -> AlloyAddress {
    AlloyAddress::from_slice(&address)
}

pub fn sdk_address(address: AlloyAddress) -> Address {
    let mut output = [0u8; 20];
    output.copy_from_slice(address.as_slice());
    output
}

pub async fn send<P: Provider>(
    provider: &P,
    owner: AlloyAddress,
    actor: &str,
    label: &str,
    transaction: MidnightTransaction,
    journal: &mut LifecycleJournal,
) -> Result<B256, BoxError> {
    if let Some(hash) = journal.transaction_hash(actor, label) {
        println!("SKIP {label} already confirmed as {hash}");
        return Ok(hash.parse()?);
    }
    let request: TransactionRequest = transaction.into();
    let request = request.from(owner);
    provider.call(request.clone()).await?;
    let receipt = provider
        .send_transaction(request)
        .await?
        .get_receipt()
        .await?;
    if !receipt.status() {
        return Err(format!("{label} reverted: {}", receipt.transaction_hash).into());
    }
    journal.record(JournalEntry {
        actor: actor.into(),
        label: label.into(),
        transaction_hash: receipt.transaction_hash.to_string(),
        block_number: receipt.block_number.unwrap_or_default(),
    })?;
    println!("TX {label} {}", receipt.transaction_hash);
    Ok(receipt.transaction_hash)
}

pub async fn satisfy<P: Provider>(
    provider: &P,
    owner: AlloyAddress,
    actor: &str,
    plan: &RequirementPlan,
    prefix: &str,
    journal: &mut LifecycleJournal,
) -> Result<(), BoxError> {
    for (index, requirement) in discover_requirements(provider, plan)
        .await?
        .into_iter()
        .enumerate()
    {
        let transaction = match requirement {
            ActionRequirement::Approval { transaction, .. }
            | ActionRequirement::Authorization { transaction, .. } => transaction,
        };
        send(
            provider,
            owner,
            actor,
            &format!("{prefix}-requirement-{}", index + 1),
            transaction,
            journal,
        )
        .await?;
    }
    Ok(())
}
