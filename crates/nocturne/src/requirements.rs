//! Approval and Midnight-authorization requirement planning and discovery.

use crate::{
    encode_erc20_approve_calldata, encode_set_is_authorized_calldata, ActionError, Address,
    MidnightTransaction, U256,
};

/// One ERC-20 allowance read needed by an action.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ApprovalRequest {
    pub token: Address,
    pub owner: Address,
    pub spender: Address,
    pub amount: U256,
}

/// One `Midnight.isAuthorized` read needed by an action.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AuthorizationRequest {
    pub midnight: Address,
    pub owner: Address,
    pub authorized: Address,
}

/// Read-only prerequisites for a transaction builder.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RequirementPlan {
    pub chain_id: u64,
    pub approvals: Vec<ApprovalRequest>,
    pub authorization: Option<AuthorizationRequest>,
}

/// A transaction required before the final Midnight action can succeed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ActionRequirement {
    Approval {
        token: Address,
        owner: Address,
        spender: Address,
        required: U256,
        current: U256,
        transaction: MidnightTransaction,
    },
    Authorization {
        owner: Address,
        authorized: Address,
        transaction: MidnightTransaction,
    },
}

/// Errors reading live requirement state through an Alloy provider.
#[cfg(feature = "alloy-wallet")]
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum RequirementError {
    #[error("provider chain id {actual} does not match required chain {expected}")]
    ChainIdMismatch { expected: u64, actual: u64 },
    #[error("requirement RPC call failed: {0}")]
    Rpc(String),
    #[error("requirement RPC result could not be decoded: {0}")]
    Decode(String),
    #[error(transparent)]
    Action(#[from] ActionError),
}

/// Resolve an allowance snapshot into an exact-amount approval transaction when necessary.
pub fn approval_requirement(
    request: ApprovalRequest,
    current_allowance: U256,
) -> Result<Option<ActionRequirement>, ActionError> {
    if request.token == [0; 20] || request.owner == [0; 20] || request.spender == [0; 20] {
        return Err(ActionError::ZeroAddress);
    }
    if request.amount == U256::ZERO || current_allowance >= request.amount {
        return Ok(None);
    }
    let transaction = MidnightTransaction {
        to: request.token,
        value: U256::ZERO,
        data: encode_erc20_approve_calldata(&request.spender, request.amount),
    };
    Ok(Some(ActionRequirement::Approval {
        token: request.token,
        owner: request.owner,
        spender: request.spender,
        required: request.amount,
        current: current_allowance,
        transaction,
    }))
}

/// Resolve an authorization snapshot into a grant transaction when necessary.
pub fn authorization_requirement(
    request: AuthorizationRequest,
    currently_authorized: bool,
) -> Result<Option<ActionRequirement>, ActionError> {
    if request.midnight == [0; 20] || request.owner == [0; 20] || request.authorized == [0; 20] {
        return Err(ActionError::ZeroAddress);
    }
    if currently_authorized {
        return Ok(None);
    }
    let transaction = MidnightTransaction {
        to: request.midnight,
        value: U256::ZERO,
        data: encode_set_is_authorized_calldata(&request.authorized, true, &request.owner),
    };
    Ok(Some(ActionRequirement::Authorization {
        owner: request.owner,
        authorized: request.authorized,
        transaction,
    }))
}

/// Requirements for direct collateral supply by the position owner.
pub fn supply_collateral_requirement_plan(
    market: &crate::Market,
    collateral_index: usize,
    assets: U256,
    owner: Address,
) -> Result<RequirementPlan, ActionError> {
    let collateral = market.collateral_params.get(collateral_index).ok_or(
        ActionError::InvalidCollateralIndex {
            index: collateral_index,
            collaterals: market.collateral_params.len(),
        },
    )?;
    if assets == U256::ZERO {
        return Err(ActionError::ZeroAmount("collateral assets"));
    }
    let chain_id = crate::word_to_u128(&market.chain_id)
        .and_then(|value| u64::try_from(value).ok())
        .ok_or(ActionError::ChainIdMismatch(crate::BASE_CHAIN_ID))?;
    Ok(RequirementPlan {
        chain_id,
        approvals: vec![ApprovalRequest {
            token: collateral.token,
            owner,
            spender: market.midnight,
            amount: assets,
        }],
        authorization: None,
    })
}

/// Requirements for lending through MidnightBundles.
pub fn take_lend_requirement_plan(
    market: &crate::Market,
    assets: U256,
    owner: Address,
    bundles: Address,
) -> Result<RequirementPlan, ActionError> {
    if assets == U256::ZERO {
        return Err(ActionError::ZeroAmount("lend assets"));
    }
    bundle_plan(market, owner, bundles, Some((market.loan_token, assets)))
}

/// Requirements for borrowing through MidnightBundles using existing collateral.
pub fn take_borrow_requirement_plan(
    market: &crate::Market,
    owner: Address,
    bundles: Address,
) -> Result<RequirementPlan, ActionError> {
    bundle_plan(market, owner, bundles, None)
}

/// Requirements for supplying collateral and borrowing atomically through MidnightBundles.
pub fn supply_collateral_take_borrow_requirement_plan(
    market: &crate::Market,
    collateral_index: usize,
    assets: U256,
    owner: Address,
    bundles: Address,
) -> Result<RequirementPlan, ActionError> {
    let token = market
        .collateral_params
        .get(collateral_index)
        .ok_or(ActionError::InvalidCollateralIndex {
            index: collateral_index,
            collaterals: market.collateral_params.len(),
        })?
        .token;
    if assets == U256::ZERO {
        return Err(ActionError::ZeroAmount("collateral assets"));
    }
    bundle_plan(market, owner, bundles, Some((token, assets)))
}

/// Requirements for repaying and optionally withdrawing through MidnightBundles.
pub fn repay_withdraw_requirement_plan(
    market: &crate::Market,
    repay_assets: U256,
    owner: Address,
    bundles: Address,
) -> Result<RequirementPlan, ActionError> {
    bundle_plan(
        market,
        owner,
        bundles,
        (repay_assets != U256::ZERO).then_some((market.loan_token, repay_assets)),
    )
}

fn bundle_plan(
    market: &crate::Market,
    owner: Address,
    bundles: Address,
    approval: Option<(Address, U256)>,
) -> Result<RequirementPlan, ActionError> {
    if owner == [0; 20] || bundles == [0; 20] || market.midnight == [0; 20] {
        return Err(ActionError::ZeroAddress);
    }
    let chain_id = crate::word_to_u128(&market.chain_id)
        .and_then(|value| u64::try_from(value).ok())
        .ok_or(ActionError::ChainIdMismatch(crate::BASE_CHAIN_ID))?;
    Ok(RequirementPlan {
        chain_id,
        approvals: approval
            .into_iter()
            .map(|(token, amount)| ApprovalRequest {
                token,
                owner,
                spender: bundles,
                amount,
            })
            .collect(),
        authorization: Some(AuthorizationRequest {
            midnight: market.midnight,
            owner,
            authorized: bundles,
        }),
    })
}

#[cfg(feature = "alloy-wallet")]
alloy::sol! {
    function allowance(address owner, address spender) external view returns (uint256);
    function isAuthorized(address authorizer, address authorized) external view returns (bool);
}

/// Read all allowances and authorization state in a plan through an Alloy provider.
#[cfg(feature = "alloy-wallet")]
pub async fn discover_requirements<P: alloy::providers::Provider>(
    provider: &P,
    plan: &RequirementPlan,
) -> Result<Vec<ActionRequirement>, RequirementError> {
    use alloy::sol_types::SolCall;

    let actual_chain_id = provider
        .get_chain_id()
        .await
        .map_err(|error| RequirementError::Rpc(error.to_string()))?;
    if actual_chain_id != plan.chain_id {
        return Err(RequirementError::ChainIdMismatch {
            expected: plan.chain_id,
            actual: actual_chain_id,
        });
    }

    let mut requirements = Vec::new();
    for request in &plan.approvals {
        let call = allowanceCall {
            owner: alloy::primitives::Address::from_slice(&request.owner),
            spender: alloy::primitives::Address::from_slice(&request.spender),
        };
        let output = provider
            .call(
                alloy::rpc::types::TransactionRequest::default()
                    .to(alloy::primitives::Address::from_slice(&request.token))
                    .input(alloy::primitives::Bytes::from(call.abi_encode()).into()),
            )
            .await
            .map_err(|error| RequirementError::Rpc(error.to_string()))?;
        let allowance = allowanceCall::abi_decode_returns(&output)
            .map_err(|error| RequirementError::Decode(error.to_string()))?;
        let allowance = U256::from_be_bytes(allowance.to_be_bytes::<32>());
        if let Some(requirement) = approval_requirement(*request, allowance)? {
            requirements.push(requirement);
        }
    }

    if let Some(request) = plan.authorization {
        let call = isAuthorizedCall {
            authorizer: alloy::primitives::Address::from_slice(&request.owner),
            authorized: alloy::primitives::Address::from_slice(&request.authorized),
        };
        let output = provider
            .call(
                alloy::rpc::types::TransactionRequest::default()
                    .to(alloy::primitives::Address::from_slice(&request.midnight))
                    .input(alloy::primitives::Bytes::from(call.abi_encode()).into()),
            )
            .await
            .map_err(|error| RequirementError::Rpc(error.to_string()))?;
        let authorized = isAuthorizedCall::abi_decode_returns(&output)
            .map_err(|error| RequirementError::Decode(error.to_string()))?;
        if let Some(requirement) = authorization_requirement(request, authorized)? {
            requirements.push(requirement);
        }
    }

    Ok(requirements)
}
