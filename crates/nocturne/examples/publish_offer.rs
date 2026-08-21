//! Build, ratify, encode, and publish one maker offer through an Alloy wallet.
//!
//! Required environment: `RPC_URL`, `PRIVATE_KEY`. On chains outside the built-in registry, also
//! set `MEMPOOL_ADDRESS`.
//!
//! ```text
//! cargo run -p nocturne-midnight --features alloy-wallet --example publish_offer
//! ```

use std::time::{SystemTime, UNIX_EPOCH};

use alloy::{
    consensus::Transaction as _,
    network::EthereumWallet,
    providers::{Provider, ProviderBuilder},
    signers::local::PrivateKeySigner,
};
use nocturne::*;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let rpc_url = std::env::var("RPC_URL")?;
    let private_key = std::env::var("PRIVATE_KEY")?;
    let key_bytes = decode_fixed::<32>(&private_key)?;

    let maker_signer = LocalSigner::from_bytes(&key_bytes)?;
    let maker = maker_signer.address();
    let wallet_signer: PrivateKeySigner = private_key.parse()?;
    let wallet = EthereumWallet::from(wallet_signer);
    let provider = ProviderBuilder::new()
        .wallet(wallet)
        .connect_http(rpc_url.parse()?);
    let chain_id = provider.get_chain_id().await?;

    let now = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs();
    let maturity = ((now / 86_400) + 30) * 86_400 + 15 * 3_600;
    let midnight = address_env("MIDNIGHT_ADDRESS")?.unwrap_or(BASE_MIDNIGHT);
    let ratifier = address_env("RATIFIER_ADDRESS")?.unwrap_or(BASE_ECRECOVER_RATIFIER);

    let market = MarketBuilder::new(chain_id, midnight, [0x22; 20])
        .collateral(
            [0x33; 20],
            U256::from(770_000_000_000_000_000u64),
            U256::from(250_000_000_000_000_000u64),
            [0x44; 20],
        )
        .maturity(maturity)
        .build_checked()?;
    let offer = OfferBuilder::new(market, maker)
        .lend()
        .tick(5_000)
        .start(now)
        .expiry(maturity)
        .ratifier(ratifier)
        .max_units(1_000_000)
        .build_checked()?;

    let descriptor = OfferTree::from_entries([offer])?;
    let offer = descriptor.offers[0].clone();
    let tree = descriptor.tree;
    let digest = tree_digest(
        tree.root(),
        tree.height(),
        word_from_u64(chain_id),
        &ratifier,
    );
    let signature = maker_signer.sign_digest(&digest)?;
    let ratifier_data = encode_ratifier_data(&signature, &tree.root(), 0, &tree.proof(0).unwrap());
    let payload = Payload::encode(&[PayloadItem {
        offer,
        ratifier_data,
    }])?;

    if std::env::var_os("VALIDATE_PAYLOAD").is_some() {
        let validation = MidnightApi::default()
            .validate_payload(chain_id, &payload, None)
            .await?;
        if !validation.valid {
            return Err(format!("API rejected payload: {:?}", validation.issues).into());
        }
    }

    let transaction = match address_env("MEMPOOL_ADDRESS")? {
        Some(address) => mempool_submission_to(address, payload, [])?,
        None => mempool_submission(chain_id, payload, [])?,
    };
    let expected_data = transaction.data.clone();
    let expected_to = transaction.to;
    let receipt = provider
        .send_transaction(transaction.into())
        .await?
        .get_receipt()
        .await?;
    let transaction = provider
        .get_transaction_by_hash(receipt.transaction_hash)
        .await?
        .ok_or("mined transaction is missing")?;
    if transaction.inner.to() != Some(alloy::primitives::Address::from_slice(&expected_to))
        || transaction.inner.value() != alloy::primitives::U256::ZERO
        || transaction.inner.input().as_ref() != expected_data
    {
        return Err("mined transaction does not match the publication request".into());
    }
    println!("published 0x{}", hex::encode(receipt.transaction_hash));
    Ok(())
}

fn address_env(name: &str) -> Result<Option<Address>, Box<dyn std::error::Error>> {
    std::env::var(name)
        .ok()
        .map(|value| decode_fixed::<20>(&value))
        .transpose()
}

fn decode_fixed<const N: usize>(value: &str) -> Result<[u8; N], Box<dyn std::error::Error>> {
    let bytes = hex::decode(
        value
            .strip_prefix("0x")
            .or_else(|| value.strip_prefix("0X"))
            .unwrap_or(value),
    )?;
    Ok(bytes.try_into().map_err(|_| "incorrect hex length")?)
}
