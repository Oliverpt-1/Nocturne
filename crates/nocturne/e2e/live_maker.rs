//! Two-wallet Base mainnet maker lifecycle test.

use std::{thread, time::Duration, time::SystemTime, time::UNIX_EPOCH};

use alloy::{
    network::EthereumWallet,
    providers::{Provider, ProviderBuilder},
    signers::local::PrivateKeySigner,
};
use nocturne::*;

use super::live_base::support::{
    satisfy, sdk_address, send, snapshot, token_balance, LifecycleInvariants, LifecycleJournal,
};

const USDC: Address = [
    0x83, 0x35, 0x89, 0xfc, 0xd6, 0xed, 0xb6, 0xe0, 0x8f, 0x4c, 0x7c, 0x32, 0xd4, 0xf7, 0x1b, 0x54,
    0xbd, 0xa0, 0x29, 0x13,
];
const CBBTC: Address = [
    0xcb, 0xb7, 0xc0, 0x00, 0x0a, 0xb8, 0x8b, 0x47, 0x3b, 0x1f, 0x5a, 0xfd, 0x9e, 0xf8, 0x08, 0x44,
    0x0e, 0xed, 0x33, 0xbf,
];
const OFFER_ASSETS: u64 = 110_000_000; // 110 USDC, above the current router floor.
const BORROW_ASSETS: u64 = 100_000; // Fill only 0.10 USDC.
const COLLATERAL_ASSETS: u64 = 800;

fn key_bytes(name: &str) -> Result<[u8; 32], Box<dyn std::error::Error>> {
    let value = std::env::var(name)?;
    hex::decode(value.trim_start_matches("0x"))?
        .try_into()
        .map_err(|_| format!("{name} must contain exactly 32 bytes").into())
}

fn parse_word(value: &str) -> Result<Word, Box<dyn std::error::Error>> {
    hex::decode(value.trim_start_matches("0x"))?
        .try_into()
        .map_err(|_| "root must contain exactly 32 bytes".into())
}

fn market_from_book(book: &BookMarket) -> Result<Market, MarketBuildError> {
    let mut builder = MarketBuilder::new(book.chain_id, book.midnight, book.loan_token);
    for item in &book.collaterals {
        builder = builder.collateral(item.token, item.lltv, item.liquidation_cursor, item.oracle);
    }
    builder
        .maturity(book.maturity)
        .rcf_threshold(book.rcf_threshold)
        .enter_gate(book.enter_gate)
        .liquidator_gate(book.liquidator_gate)
        .build_checked()
}

pub async fn run(resume: bool, cleanup: bool) -> Result<(), Box<dyn std::error::Error>> {
    if std::env::var("LIVE_MAKER_CONFIRM").as_deref() != Ok("I_UNDERSTAND") {
        return Err("set LIVE_MAKER_CONFIRM=I_UNDERSTAND to enable Base transactions".into());
    }
    let rpc_url = std::env::var("RPC_URL")?;
    let maker_key = std::env::var("PRIVATE_KEY_BIGGER")?;
    let taker_key = std::env::var("PRIVATE_KEY_Wallet_1")?;
    let maker_wallet_signer: PrivateKeySigner = maker_key.parse()?;
    let taker_wallet_signer: PrivateKeySigner = taker_key.parse()?;
    let maker_alloy = maker_wallet_signer.address();
    let taker_alloy = taker_wallet_signer.address();
    let maker = sdk_address(maker_alloy);
    let taker = sdk_address(taker_alloy);
    let maker_provider = ProviderBuilder::new()
        .wallet(EthereumWallet::from(maker_wallet_signer))
        .connect_http(rpc_url.parse()?);
    let taker_provider = ProviderBuilder::new()
        .wallet(EthereumWallet::from(taker_wallet_signer))
        .connect_http(rpc_url.parse()?);
    if maker_provider.get_chain_id().await? != BASE_CHAIN_ID
        || taker_provider.get_chain_id().await? != BASE_CHAIN_ID
    {
        return Err("RPC is not Base mainnet".into());
    }
    println!("PREFLIGHT maker={maker_alloy} taker={taker_alloy}");
    let mut journal = if resume {
        LifecycleJournal::load_for("base-maker")?
    } else if cleanup {
        LifecycleJournal::load_or_start("base-maker")?
    } else {
        LifecycleJournal::start("base-maker")?
    };

    if cleanup {
        send(
            &maker_provider,
            maker_alloy,
            "maker",
            "revoke-maker-allowance",
            MidnightTransaction {
                to: USDC,
                value: U256::ZERO,
                data: encode_erc20_approve_calldata(&BASE_MIDNIGHT, U256::ZERO),
            },
            &mut journal,
        )
        .await?;
        send(
            &maker_provider,
            maker_alloy,
            "maker",
            "revoke-ratifier-authorization",
            set_is_authorized_to(BASE_MIDNIGHT, BASE_ECRECOVER_RATIFIER, false, maker)?,
            &mut journal,
        )
        .await?;
        println!("RESULT maker-allowance=0 ratifier-authorized=false");
        return Ok(());
    }

    let maker_usdc_before = token_balance(&maker_provider, USDC, maker_alloy).await?;
    let taker_usdc_before = token_balance(&taker_provider, USDC, taker_alloy).await?;
    let taker_cbbtc_before = token_balance(&taker_provider, CBBTC, taker_alloy).await?;
    let resume_root = if resume {
        Some(parse_word(
            &journal
                .root
                .clone()
                .ok_or("maker journal has no offer root")?,
        )?)
    } else {
        None
    };
    if resume_root.is_none()
        && (maker_usdc_before < U256::from(OFFER_ASSETS)
            || taker_usdc_before < U256::from(200_000u64)
            || taker_cbbtc_before < U256::from(COLLATERAL_ASSETS))
    {
        return Err("insufficient token balances for maker lifecycle".into());
    }

    let api = MidnightApi::default();
    let books = api
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
        .ok_or("no active USDC/cbBTC bid market")?;
    let market = market_from_book(&book)?;
    let id = market_id(&market);
    let current = snapshot(
        &taker_provider,
        market.midnight,
        id,
        taker_alloy,
        maker_alloy,
        USDC,
        CBBTC,
    )
    .await?;
    let current_debt = current.debt;
    let current_collateral = current.collateral;
    let current_credit = current.credit;
    if let Some(root) = resume_root {
        if (current_debt == U256::ZERO) != (current_collateral == U256::ZERO) {
            return Err("partial borrower position cannot be reconciled automatically".into());
        }
        send(
            &maker_provider,
            maker_alloy,
            "maker",
            "cancel-offer-root",
            MidnightTransaction {
                to: BASE_ECRECOVER_RATIFIER,
                value: U256::ZERO,
                data: encode_cancel_root_calldata(&maker, &root),
            },
            &mut journal,
        )
        .await?;
        if current_debt != U256::ZERO {
            let bundles = midnight_bundles(BASE_CHAIN_ID)?;
            let repay_plan =
                repay_withdraw_requirement_plan(&market, current_debt, taker, bundles)?;
            satisfy(
                &taker_provider,
                taker_alloy,
                "taker",
                &repay_plan,
                "taker-repay",
                &mut journal,
            )
            .await?;
            let deadline = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs() + 600;
            send(
                &taker_provider,
                taker_alloy,
                "taker",
                "repay-withdraw",
                repay_withdraw_collateral(&RepayWithdraw {
                    market: &market,
                    repay_assets: current_debt,
                    withdraw_collateral: Some(CollateralDeposit {
                        collateral_index: 0,
                        assets: current_collateral,
                    }),
                    on_behalf: taker,
                    deadline: U256::from(deadline),
                })?,
                &mut journal,
            )
            .await?;
        }
        if current_credit != U256::ZERO {
            send(
                &maker_provider,
                maker_alloy,
                "maker",
                "redeem-credit",
                redeem(&market, current_credit, maker, Some(maker))?,
                &mut journal,
            )
            .await?;
        }
        let expected_cbbtc = taker_cbbtc_before + current_collateral;
        LifecycleInvariants::clean(expected_cbbtc).assert(
            snapshot(
                &taker_provider,
                market.midnight,
                id,
                taker_alloy,
                maker_alloy,
                USDC,
                CBBTC,
            )
            .await?,
        )?;
        journal.complete()?;
        println!("MARKET 0x{}", hex::encode(id));
        println!("ROOT 0x{}", hex::encode(root));
        println!("REPAID_UNITS {current_debt}");
        println!("REDEEMED_UNITS {current_credit}");
        println!("RESULT indexed=true debt=0 collateral=0 credit=0 cbbtc_restored=true");
        return Ok(());
    }
    if current_debt != U256::ZERO
        || current_collateral != U256::ZERO
        || current_credit != U256::ZERO
    {
        return Err("selected wallets already have state in the target market".into());
    }

    let now = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs();
    let expiry = (now + 86_400).min(book.maturity - 1);
    let tick = book.bids.first().ok_or("book has no bid")?.tick;
    let offer = OfferBuilder::new(market.clone(), maker)
        .lend()
        .tick(tick)
        .start(now)
        .expiry(expiry)
        .ratifier(BASE_ECRECOVER_RATIFIER)
        .max_assets(OFFER_ASSETS as u128)
        .build_checked()?;
    let descriptor = OfferTree::from_entries([offer])?;
    let offer = descriptor.offers[0].clone();
    let tree = descriptor.tree;
    let root = tree.root();
    journal.set_context(
        format!("0x{}", hex::encode(id)),
        Some(format!("0x{}", hex::encode(root))),
    )?;
    let maker_signer = LocalSigner::from_bytes(&key_bytes("PRIVATE_KEY_BIGGER")?)?;
    if maker_signer.address() != maker {
        return Err("maker signer mismatch".into());
    }
    let signature = maker_signer.sign_digest(&tree_digest(
        root,
        tree.height(),
        word_from_u64(BASE_CHAIN_ID),
        &BASE_ECRECOVER_RATIFIER,
    ))?;
    let ratifier_data = encode_ratifier_data(&signature, &root, 0, &tree.proof(0)?);
    let payload = Payload::encode(&[PayloadItem {
        offer: offer.clone(),
        ratifier_data,
    }])?;
    let validation = api.validate_payload(BASE_CHAIN_ID, &payload, None).await?;
    if !validation.valid {
        return Err(format!("router rejected payload: {:?}", validation.issues).into());
    }
    println!(
        "PREFLIGHT router-validation=accepted root=0x{}",
        hex::encode(root)
    );

    let maker_plan = RequirementPlan {
        chain_id: BASE_CHAIN_ID,
        approvals: vec![ApprovalRequest {
            token: USDC,
            owner: maker,
            spender: market.midnight,
            amount: U256::from(OFFER_ASSETS),
        }],
        authorization: Some(AuthorizationRequest {
            midnight: market.midnight,
            owner: maker,
            authorized: BASE_ECRECOVER_RATIFIER,
        }),
    };
    satisfy(
        &maker_provider,
        maker_alloy,
        "maker",
        &maker_plan,
        "maker",
        &mut journal,
    )
    .await?;
    let publication = mempool_submission(BASE_CHAIN_ID, payload, [])?;
    send(
        &maker_provider,
        maker_alloy,
        "maker",
        "publish-offer",
        MidnightTransaction {
            to: publication.to,
            value: publication.value,
            data: publication.data,
        },
        &mut journal,
    )
    .await?;

    let mut indexed = None;
    for attempt in 1..=10 {
        let page = api
            .fetch_takeable_offers(
                maker,
                &TakeableOffersQuery {
                    market_ids: vec![id],
                    limit: Some(20),
                    ..Default::default()
                },
            )
            .await?;
        indexed = page
            .data
            .into_iter()
            .find(|item| item.offer == offer && item.offer.maker == maker);
        if indexed.is_some() {
            println!("PREFLIGHT router-indexed attempt={attempt}");
            break;
        }
        thread::sleep(Duration::from_secs(4));
    }
    let indexed = indexed.ok_or("offer was published but not indexed within 40 seconds")?;

    let bundles = midnight_bundles(BASE_CHAIN_ID)?;
    let borrow_plan = supply_collateral_take_borrow_requirement_plan(
        &market,
        0,
        U256::from(COLLATERAL_ASSETS),
        taker,
        bundles,
    )?;
    satisfy(
        &taker_provider,
        taker_alloy,
        "taker",
        &borrow_plan,
        "taker-borrow",
        &mut journal,
    )
    .await?;
    let deadline = now + 600;
    send(
        &taker_provider,
        taker_alloy,
        "taker",
        "take-maker-offer",
        supply_collateral_take_borrow(
            &TakeBorrow {
                market: &market,
                loan_assets: U256::from(BORROW_ASSETS),
                max_units: indexed.units,
                taker,
                offers: &[indexed],
                deadline: U256::from(deadline),
            },
            CollateralDeposit {
                collateral_index: 0,
                assets: U256::from(COLLATERAL_ASSETS),
            },
        )?,
        &mut journal,
    )
    .await?;

    let open = snapshot(
        &taker_provider,
        market.midnight,
        id,
        taker_alloy,
        maker_alloy,
        USDC,
        CBBTC,
    )
    .await?;
    LifecycleInvariants::borrowed(U256::from(COLLATERAL_ASSETS), true).assert(open)?;
    let open_debt = open.debt;
    let open_collateral = open.collateral;
    let maker_credit = open.credit;

    let repay_plan =
        repay_withdraw_requirement_plan(&market, U256::from(open_debt), taker, bundles)?;
    satisfy(
        &taker_provider,
        taker_alloy,
        "taker",
        &repay_plan,
        "taker-repay",
        &mut journal,
    )
    .await?;
    send(
        &taker_provider,
        taker_alloy,
        "taker",
        "repay-withdraw",
        repay_withdraw_collateral(&RepayWithdraw {
            market: &market,
            repay_assets: U256::from(open_debt),
            withdraw_collateral: Some(CollateralDeposit {
                collateral_index: 0,
                assets: U256::from(open_collateral),
            }),
            on_behalf: taker,
            deadline: U256::from(deadline),
        })?,
        &mut journal,
    )
    .await?;

    send(
        &maker_provider,
        maker_alloy,
        "maker",
        "redeem-credit",
        redeem(&market, U256::from(maker_credit), maker, Some(maker))?,
        &mut journal,
    )
    .await?;
    send(
        &maker_provider,
        maker_alloy,
        "maker",
        "cancel-offer-root",
        MidnightTransaction {
            to: BASE_ECRECOVER_RATIFIER,
            value: U256::ZERO,
            data: encode_cancel_root_calldata(&maker, &root),
        },
        &mut journal,
    )
    .await?;

    LifecycleInvariants::clean(taker_cbbtc_before).assert(
        snapshot(
            &taker_provider,
            market.midnight,
            id,
            taker_alloy,
            maker_alloy,
            USDC,
            CBBTC,
        )
        .await?,
    )?;
    journal.complete()?;
    println!("MARKET 0x{}", hex::encode(id));
    println!("ROOT 0x{}", hex::encode(root));
    println!("BORROWED_USDC {BORROW_ASSETS}");
    println!("REPAID_UNITS {open_debt}");
    println!("REDEEMED_UNITS {maker_credit}");
    println!("RESULT indexed=true debt=0 collateral=0 credit=0 cbbtc_restored=true");
    Ok(())
}
