//! Wallet-agnostic transaction builders for complete Midnight position workflows.

use crate::{
    decode_any_ratifier_data, encode_bundle_calldata, encode_repay_withdraw_collateral_calldata,
    encode_set_is_authorized_calldata, encode_supply_collateral_calldata, encode_withdraw_calldata,
    market_id, word_to_u256, Address, BundleCall, BundleKind, BundleSide, CollateralSupply,
    CollateralWithdrawal, Market, OfferFill, TakeableOffer, TokenPermit, BASE_MIDNIGHT_BUNDLES,
    U256,
};

/// A zero-value EVM transaction produced by a Nocturne action builder.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MidnightTransaction {
    pub to: Address,
    pub value: U256,
    pub data: Vec<u8>,
}

#[cfg(feature = "alloy-wallet")]
impl From<MidnightTransaction> for alloy::rpc::types::TransactionRequest {
    fn from(transaction: MidnightTransaction) -> Self {
        Self::default()
            .to(alloy::primitives::Address::from_slice(&transaction.to))
            .value(alloy::primitives::U256::from_be_bytes(
                transaction.value.to_be_bytes::<32>(),
            ))
            .input(alloy::primitives::Bytes::from(transaction.data).into())
    }
}

/// Errors returned before a position-management transaction is constructed.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum ActionError {
    #[error("Midnight actions are not registered for chain {0}")]
    UnsupportedChain(u64),
    #[error("market chain id does not match requested chain {0}")]
    ChainIdMismatch(u64),
    #[error("market Midnight address does not match the transaction destination")]
    MidnightMismatch,
    #[error("address must not be zero")]
    ZeroAddress,
    #[error("{0} must be greater than zero")]
    ZeroAmount(&'static str),
    #[error("collateral index {index} is not configured; market has {collaterals} collaterals")]
    InvalidCollateralIndex { index: usize, collaterals: usize },
    #[error("at least one takeable offer is required")]
    EmptyOffers,
    #[error("takeable offer {0} belongs to a different market")]
    MarketMismatch(usize),
    #[error("takeable offer {0} is on the wrong side of the book")]
    SideMismatch(usize),
    #[error("takeable offer {0} has zero units")]
    ZeroOfferUnits(usize),
    #[error("takeable offer {0} belongs to the taker; Midnight rejects self-takes")]
    SelfTake(usize),
    #[error("takeable offer {0} contains unsupported ratifier data")]
    RatifierData(usize),
    #[error("repay assets and collateral withdrawal cannot both be zero")]
    EmptyRepayWithdraw,
}

/// Inputs for lending by taking borrow-side asks through MidnightBundles.
#[derive(Clone, Debug)]
pub struct TakeLend<'a> {
    pub market: &'a Market,
    pub assets: U256,
    pub min_units: U256,
    pub taker: Address,
    pub offers: &'a [TakeableOffer],
    pub deadline: U256,
}

/// Inputs for borrowing by taking lend-side bids through MidnightBundles.
#[derive(Clone, Debug)]
pub struct TakeBorrow<'a> {
    pub market: &'a Market,
    pub loan_assets: U256,
    pub max_units: U256,
    pub taker: Address,
    pub offers: &'a [TakeableOffer],
    pub deadline: U256,
}

/// Optional collateral supplied atomically before a borrow take.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CollateralDeposit {
    pub collateral_index: usize,
    pub assets: U256,
}

/// Inputs for repaying debt and/or withdrawing collateral through MidnightBundles.
#[derive(Clone, Debug)]
pub struct RepayWithdraw<'a> {
    pub market: &'a Market,
    pub repay_assets: U256,
    pub withdraw_collateral: Option<CollateralDeposit>,
    pub on_behalf: Address,
    pub deadline: U256,
}

/// Resolve the registered MidnightBundles deployment for a chain.
pub fn midnight_bundles(chain_id: u64) -> Result<Address, ActionError> {
    match chain_id {
        crate::BASE_CHAIN_ID => Ok(BASE_MIDNIGHT_BUNDLES),
        _ => Err(ActionError::UnsupportedChain(chain_id)),
    }
}

fn transaction(to: Address, data: Vec<u8>) -> Result<MidnightTransaction, ActionError> {
    if to == [0; 20] {
        return Err(ActionError::ZeroAddress);
    }
    Ok(MidnightTransaction {
        to,
        value: U256::ZERO,
        data,
    })
}

fn validate_address(address: Address) -> Result<(), ActionError> {
    if address == [0; 20] {
        return Err(ActionError::ZeroAddress);
    }
    Ok(())
}

fn validate_market(market: &Market, chain_id: u64, midnight: Address) -> Result<(), ActionError> {
    if word_to_u256(&market.chain_id) != U256::from(chain_id) {
        return Err(ActionError::ChainIdMismatch(chain_id));
    }
    if market.midnight != midnight {
        return Err(ActionError::MidnightMismatch);
    }
    Ok(())
}

fn collateral(market: &Market, index: usize) -> Result<(), ActionError> {
    if index >= market.collateral_params.len() {
        return Err(ActionError::InvalidCollateralIndex {
            index,
            collaterals: market.collateral_params.len(),
        });
    }
    Ok(())
}

fn fills(
    market: &Market,
    taker: &Address,
    offers: &[TakeableOffer],
    expected_buy: bool,
) -> Result<Vec<OfferFill>, ActionError> {
    if offers.is_empty() {
        return Err(ActionError::EmptyOffers);
    }
    let expected_market = market_id(market);
    offers
        .iter()
        .enumerate()
        .map(|(index, take)| {
            if take.market_id != expected_market || take.offer.market != *market {
                return Err(ActionError::MarketMismatch(index));
            }
            if take.offer.buy != expected_buy {
                return Err(ActionError::SideMismatch(index));
            }
            if take.units == U256::ZERO {
                return Err(ActionError::ZeroOfferUnits(index));
            }
            if take.offer.maker == *taker {
                return Err(ActionError::SelfTake(index));
            }
            let ratifier_data = decode_any_ratifier_data(&take.ratifier_data)
                .map_err(|_| ActionError::RatifierData(index))?;
            Ok(OfferFill {
                offer: take.offer.clone(),
                ratifier_data_raw: take.ratifier_data.clone(),
                ratifier_data,
                units: take.units,
            })
        })
        .collect()
}

/// Build a direct collateral-supply transaction for Base.
pub fn supply_collateral(
    market: &Market,
    collateral_index: usize,
    assets: U256,
    on_behalf: Address,
) -> Result<MidnightTransaction, ActionError> {
    supply_collateral_to(
        crate::BASE_CHAIN_ID,
        crate::BASE_MIDNIGHT,
        market,
        collateral_index,
        assets,
        on_behalf,
    )
}

/// Build a direct collateral-supply transaction for an explicit deployment.
pub fn supply_collateral_to(
    chain_id: u64,
    midnight: Address,
    market: &Market,
    collateral_index: usize,
    assets: U256,
    on_behalf: Address,
) -> Result<MidnightTransaction, ActionError> {
    validate_market(market, chain_id, midnight)?;
    validate_address(on_behalf)?;
    collateral(market, collateral_index)?;
    if assets == U256::ZERO {
        return Err(ActionError::ZeroAmount("collateral assets"));
    }
    transaction(
        midnight,
        encode_supply_collateral_calldata(market, U256::from(collateral_index), assets, &on_behalf),
    )
}

/// Build a lend take against Base MidnightBundles.
pub fn take_lend(input: &TakeLend<'_>) -> Result<MidnightTransaction, ActionError> {
    validate_market(input.market, crate::BASE_CHAIN_ID, crate::BASE_MIDNIGHT)?;
    take_lend_to(crate::BASE_CHAIN_ID, BASE_MIDNIGHT_BUNDLES, input)
}

/// Build a lend take against an explicit MidnightBundles deployment.
pub fn take_lend_to(
    chain_id: u64,
    bundles: Address,
    input: &TakeLend<'_>,
) -> Result<MidnightTransaction, ActionError> {
    validate_market(input.market, chain_id, input.market.midnight)?;
    validate_address(input.taker)?;
    if input.assets == U256::ZERO {
        return Err(ActionError::ZeroAmount("lend assets"));
    }
    let bundle = BundleCall {
        kind: BundleKind::BuyWithAssetsTarget,
        target: input.assets,
        limit: input.min_units,
        taker: input.taker,
        reduce_only: false,
        side: BundleSide::Buy {
            loan_token_permit: TokenPermit {
                kind: 0,
                data: vec![],
            },
            collateral_withdrawals: vec![],
            collateral_receiver: [0; 20],
        },
        fills: fills(input.market, &input.taker, input.offers, false)?,
        referral_fee_pct: U256::ZERO,
        referral_fee_recipient: [0; 20],
        max_continuous_fee: U256::MAX,
        deadline: input.deadline,
    };
    transaction(bundles, encode_bundle_calldata(&bundle))
}

/// Build a borrow take against Base MidnightBundles without supplying new collateral.
pub fn take_borrow(input: &TakeBorrow<'_>) -> Result<MidnightTransaction, ActionError> {
    validate_market(input.market, crate::BASE_CHAIN_ID, crate::BASE_MIDNIGHT)?;
    take_borrow_to(crate::BASE_CHAIN_ID, BASE_MIDNIGHT_BUNDLES, input, None)
}

/// Build an atomic supply-collateral-and-borrow transaction against Base MidnightBundles.
pub fn supply_collateral_take_borrow(
    input: &TakeBorrow<'_>,
    deposit: CollateralDeposit,
) -> Result<MidnightTransaction, ActionError> {
    validate_market(input.market, crate::BASE_CHAIN_ID, crate::BASE_MIDNIGHT)?;
    take_borrow_to(
        crate::BASE_CHAIN_ID,
        BASE_MIDNIGHT_BUNDLES,
        input,
        Some(deposit),
    )
}

/// Build a borrow take against an explicit MidnightBundles deployment.
pub fn take_borrow_to(
    chain_id: u64,
    bundles: Address,
    input: &TakeBorrow<'_>,
    deposit: Option<CollateralDeposit>,
) -> Result<MidnightTransaction, ActionError> {
    validate_market(input.market, chain_id, input.market.midnight)?;
    validate_address(input.taker)?;
    if input.loan_assets == U256::ZERO {
        return Err(ActionError::ZeroAmount("borrow assets"));
    }
    if input.max_units == U256::ZERO {
        return Err(ActionError::ZeroAmount("maximum units"));
    }
    let collateral_supplies = match deposit {
        Some(deposit) => {
            collateral(input.market, deposit.collateral_index)?;
            if deposit.assets == U256::ZERO {
                return Err(ActionError::ZeroAmount("collateral assets"));
            }
            vec![CollateralSupply {
                collateral_index: U256::from(deposit.collateral_index),
                assets: deposit.assets,
                permit: TokenPermit {
                    kind: 0,
                    data: vec![],
                },
            }]
        }
        None => vec![],
    };
    let bundle = BundleCall {
        kind: BundleKind::SellWithAssetsTarget,
        target: input.loan_assets,
        limit: input.max_units,
        taker: input.taker,
        reduce_only: false,
        side: BundleSide::Sell {
            receiver: input.taker,
            collateral_supplies,
        },
        fills: fills(input.market, &input.taker, input.offers, true)?,
        referral_fee_pct: U256::ZERO,
        referral_fee_recipient: [0; 20],
        max_continuous_fee: U256::MAX,
        deadline: input.deadline,
    };
    transaction(bundles, encode_bundle_calldata(&bundle))
}

/// Build a repay-and/or-withdraw transaction against Base MidnightBundles.
pub fn repay_withdraw_collateral(
    input: &RepayWithdraw<'_>,
) -> Result<MidnightTransaction, ActionError> {
    validate_market(input.market, crate::BASE_CHAIN_ID, crate::BASE_MIDNIGHT)?;
    repay_withdraw_collateral_to(crate::BASE_CHAIN_ID, BASE_MIDNIGHT_BUNDLES, input)
}

/// Build a repay-and/or-withdraw transaction against an explicit MidnightBundles deployment.
pub fn repay_withdraw_collateral_to(
    chain_id: u64,
    bundles: Address,
    input: &RepayWithdraw<'_>,
) -> Result<MidnightTransaction, ActionError> {
    validate_market(input.market, chain_id, input.market.midnight)?;
    validate_address(input.on_behalf)?;
    let withdrawals = match input.withdraw_collateral {
        Some(withdrawal) => {
            collateral(input.market, withdrawal.collateral_index)?;
            if withdrawal.assets == U256::ZERO {
                return Err(ActionError::ZeroAmount("collateral withdrawal"));
            }
            vec![CollateralWithdrawal {
                collateral_index: U256::from(withdrawal.collateral_index),
                assets: withdrawal.assets,
            }]
        }
        None => vec![],
    };
    if input.repay_assets == U256::ZERO && withdrawals.is_empty() {
        return Err(ActionError::EmptyRepayWithdraw);
    }
    transaction(
        bundles,
        encode_repay_withdraw_collateral_calldata(
            input.market,
            input.repay_assets,
            &input.on_behalf,
            &TokenPermit {
                kind: 0,
                data: vec![],
            },
            &withdrawals,
            &input.on_behalf,
            U256::ZERO,
            &[0; 20],
            input.deadline,
        ),
    )
}

/// Build a direct credit-redemption transaction against Base Midnight.
pub fn redeem(
    market: &Market,
    units: U256,
    on_behalf: Address,
    receiver: Option<Address>,
) -> Result<MidnightTransaction, ActionError> {
    redeem_to(
        crate::BASE_CHAIN_ID,
        crate::BASE_MIDNIGHT,
        market,
        units,
        on_behalf,
        receiver.unwrap_or(on_behalf),
    )
}

/// Build a direct credit-redemption transaction for an explicit Midnight deployment.
pub fn redeem_to(
    chain_id: u64,
    midnight: Address,
    market: &Market,
    units: U256,
    on_behalf: Address,
    receiver: Address,
) -> Result<MidnightTransaction, ActionError> {
    validate_market(market, chain_id, midnight)?;
    validate_address(on_behalf)?;
    validate_address(receiver)?;
    if units == U256::ZERO {
        return Err(ActionError::ZeroAmount("redeem units"));
    }
    transaction(
        midnight,
        encode_withdraw_calldata(market, units, &on_behalf, &receiver),
    )
}

/// Build a direct Midnight operator-authorization transaction.
pub fn set_is_authorized_to(
    midnight: Address,
    authorized: Address,
    is_authorized: bool,
    on_behalf: Address,
) -> Result<MidnightTransaction, ActionError> {
    validate_address(authorized)?;
    validate_address(on_behalf)?;
    transaction(
        midnight,
        encode_set_is_authorized_calldata(&authorized, is_authorized, &on_behalf),
    )
}
