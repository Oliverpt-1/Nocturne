//! Independent ABI parity and guard tests for complete position transaction builders.

use alloy_primitives::{Address as AlloyAddress, Bytes, U256 as AlloyU256};
use alloy_sol_types::{sol, SolCall};
use nocturne::*;

sol! {
    struct SCollateralParams {
        address token;
        uint256 lltv;
        uint256 liquidationCursor;
        address oracle;
    }
    struct SMarket {
        uint256 chainId;
        address midnight;
        address loanToken;
        SCollateralParams[] collateralParams;
        uint256 maturity;
        uint256 rcfThreshold;
        address enterGate;
        address liquidatorGate;
    }
    struct SOffer {
        SMarket market;
        bool buy;
        address maker;
        uint256 start;
        uint256 expiry;
        uint256 tick;
        bytes32 group;
        address callback;
        bytes callbackData;
        address receiverIfMakerIsSeller;
        address ratifier;
        bool reduceOnly;
        uint128 maxUnits;
        uint128 maxAssets;
        uint256 continuousFeeCap;
    }
    struct STokenPermit { uint8 kind; bytes data; }
    struct SCollateralWithdrawal { uint256 collateralIndex; uint256 assets; }
    struct SCollateralSupply { uint256 collateralIndex; uint256 assets; STokenPermit permit; }
    struct SOfferFill { SOffer offer; bytes ratifierData; uint256 units; }

    function supplyCollateral(SMarket market, uint256 collateralIndex, uint256 assets, address onBehalf);
    function withdraw(SMarket market, uint256 units, address onBehalf, address receiver);
    function setIsAuthorized(address authorized, bool newIsAuthorized, address onBehalf);
    function approve(address spender, uint256 amount);
    function midnightBundlesV1BuyWithAssetsTargetAndWithdrawCollateral(
        uint256 targetBuyerAssets,
        uint256 minUnits,
        address taker,
        bool reduceOnly,
        STokenPermit loanTokenPermit,
        SOfferFill[] offerFills,
        SCollateralWithdrawal[] collateralWithdrawals,
        address collateralReceiver,
        uint256 referralFeePct,
        address referralFeeRecipient,
        uint256 maxContinuousFee,
        uint256 deadline
    );
    function midnightBundlesV1SupplyCollateralAndSellWithAssetsTarget(
        uint256 targetSellerAssets,
        uint256 maxUnits,
        address taker,
        bool reduceOnly,
        address receiver,
        SCollateralSupply[] collateralSupplies,
        SOfferFill[] offerFills,
        uint256 referralFeePct,
        address referralFeeRecipient,
        uint256 maxContinuousFee,
        uint256 deadline
    );
    function midnightBundlesV1RepayAndWithdrawCollateral(
        SMarket market,
        uint256 assets,
        address onBehalf,
        STokenPermit loanTokenPermit,
        SCollateralWithdrawal[] collateralWithdrawals,
        address collateralReceiver,
        uint256 referralFeePct,
        address referralFeeRecipient,
        uint256 deadline
    );
}

fn address(value: Address) -> AlloyAddress {
    AlloyAddress::from_slice(&value)
}

fn u256(value: U256) -> AlloyU256 {
    AlloyU256::from_be_bytes(value.to_be_bytes::<32>())
}

fn market() -> Market {
    MarketBuilder::new(BASE_CHAIN_ID, BASE_MIDNIGHT, [0x22; 20])
        .collateral(
            [0x33; 20],
            U256::from(860_000_000_000_000_000u128),
            U256::from(300_000_000_000_000_000u128),
            [0x44; 20],
        )
        .maturity(1_798_815_600)
        .rcf_threshold(U256::from(3_000_000_000u64))
        .build_checked()
        .unwrap()
}

fn s_market(value: &Market) -> SMarket {
    SMarket {
        chainId: u256(word_to_u256(&value.chain_id)),
        midnight: address(value.midnight),
        loanToken: address(value.loan_token),
        collateralParams: value
            .collateral_params
            .iter()
            .map(|collateral| SCollateralParams {
                token: address(collateral.token),
                lltv: u256(word_to_u256(&collateral.lltv)),
                liquidationCursor: u256(word_to_u256(&collateral.liquidation_cursor)),
                oracle: address(collateral.oracle),
            })
            .collect(),
        maturity: u256(word_to_u256(&value.maturity)),
        rcfThreshold: u256(word_to_u256(&value.rcf_threshold)),
        enterGate: address(value.enter_gate),
        liquidatorGate: address(value.liquidator_gate),
    }
}

fn s_offer(value: &Offer) -> SOffer {
    SOffer {
        market: s_market(&value.market),
        buy: value.buy,
        maker: address(value.maker),
        start: u256(word_to_u256(&value.start)),
        expiry: u256(word_to_u256(&value.expiry)),
        tick: u256(word_to_u256(&value.tick)),
        group: value.group.into(),
        callback: address(value.callback),
        callbackData: Bytes::copy_from_slice(&value.callback_data),
        receiverIfMakerIsSeller: address(value.receiver_if_maker_is_seller),
        ratifier: address(value.ratifier),
        reduceOnly: value.reduce_only,
        maxUnits: value.max_units,
        maxAssets: value.max_assets,
        continuousFeeCap: u256(word_to_u256(&value.continuous_fee_cap)),
    }
}

fn takeable(buy: bool, maker: Address) -> TakeableOffer {
    let builder = OfferBuilder::new(market(), maker)
        .tick(4_892)
        .expiry(1_798_815_500)
        .ratifier(BASE_ECRECOVER_RATIFIER)
        .max_assets(400_000_000);
    let offer = if buy {
        builder.buy()
    } else {
        builder.sell().receiver_if_maker_is_seller(maker)
    }
    .build_checked()
    .unwrap();
    TakeableOffer {
        market_id: market_id(&offer.market),
        units: U256::from(393_193_258u64),
        offer,
        ratifier_data: encode_setter_ratifier_data(&[0x88; 32], 0, &[]),
    }
}

fn s_fill(value: &TakeableOffer) -> SOfferFill {
    SOfferFill {
        offer: s_offer(&value.offer),
        ratifierData: Bytes::copy_from_slice(&value.ratifier_data),
        units: u256(value.units),
    }
}

#[test]
fn direct_actions_match_solidity_abi() {
    let market = market();
    let owner = [0x55; 20];
    let receiver = [0x66; 20];

    let supply = supply_collateral(&market, 0, U256::from(20_000u64), owner).unwrap();
    let expected = supplyCollateralCall {
        market: s_market(&market),
        collateralIndex: AlloyU256::ZERO,
        assets: AlloyU256::from(20_000u64),
        onBehalf: address(owner),
    }
    .abi_encode();
    assert_eq!(supply.to, BASE_MIDNIGHT);
    assert_eq!(supply.data, expected);

    let redeem = redeem(&market, U256::from(12_345u64), owner, Some(receiver)).unwrap();
    let expected = withdrawCall {
        market: s_market(&market),
        units: AlloyU256::from(12_345u64),
        onBehalf: address(owner),
        receiver: address(receiver),
    }
    .abi_encode();
    assert_eq!(redeem.data, expected);

    assert_eq!(
        encode_set_is_authorized_calldata(&BASE_MIDNIGHT_BUNDLES, true, &owner),
        setIsAuthorizedCall {
            authorized: address(BASE_MIDNIGHT_BUNDLES),
            newIsAuthorized: true,
            onBehalf: address(owner),
        }
        .abi_encode()
    );
    assert_eq!(
        encode_erc20_approve_calldata(&BASE_MIDNIGHT_BUNDLES, U256::from(99u64)),
        approveCall {
            spender: address(BASE_MIDNIGHT_BUNDLES),
            amount: AlloyU256::from(99u64),
        }
        .abi_encode()
    );
}

#[test]
fn lend_builder_matches_current_sdk_bundle_shape() {
    let market = market();
    let taker = [0x77; 20];
    let offers = [takeable(false, [0x55; 20])];
    let tx = take_lend(&TakeLend {
        market: &market,
        assets: U256::from(1_000_000u64),
        min_units: U256::from(999_000u64),
        taker,
        offers: &offers,
        deadline: U256::from(1_798_000_000u64),
    })
    .unwrap();
    let expected = midnightBundlesV1BuyWithAssetsTargetAndWithdrawCollateralCall {
        targetBuyerAssets: AlloyU256::from(1_000_000u64),
        minUnits: AlloyU256::from(999_000u64),
        taker: address(taker),
        reduceOnly: false,
        loanTokenPermit: STokenPermit {
            kind: 0,
            data: Bytes::new(),
        },
        offerFills: vec![s_fill(&offers[0])],
        collateralWithdrawals: vec![],
        collateralReceiver: AlloyAddress::ZERO,
        referralFeePct: AlloyU256::ZERO,
        referralFeeRecipient: AlloyAddress::ZERO,
        maxContinuousFee: AlloyU256::MAX,
        deadline: AlloyU256::from(1_798_000_000u64),
    }
    .abi_encode();
    assert_eq!(tx.to, BASE_MIDNIGHT_BUNDLES);
    assert_eq!(tx.data, expected);
}

#[test]
fn collateral_borrow_builder_matches_current_sdk_bundle_shape() {
    let market = market();
    let taker = [0x77; 20];
    let offers = [takeable(true, [0x55; 20])];
    let tx = supply_collateral_take_borrow(
        &TakeBorrow {
            market: &market,
            loan_assets: U256::from(500_000u64),
            max_units: U256::from(501_000u64),
            taker,
            offers: &offers,
            deadline: U256::from(1_798_000_000u64),
        },
        CollateralDeposit {
            collateral_index: 0,
            assets: U256::from(2_000u64),
        },
    )
    .unwrap();
    let expected = midnightBundlesV1SupplyCollateralAndSellWithAssetsTargetCall {
        targetSellerAssets: AlloyU256::from(500_000u64),
        maxUnits: AlloyU256::from(501_000u64),
        taker: address(taker),
        reduceOnly: false,
        receiver: address(taker),
        collateralSupplies: vec![SCollateralSupply {
            collateralIndex: AlloyU256::ZERO,
            assets: AlloyU256::from(2_000u64),
            permit: STokenPermit {
                kind: 0,
                data: Bytes::new(),
            },
        }],
        offerFills: vec![s_fill(&offers[0])],
        referralFeePct: AlloyU256::ZERO,
        referralFeeRecipient: AlloyAddress::ZERO,
        maxContinuousFee: AlloyU256::MAX,
        deadline: AlloyU256::from(1_798_000_000u64),
    }
    .abi_encode();
    assert_eq!(tx.data, expected);
}

#[test]
fn repay_withdraw_builder_matches_current_sdk_bundle_shape() {
    let market = market();
    let owner = [0x77; 20];
    let tx = repay_withdraw_collateral(&RepayWithdraw {
        market: &market,
        repay_assets: U256::from(500_000u64),
        withdraw_collateral: Some(CollateralDeposit {
            collateral_index: 0,
            assets: U256::from(2_000u64),
        }),
        on_behalf: owner,
        deadline: U256::from(1_798_000_000u64),
    })
    .unwrap();
    let expected = midnightBundlesV1RepayAndWithdrawCollateralCall {
        market: s_market(&market),
        assets: AlloyU256::from(500_000u64),
        onBehalf: address(owner),
        loanTokenPermit: STokenPermit {
            kind: 0,
            data: Bytes::new(),
        },
        collateralWithdrawals: vec![SCollateralWithdrawal {
            collateralIndex: AlloyU256::ZERO,
            assets: AlloyU256::from(2_000u64),
        }],
        collateralReceiver: address(owner),
        referralFeePct: AlloyU256::ZERO,
        referralFeeRecipient: AlloyAddress::ZERO,
        deadline: AlloyU256::from(1_798_000_000u64),
    }
    .abi_encode();
    assert_eq!(tx.data, expected);

    let decoded = decode_repay_withdraw_collateral_calldata(&expected).unwrap();
    assert_eq!(
        decoded,
        RepayWithdrawCall {
            market,
            repay_assets: U256::from(500_000u64),
            on_behalf: owner,
            loan_token_permit: TokenPermit {
                kind: 0,
                data: Vec::new(),
            },
            collateral_withdrawals: vec![CollateralWithdrawal {
                collateral_index: U256::ZERO,
                assets: U256::from(2_000u64),
            }],
            collateral_receiver: owner,
            referral_fee_pct: U256::ZERO,
            referral_fee_recipient: [0; 20],
            deadline: U256::from(1_798_000_000u64),
        }
    );
}

#[test]
fn action_guards_reject_unsafe_inputs() {
    let market = market();
    let taker = [0x77; 20];
    let self_offer = [takeable(true, taker)];
    let input = TakeBorrow {
        market: &market,
        loan_assets: U256::from(1u64),
        max_units: U256::from(1u64),
        taker,
        offers: &self_offer,
        deadline: U256::MAX,
    };
    assert_eq!(take_borrow(&input), Err(ActionError::SelfTake(0)));
    assert_eq!(
        supply_collateral(&market, 0, U256::from(1u64), [0; 20]),
        Err(ActionError::ZeroAddress)
    );
    assert!(matches!(
        supply_collateral(&market, 1, U256::from(1u64), taker),
        Err(ActionError::InvalidCollateralIndex { .. })
    ));
    assert_eq!(
        repay_withdraw_collateral(&RepayWithdraw {
            market: &market,
            repay_assets: U256::ZERO,
            withdraw_collateral: None,
            on_behalf: taker,
            deadline: U256::MAX,
        }),
        Err(ActionError::EmptyRepayWithdraw)
    );

    let mut wrong_midnight = market.clone();
    wrong_midnight.midnight = [0x99; 20];
    let offers = [takeable(true, [0x55; 20])];
    assert_eq!(
        take_borrow(&TakeBorrow {
            market: &wrong_midnight,
            loan_assets: U256::from(1u64),
            max_units: U256::from(1u64),
            taker,
            offers: &offers,
            deadline: U256::MAX,
        }),
        Err(ActionError::MidnightMismatch)
    );
}

#[test]
fn requirements_emit_only_missing_transactions() {
    let market = market();
    let owner = [0x77; 20];
    let plan = supply_collateral_take_borrow_requirement_plan(
        &market,
        0,
        U256::from(2_000u64),
        owner,
        BASE_MIDNIGHT_BUNDLES,
    )
    .unwrap();
    assert_eq!(plan.approvals.len(), 1);
    assert!(
        approval_requirement(plan.approvals[0], U256::from(1_999u64))
            .unwrap()
            .is_some()
    );
    assert!(
        approval_requirement(plan.approvals[0], U256::from(2_000u64))
            .unwrap()
            .is_none()
    );
    let authorization = plan.authorization.unwrap();
    assert!(authorization_requirement(authorization, false)
        .unwrap()
        .is_some());
    assert!(authorization_requirement(authorization, true)
        .unwrap()
        .is_none());
}
