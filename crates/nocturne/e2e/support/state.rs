use alloy::{
    primitives::{Address as AlloyAddress, Bytes, B256, U256 as AlloyU256},
    providers::Provider,
    rpc::types::TransactionRequest,
    sol,
    sol_types::SolCall,
};
use nocturne::{Address, Word, U256};

use super::{alloy_address, BoxError};

sol! {
    function balanceOf(address owner) external view returns (uint256);
    function debt(bytes32 id, address owner) external view returns (uint128);
    function collateral(bytes32 id, address owner, uint256 index) external view returns (uint128);
    function credit(bytes32 id, address owner) external view returns (uint128);
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LifecycleSnapshot {
    pub debt: U256,
    pub collateral: U256,
    pub credit: U256,
    pub loan_balance: U256,
    pub collateral_balance: U256,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExpectedAmount {
    Ignore,
    Zero,
    NonZero,
    Exact(U256),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LifecycleInvariants {
    pub debt: ExpectedAmount,
    pub collateral: ExpectedAmount,
    pub credit: ExpectedAmount,
    pub loan_balance: ExpectedAmount,
    pub collateral_balance: ExpectedAmount,
}

impl LifecycleInvariants {
    pub fn clean(collateral_balance: U256) -> Self {
        Self {
            debt: ExpectedAmount::Zero,
            collateral: ExpectedAmount::Zero,
            credit: ExpectedAmount::Zero,
            loan_balance: ExpectedAmount::Ignore,
            collateral_balance: ExpectedAmount::Exact(collateral_balance),
        }
    }

    pub fn borrowed(collateral: U256, maker_flow: bool) -> Self {
        Self {
            debt: ExpectedAmount::NonZero,
            collateral: ExpectedAmount::Exact(collateral),
            credit: if maker_flow {
                ExpectedAmount::NonZero
            } else {
                ExpectedAmount::Ignore
            },
            loan_balance: ExpectedAmount::Ignore,
            collateral_balance: ExpectedAmount::Ignore,
        }
    }

    pub fn assert(self, actual: LifecycleSnapshot) -> Result<(), InvariantError> {
        check("debt", self.debt, actual.debt)?;
        check("collateral", self.collateral, actual.collateral)?;
        check("credit", self.credit, actual.credit)?;
        check("loan_balance", self.loan_balance, actual.loan_balance)?;
        check(
            "collateral_balance",
            self.collateral_balance,
            actual.collateral_balance,
        )?;
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[error("lifecycle invariant {field} expected {expected:?}, got {actual}")]
pub struct InvariantError {
    pub field: &'static str,
    pub expected: ExpectedAmount,
    pub actual: U256,
}

fn check(
    field: &'static str,
    expected: ExpectedAmount,
    actual: U256,
) -> Result<(), InvariantError> {
    let matches = match expected {
        ExpectedAmount::Ignore => true,
        ExpectedAmount::Zero => actual == U256::ZERO,
        ExpectedAmount::NonZero => actual != U256::ZERO,
        ExpectedAmount::Exact(value) => actual == value,
    };
    if matches {
        Ok(())
    } else {
        Err(InvariantError {
            field,
            expected,
            actual,
        })
    }
}

fn sdk_u256(value: AlloyU256) -> U256 {
    U256::from_be_bytes(value.to_be_bytes::<32>())
}

async fn raw_call<P: Provider>(
    provider: &P,
    to: Address,
    data: Vec<u8>,
) -> Result<Bytes, BoxError> {
    Ok(provider
        .call(
            TransactionRequest::default()
                .to(alloy_address(to))
                .input(Bytes::from(data).into()),
        )
        .await?)
}

pub async fn token_balance<P: Provider>(
    provider: &P,
    token: Address,
    owner: AlloyAddress,
) -> Result<U256, BoxError> {
    let output = raw_call(provider, token, balanceOfCall { owner }.abi_encode()).await?;
    Ok(sdk_u256(balanceOfCall::abi_decode_returns(&output)?))
}

pub async fn snapshot<P: Provider>(
    provider: &P,
    midnight: Address,
    market: Word,
    position_owner: AlloyAddress,
    credit_owner: AlloyAddress,
    loan_token: Address,
    collateral_token: Address,
) -> Result<LifecycleSnapshot, BoxError> {
    let id = B256::from(market);
    let debt = debtCall::abi_decode_returns(
        &raw_call(
            provider,
            midnight,
            debtCall {
                id,
                owner: position_owner,
            }
            .abi_encode(),
        )
        .await?,
    )?;
    let collateral = collateralCall::abi_decode_returns(
        &raw_call(
            provider,
            midnight,
            collateralCall {
                id,
                owner: position_owner,
                index: AlloyU256::ZERO,
            }
            .abi_encode(),
        )
        .await?,
    )?;
    let credit = creditCall::abi_decode_returns(
        &raw_call(
            provider,
            midnight,
            creditCall {
                id,
                owner: credit_owner,
            }
            .abi_encode(),
        )
        .await?,
    )?;
    Ok(LifecycleSnapshot {
        debt: U256::from(debt),
        collateral: U256::from(collateral),
        credit: U256::from(credit),
        loan_balance: token_balance(provider, loan_token, position_owner).await?,
        collateral_balance: token_balance(provider, collateral_token, position_owner).await?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state(
        debt: u64,
        collateral: u64,
        credit: u64,
        collateral_balance: u64,
    ) -> LifecycleSnapshot {
        LifecycleSnapshot {
            debt: U256::from(debt),
            collateral: U256::from(collateral),
            credit: U256::from(credit),
            loan_balance: U256::ZERO,
            collateral_balance: U256::from(collateral_balance),
        }
    }

    #[test]
    fn clean_state_bundles_all_reconciliation_invariants() {
        LifecycleInvariants::clean(U256::from(1_000u64))
            .assert(state(0, 0, 0, 1_000))
            .unwrap();
        let error = LifecycleInvariants::clean(U256::from(1_000u64))
            .assert(state(1, 0, 0, 1_000))
            .unwrap_err();
        assert_eq!(error.field, "debt");
    }

    #[test]
    fn borrowed_state_supports_taker_and_maker_scenarios() {
        LifecycleInvariants::borrowed(U256::from(800u64), false)
            .assert(state(100, 800, 0, 200))
            .unwrap();
        LifecycleInvariants::borrowed(U256::from(800u64), true)
            .assert(state(100, 800, 100, 200))
            .unwrap();
        assert!(LifecycleInvariants::borrowed(U256::from(800u64), true)
            .assert(state(100, 800, 0, 200))
            .is_err());
    }
}
