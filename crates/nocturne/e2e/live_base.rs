//! Small, reversible Base mainnet lifecycle test.

use std::time::{SystemTime, UNIX_EPOCH};

use alloy::{
    network::EthereumWallet,
    providers::{Provider, ProviderBuilder},
    signers::local::PrivateKeySigner,
};
use nocturne::*;

#[path = "support/mod.rs"]
pub(crate) mod support;
use support::{
    satisfy, send, snapshot, token_balance, BoxError, LifecycleInvariants, LifecycleJournal,
};

const USDC: Address = [
    0x83, 0x35, 0x89, 0xfc, 0xd6, 0xed, 0xb6, 0xe0, 0x8f, 0x4c, 0x7c, 0x32, 0xd4, 0xf7, 0x1b, 0x54,
    0xbd, 0xa0, 0x29, 0x13,
];
const CBBTC: Address = [
    0xcb, 0xb7, 0xc0, 0x00, 0x0a, 0xb8, 0x8b, 0x47, 0x3b, 0x1f, 0x5a, 0xfd, 0x9e, 0xf8, 0x08, 0x44,
    0x0e, 0xed, 0x33, 0xbf,
];
const DIRECT_COLLATERAL: u64 = 300;
const ATOMIC_COLLATERAL: u64 = 500;
const BORROW_ASSETS: u64 = 100_000; // 0.10 USDC

pub async fn run(resume: bool) -> Result<(), BoxError> {
    if std::env::var("LIVE_BASE_CONFIRM").as_deref() != Ok("I_UNDERSTAND") {
        return Err("set LIVE_BASE_CONFIRM=I_UNDERSTAND to enable Base transactions".into());
    }
    let rpc_url = std::env::var("RPC_URL")?;
    let signer: PrivateKeySigner = std::env::var("PRIVATE_KEY")?.parse()?;
    let owner_alloy = signer.address();
    let mut owner = [0u8; 20];
    owner.copy_from_slice(owner_alloy.as_slice());
    let provider = ProviderBuilder::new()
        .wallet(EthereumWallet::from(signer))
        .connect_http(rpc_url.parse()?);
    if provider.get_chain_id().await? != BASE_CHAIN_ID {
        return Err("RPC is not Base mainnet".into());
    }
    println!("PREFLIGHT wallet={owner_alloy}");

    let mut journal = if resume {
        LifecycleJournal::load_for("base-taker")?
    } else {
        LifecycleJournal::start("base-taker")?
    };
    let initial_usdc = token_balance(&provider, USDC, owner_alloy).await?;
    let initial_cbbtc = token_balance(&provider, CBBTC, owner_alloy).await?;
    println!("PREFLIGHT usdc={initial_usdc} cbbtc={initial_cbbtc}");
    let resuming_repay = resume;
    if initial_usdc < U256::from(200_000u64)
        || (!resuming_repay && initial_cbbtc < U256::from(DIRECT_COLLATERAL + ATOMIC_COLLATERAL))
    {
        return Err("insufficient token safety balance for the live test".into());
    }

    let books = MidnightApi::default()
        .fetch_books(&BooksQuery {
            chain_ids: vec![BASE_CHAIN_ID],
            loan_tokens: vec![USDC],
            collateral_tokens: vec![CBBTC],
            limit: Some(20),
            ..Default::default()
        })
        .await?;
    let book = books
        .data
        .into_iter()
        .find(|book| !book.bids.is_empty())
        .ok_or("no live USDC/cbBTC bid book")?;
    let quote = MidnightApi::default()
        .fetch_quote(
            book.market_id,
            BookSide::Bids,
            QuoteTarget::Assets(U256::from(BORROW_ASSETS)),
            None,
        )
        .await?;
    println!(
        "PREFLIGHT quote=ok market=0x{}",
        hex::encode(book.market_id)
    );
    let market = quote
        .takeable_offers
        .first()
        .ok_or("live quote returned no offers")?
        .offer
        .market
        .clone();
    let id = market_id(&market);
    journal.set_context(format!("0x{}", hex::encode(id)), None)?;
    let initial = snapshot(
        &provider,
        market.midnight,
        id,
        owner_alloy,
        owner_alloy,
        USDC,
        CBBTC,
    )
    .await?;
    let existing_debt = initial.debt;
    let existing_collateral = initial.collateral;
    if resuming_repay {
        if existing_debt == U256::ZERO && existing_collateral == U256::ZERO {
            LifecycleInvariants::clean(initial_cbbtc).assert(initial)?;
            journal.complete()?;
            println!("RESULT already reconciled; journal marked complete");
            return Ok(());
        }
        if existing_debt == U256::ZERO || existing_collateral == U256::ZERO {
            return Err("partial position state cannot be reconciled automatically".into());
        }
        let bundles = midnight_bundles(BASE_CHAIN_ID)?;
        let repay_plan =
            repay_withdraw_requirement_plan(&market, U256::from(existing_debt), owner, bundles)?;
        satisfy(
            &provider,
            owner_alloy,
            "taker",
            &repay_plan,
            "repay",
            &mut journal,
        )
        .await?;
        let deadline = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs() + 600;
        send(
            &provider,
            owner_alloy,
            "taker",
            "repay-withdraw",
            repay_withdraw_collateral(&RepayWithdraw {
                market: &market,
                repay_assets: U256::from(existing_debt),
                withdraw_collateral: Some(CollateralDeposit {
                    collateral_index: 0,
                    assets: U256::from(existing_collateral),
                }),
                on_behalf: owner,
                deadline: U256::from(deadline),
            })?,
            &mut journal,
        )
        .await?;
        let expected_cbbtc = initial_cbbtc + existing_collateral;
        LifecycleInvariants::clean(expected_cbbtc).assert(
            snapshot(
                &provider,
                market.midnight,
                id,
                owner_alloy,
                owner_alloy,
                USDC,
                CBBTC,
            )
            .await?,
        )?;
        journal.complete()?;
        println!("MARKET 0x{}", hex::encode(id));
        println!("REPAID_UNITS {existing_debt}");
        println!("RESULT debt=0 collateral=0 cbbtc_restored=true");
        return Ok(());
    }
    if existing_debt != U256::ZERO || existing_collateral != U256::ZERO {
        return Err("wallet already has a position in the selected market".into());
    }
    println!("PREFLIGHT initial-position=clean");

    let direct_plan =
        supply_collateral_requirement_plan(&market, 0, U256::from(DIRECT_COLLATERAL), owner)?;
    satisfy(
        &provider,
        owner_alloy,
        "taker",
        &direct_plan,
        "direct-supply",
        &mut journal,
    )
    .await?;
    send(
        &provider,
        owner_alloy,
        "taker",
        "direct-supply",
        supply_collateral(&market, 0, U256::from(DIRECT_COLLATERAL), owner)?,
        &mut journal,
    )
    .await?;

    let bundles = midnight_bundles(BASE_CHAIN_ID)?;
    let borrow_plan = supply_collateral_take_borrow_requirement_plan(
        &market,
        0,
        U256::from(ATOMIC_COLLATERAL),
        owner,
        bundles,
    )?;
    satisfy(
        &provider,
        owner_alloy,
        "taker",
        &borrow_plan,
        "atomic-borrow",
        &mut journal,
    )
    .await?;
    let deadline = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs() + 600;
    send(
        &provider,
        owner_alloy,
        "taker",
        "atomic-borrow",
        supply_collateral_take_borrow(
            &TakeBorrow {
                market: &market,
                loan_assets: U256::from(BORROW_ASSETS),
                max_units: quote.available_units,
                taker: owner,
                offers: &quote.takeable_offers,
                deadline: U256::from(deadline),
            },
            CollateralDeposit {
                collateral_index: 0,
                assets: U256::from(ATOMIC_COLLATERAL),
            },
        )?,
        &mut journal,
    )
    .await?;

    let open = snapshot(
        &provider,
        market.midnight,
        id,
        owner_alloy,
        owner_alloy,
        USDC,
        CBBTC,
    )
    .await?;
    LifecycleInvariants::borrowed(U256::from(DIRECT_COLLATERAL + ATOMIC_COLLATERAL), false)
        .assert(open)?;
    let debt = open.debt;
    let collateral_assets = open.collateral;
    let repay_plan = repay_withdraw_requirement_plan(&market, U256::from(debt), owner, bundles)?;
    satisfy(
        &provider,
        owner_alloy,
        "taker",
        &repay_plan,
        "repay",
        &mut journal,
    )
    .await?;
    send(
        &provider,
        owner_alloy,
        "taker",
        "repay-withdraw",
        repay_withdraw_collateral(&RepayWithdraw {
            market: &market,
            repay_assets: U256::from(debt),
            withdraw_collateral: Some(CollateralDeposit {
                collateral_index: 0,
                assets: U256::from(collateral_assets),
            }),
            on_behalf: owner,
            deadline: U256::from(deadline),
        })?,
        &mut journal,
    )
    .await?;

    LifecycleInvariants::clean(initial_cbbtc).assert(
        snapshot(
            &provider,
            market.midnight,
            id,
            owner_alloy,
            owner_alloy,
            USDC,
            CBBTC,
        )
        .await?,
    )?;
    journal.complete()?;
    let final_usdc = token_balance(&provider, USDC, owner_alloy).await?;
    println!("MARKET 0x{}", hex::encode(id));
    println!("BORROWED_USDC {BORROW_ASSETS}");
    println!("REPAID_UNITS {debt}");
    println!("USDC_DELTA {}", initial_usdc.saturating_sub(final_usdc));
    println!("RESULT debt=0 collateral=0 cbbtc_restored=true");
    Ok(())
}
