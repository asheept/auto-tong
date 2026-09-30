use serde::Serialize;
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT_JOB_ID: AtomicU64 = AtomicU64::new(1);

#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ImportPhase {
    Preparing,
    Downloading,
    Extracting,
    Java,
    Recording,
    Deferred,
    RecoveryRequired,
    Completed,
    Failed,
    Cancelled,
}

#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorCategory {
    Storage,
    Launcher,
    Java,
    Unknown,
}

#[derive(Clone, Debug, Serialize)]
pub struct ImportProgress<'a> {
    pub job_id: u64,
    pub source_id: &'a str,
    pub file_name: &'a str,
    pub phase: ImportPhase,
    pub percent: u32,
    pub status: &'a str,
    pub terminal: bool,
    pub error_category: Option<ErrorCategory>,
    pub error: Option<&'a str>,
    pub retryable: bool,
}

pub fn next_job_id() -> u64 {
    NEXT_JOB_ID.fetch_add(1, Ordering::Relaxed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn progress_contract_matches_frontend_fixture() {
        let event = ImportProgress {
            job_id: 7,
            source_id: "C:/packs|nested/pack.zip",
            file_name: "pack.zip",
            phase: ImportPhase::Completed,
            percent: 100,
            status: "완료",
            terminal: true,
            error_category: None,
            error: None,
            retryable: false,
        };
        let expected: serde_json::Value =
            serde_json::from_str(include_str!("../../src/fixtures/import-progress.json")).unwrap();
        assert_eq!(serde_json::to_value(event).unwrap(), expected);
    }
}
