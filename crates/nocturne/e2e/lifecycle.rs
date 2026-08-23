//! Unified maintainer entrypoint for local and guarded Base lifecycle checks.

use std::{path::Path, process::Command};

#[path = "live_base.rs"]
mod live_base;
#[path = "live_maker.rs"]
mod live_maker;

type BoxError = Box<dyn std::error::Error>;

fn run_script(name: &str) -> Result<(), BoxError> {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("e2e").join(name);
    let status = Command::new(path).status()?;
    if !status.success() {
        return Err(format!("{name} exited with {status}").into());
    }
    Ok(())
}

fn usage() -> &'static str {
    "usage: lifecycle <anvil|market-maker|base-taker|base-maker|resume|cleanup>"
}

#[tokio::main]
async fn main() -> Result<(), BoxError> {
    match std::env::args().nth(1).as_deref() {
        Some("anvil") => run_script("run.sh"),
        Some("market-maker") => run_script("mm.sh"),
        Some("base-taker") => live_base::run(false).await,
        Some("base-maker") => live_maker::run(false, false)
            .await
            .map_err(|error| error.to_string().into()),
        Some("resume") => {
            let journal = live_base::support::LifecycleJournal::load()?;
            match journal.scenario.as_str() {
                "base-taker" => live_base::run(true).await,
                "base-maker" => live_maker::run(true, false)
                    .await
                    .map_err(|error| error.to_string().into()),
                scenario => Err(format!("cannot resume unknown scenario {scenario:?}").into()),
            }
        }
        Some("cleanup") => live_maker::run(false, true)
            .await
            .map_err(|error| error.to_string().into()),
        _ => Err(usage().into()),
    }
}
