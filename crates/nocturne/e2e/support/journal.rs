use std::{fs, path::PathBuf};

use serde::{Deserialize, Serialize};

use super::BoxError;

const JOURNAL_PATH: &str = ".nocturne-e2e-journal.json";

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct JournalEntry {
    pub actor: String,
    pub label: String,
    pub transaction_hash: String,
    pub block_number: u64,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct LifecycleJournal {
    pub scenario: String,
    pub market: Option<String>,
    pub root: Option<String>,
    pub completed: bool,
    pub transactions: Vec<JournalEntry>,
}

impl LifecycleJournal {
    pub fn start(scenario: impl Into<String>) -> Result<Self, BoxError> {
        if Self::path().exists() {
            let previous = Self::load()?;
            if !previous.completed {
                return Err(format!(
                    "unfinished {:?} journal exists; run `lifecycle resume` or `lifecycle cleanup`",
                    previous.scenario
                )
                .into());
            }
        }
        let journal = Self {
            scenario: scenario.into(),
            ..Default::default()
        };
        journal.save()?;
        Ok(journal)
    }

    pub fn load_or_start(scenario: &str) -> Result<Self, BoxError> {
        if !Self::path().exists() {
            return Self::start(scenario);
        }
        let journal = Self::load()?;
        if journal.completed {
            return Self::start(scenario);
        }
        if journal.scenario != scenario {
            return Err(format!(
                "journal belongs to {:?}, not {scenario:?}",
                journal.scenario
            )
            .into());
        }
        Ok(journal)
    }

    pub fn load() -> Result<Self, BoxError> {
        Ok(serde_json::from_slice(&fs::read(Self::path())?)?)
    }

    pub fn load_for(scenario: &str) -> Result<Self, BoxError> {
        let journal = Self::load()?;
        if journal.scenario != scenario {
            return Err(format!(
                "journal belongs to {:?}, not {scenario:?}",
                journal.scenario
            )
            .into());
        }
        if journal.completed {
            return Err(format!("{scenario} is already complete").into());
        }
        Ok(journal)
    }

    pub fn set_context(&mut self, market: String, root: Option<String>) -> Result<(), BoxError> {
        self.market = Some(market);
        self.root = root;
        self.save()
    }

    pub fn record(&mut self, entry: JournalEntry) -> Result<(), BoxError> {
        self.transactions.push(entry);
        self.save()
    }

    pub fn transaction_hash(&self, actor: &str, label: &str) -> Option<&str> {
        self.transactions
            .iter()
            .find(|entry| entry.actor == actor && entry.label == label)
            .map(|entry| entry.transaction_hash.as_str())
    }

    pub fn complete(&mut self) -> Result<(), BoxError> {
        self.completed = true;
        self.save()
    }

    fn path() -> PathBuf {
        PathBuf::from(JOURNAL_PATH)
    }

    fn save(&self) -> Result<(), BoxError> {
        let path = Self::path();
        let temporary = path.with_extension("json.tmp");
        fs::write(&temporary, serde_json::to_vec_pretty(self)?)?;
        fs::rename(temporary, path)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn journal_round_trips_resume_context() {
        let journal = LifecycleJournal {
            scenario: "base-maker".into(),
            market: Some("0xmarket".into()),
            root: Some("0xroot".into()),
            completed: false,
            transactions: vec![JournalEntry {
                actor: "maker".into(),
                label: "publish".into(),
                transaction_hash: "0xtx".into(),
                block_number: 42,
            }],
        };
        let encoded = serde_json::to_vec(&journal).unwrap();
        let decoded = serde_json::from_slice::<LifecycleJournal>(&encoded).unwrap();
        assert_eq!(decoded, journal);
        assert_eq!(decoded.transaction_hash("maker", "publish"), Some("0xtx"));
        assert_eq!(decoded.transaction_hash("taker", "publish"), None);
    }
}
