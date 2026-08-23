//! Live requirement-discovery driver used by `e2e/run.sh`.

use alloy::providers::ProviderBuilder;
use nocturne::*;

const CHAIN_ID: u64 = 31_337;
const OWNER: Address = [
    0xf3, 0x9f, 0xd6, 0xe5, 0x1a, 0xad, 0x88, 0xf6, 0xf4, 0xce, 0x6a, 0xb8, 0x82, 0x72, 0x79, 0xcf,
    0xff, 0xb9, 0x22, 0x66,
];

fn env_address(name: &str) -> Address {
    let value = std::env::var(name).unwrap_or_else(|_| panic!("missing env {name}"));
    let bytes = hex::decode(value.trim_start_matches("0x")).unwrap();
    bytes.try_into().unwrap()
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let market = MarketBuilder::new(CHAIN_ID, env_address("MIDNIGHT"), env_address("LOAN"))
        .collateral(
            env_address("COLLATERAL"),
            U256::from(770_000_000_000_000_000u64),
            U256::from(300_000_000_000_000_000u64),
            env_address("ORACLE"),
        )
        .maturity(4_000_000_000)
        .build_checked()?;
    let bundles = env_address("BUNDLES");
    let plan = supply_collateral_take_borrow_requirement_plan(
        &market,
        0,
        U256::from(2_000u64),
        OWNER,
        bundles,
    )?;
    let provider = ProviderBuilder::new().connect_http("http://127.0.0.1:8545".parse()?);
    let requirements = discover_requirements(&provider, &plan).await?;

    let mut approvals = 0;
    let mut authorizations = 0;
    for requirement in &requirements {
        match requirement {
            ActionRequirement::Approval { .. } => approvals += 1,
            ActionRequirement::Authorization { .. } => authorizations += 1,
        }
    }
    println!("REQUIREMENT_COUNT {}", requirements.len());
    println!("APPROVAL_COUNT {approvals}");
    println!("AUTHORIZATION_COUNT {authorizations}");
    Ok(())
}
