use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::io::Write;
use std::path::PathBuf;
use std::sync::Mutex;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HistoryItem {
    pub path: String,
    pub status: String,
    pub timestamp: u64,
    pub installed: bool,
}

const SCHEMA_VERSION: u32 = 2;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InstalledState {
    pub modified_secs: u64,
    pub fingerprint: Option<crate::source::Fingerprint>,
    pub commit_token: String,
    #[serde(default)]
    pub installed_at_secs: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AttemptState {
    pub modified_secs: u64,
    pub fingerprint: Option<crate::source::Fingerprint>,
    pub status: String,
    #[serde(default)]
    pub attempted_at_secs: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SourceRecord {
    pub source_root: Option<String>,
    pub relative_path: String,
    pub instance_id: Option<String>,
    pub installed: Option<InstalledState>,
    pub last_attempt: Option<AttemptState>,
    #[serde(default)]
    pub retry_requested: bool,
    #[serde(default)]
    pub migration_pending: bool,
    #[serde(default)]
    pub auto_retry: Option<AutoRetryState>,
    #[serde(default)]
    pub launcher_warning: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AutoRetryState {
    pub attempts: u8,
    pub next_retry_secs: u64,
}

const MAX_AUTO_ATTEMPTS: u8 = 3;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ProcessedFiles {
    #[serde(default)]
    pub schema_version: u32,
    #[serde(default)]
    pub records: HashMap<String, SourceRecord>,
    #[serde(default, skip_serializing)]
    pub imported: HashMap<String, u64>,
    #[serde(default, skip_serializing)]
    pub failed: HashMap<String, u64>,
    #[serde(default, skip_serializing)]
    pub retry_requested: HashSet<String>,
    #[serde(default, skip_serializing)]
    pub commit_tokens: HashMap<String, String>,
    #[serde(default)]
    pub source_map: HashMap<String, SourceMapping>,
    #[serde(default, skip_serializing)]
    pub fingerprints: HashMap<String, crate::source::Fingerprint>,
    #[serde(default, skip_serializing)]
    pub failed_fingerprints: HashMap<String, crate::source::Fingerprint>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SourceMapping {
    pub root: String,
    pub relative_path: String,
    pub instance_id: String,
    pub history_key: String,
}

pub struct Tracker {
    data: Mutex<ProcessedFiles>,
    /// Runtime-only fallback when a failed/cancelled attempt cannot be made durable.
    /// The latch is intentionally lost when the process restarts.
    nonpersisted_failures: Mutex<NonpersistedFailures>,
    path: PathBuf,
    #[cfg(test)]
    fail_writes: std::sync::atomic::AtomicBool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ContentIdentity {
    size: u64,
    sha256: String,
}

impl From<&crate::source::Fingerprint> for ContentIdentity {
    fn from(fingerprint: &crate::source::Fingerprint) -> Self {
        Self {
            size: fingerprint.size,
            sha256: fingerprint.sha256.clone(),
        }
    }
}

#[derive(Default)]
struct NonpersistedFailures {
    entries: HashMap<String, ContentIdentity>,
}

impl NonpersistedFailures {
    fn insert(&mut self, key: &str, fingerprint: &crate::source::Fingerprint) {
        self.entries
            .insert(key.to_string(), ContentIdentity::from(fingerprint));
    }

    fn blocks(&mut self, key: &str, fingerprint: &crate::source::Fingerprint) -> bool {
        let current = ContentIdentity::from(fingerprint);
        match self.entries.get(key) {
            Some(saved) if saved == &current => true,
            Some(_) => {
                self.remove(key);
                false
            }
            None => false,
        }
    }

    fn remove(&mut self, key: &str) {
        self.entries.remove(key);
    }
}

fn same_content(left: &crate::source::Fingerprint, right: &crate::source::Fingerprint) -> bool {
    left.size == right.size && left.sha256 == right.sha256
}

impl ProcessedFiles {
    fn empty_v2() -> Self {
        Self {
            schema_version: SCHEMA_VERSION,
            ..Self::default()
        }
    }

    fn rebuild_maps(&mut self) {
        self.imported.clear();
        self.failed.clear();
        self.retry_requested.clear();
        self.commit_tokens.clear();
        self.fingerprints.clear();
        self.failed_fingerprints.clear();
        for (key, record) in &self.records {
            if let Some(installed) = &record.installed {
                self.imported.insert(key.clone(), installed.modified_secs);
                self.commit_tokens
                    .insert(key.clone(), installed.commit_token.clone());
                if let Some(fingerprint) = &installed.fingerprint {
                    self.fingerprints.insert(key.clone(), fingerprint.clone());
                }
            }
            if let Some(attempt) = &record.last_attempt {
                if attempt.status == "failed" || attempt.status == "cancelled" {
                    self.failed.insert(key.clone(), attempt.modified_secs);
                    if let Some(fingerprint) = &attempt.fingerprint {
                        self.failed_fingerprints
                            .insert(key.clone(), fingerprint.clone());
                    }
                }
            }
            if record.retry_requested {
                self.retry_requested.insert(key.clone());
            }
        }
    }

    fn reconcile_records(&mut self) {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        self.reconcile_records_at(now);
    }

    fn reconcile_records_at(&mut self, now: u64) {
        self.schema_version = SCHEMA_VERSION;
        let mut keys = HashSet::new();
        keys.extend(self.imported.keys().cloned());
        keys.extend(self.failed.keys().cloned());
        keys.extend(self.retry_requested.iter().cloned());
        keys.extend(
            self.source_map
                .values()
                .map(|mapping| mapping.history_key.clone()),
        );
        for key in keys {
            let record = self
                .records
                .entry(key.clone())
                .or_insert_with(|| SourceRecord {
                    relative_path: key.clone(),
                    ..SourceRecord::default()
                });
            if let Some(mapping) = self
                .source_map
                .values()
                .find(|mapping| mapping.history_key == key)
            {
                record.source_root = Some(mapping.root.clone());
                record.relative_path = mapping.relative_path.clone();
                record.instance_id = Some(mapping.instance_id.clone());
            }
            record.installed = self.imported.get(&key).map(|&modified_secs| {
                let fingerprint = self.fingerprints.get(&key).cloned();
                let commit_token = self.commit_tokens.get(&key).cloned().unwrap_or_default();
                let installed_at_secs = record
                    .installed
                    .as_ref()
                    .filter(|old| {
                        old.modified_secs == modified_secs
                            && old.fingerprint == fingerprint
                            && old.commit_token == commit_token
                    })
                    .map_or(now, |old| old.installed_at_secs);
                InstalledState {
                    modified_secs,
                    fingerprint,
                    commit_token,
                    installed_at_secs,
                }
            });
            let attempt = if let Some(&modified_secs) = self.failed.get(&key) {
                Some((
                    modified_secs,
                    self.failed_fingerprints.get(&key).cloned(),
                    if record.last_attempt.as_ref().is_some_and(|old| {
                        old.status == "cancelled"
                            && old.modified_secs == modified_secs
                            && old.fingerprint == self.failed_fingerprints.get(&key).cloned()
                    }) {
                        "cancelled"
                    } else {
                        "failed"
                    },
                ))
            } else {
                self.imported.get(&key).map(|&modified_secs| {
                    (modified_secs, self.fingerprints.get(&key).cloned(), "ok")
                })
            };
            record.last_attempt = attempt.map(|(modified_secs, fingerprint, status)| {
                let attempted_at_secs = record
                    .last_attempt
                    .as_ref()
                    .filter(|old| {
                        old.modified_secs == modified_secs
                            && old.fingerprint == fingerprint
                            && old.status == status
                    })
                    .map_or(now, |old| old.attempted_at_secs);
                AttemptState {
                    modified_secs,
                    fingerprint,
                    status: status.into(),
                    attempted_at_secs,
                }
            });
            record.retry_requested = self.retry_requested.contains(&key);
        }
    }
}

impl Tracker {
    pub fn new() -> Result<Self, String> {
        let path = crate::config::config_dir().join("processed.json");
        Self::open_at(path)
    }

    fn open_at(path: PathBuf) -> Result<Self, String> {
        if !path.exists() {
            return Ok(Self {
                data: Mutex::new(ProcessedFiles::empty_v2()),
                nonpersisted_failures: Mutex::new(NonpersistedFailures::default()),
                path,
                #[cfg(test)]
                fail_writes: std::sync::atomic::AtomicBool::new(false),
            });
        }
        let raw = fs::read(&path).map_err(|e| format!("이력 파일 읽기 실패: {e}"))?;
        let mut data: ProcessedFiles = match serde_json::from_slice(&raw) {
            Ok(data) => data,
            Err(error) => {
                let backup = path.with_file_name("processed.backup.json");
                let backup_raw = fs::read(&backup).map_err(|_| {
                    format!("이력 JSON 손상: {error}. 원본을 보존하고 자동 가져오기를 중단합니다")
                })?;
                let restored: ProcessedFiles = serde_json::from_slice(&backup_raw)
                    .map_err(|_| format!("이력과 백업 JSON이 모두 손상되었습니다. 원본을 보존하고 자동 가져오기를 중단합니다: {error}"))?;
                let parent = path.parent().ok_or("이력 폴더 없음")?;
                let corrupt = tempfile::Builder::new()
                    .prefix("processed.corrupt-")
                    .suffix(".json")
                    .tempfile_in(parent)
                    .map_err(|e| format!("손상 이력 보존 실패: {e}"))?;
                fs::write(corrupt.path(), &raw).map_err(|e| format!("손상 이력 보존 실패: {e}"))?;
                corrupt
                    .keep()
                    .map_err(|e| format!("손상 이력 보존 실패: {e}"))?;
                Self::persist_json(&path, &backup_raw)?;
                restored
            }
        };
        if data.schema_version > SCHEMA_VERSION {
            return Err(format!(
                "지원하지 않는 이력 버전 {}. 자동 가져오기를 중단합니다",
                data.schema_version
            ));
        }
        let migrating = data.schema_version < SCHEMA_VERSION;
        if migrating {
            let backup = path.with_file_name("processed.v1.backup.json");
            if !backup.exists() {
                let original =
                    fs::read(&path).map_err(|e| format!("v1 이력 백업 읽기 실패: {e}"))?;
                Self::persist_json(&backup, &original)?;
            }
            data.reconcile_records();
        } else {
            data.rebuild_maps();
        }
        let tracker = Self {
            data: Mutex::new(data),
            nonpersisted_failures: Mutex::new(NonpersistedFailures::default()),
            path,
            #[cfg(test)]
            fail_writes: std::sync::atomic::AtomicBool::new(false),
        };
        if migrating {
            let json =
                serde_json::to_string_pretty(&*tracker.data.lock().map_err(|e| e.to_string())?)
                    .map_err(|e| e.to_string())?;
            tracker.atomic_write(&json)?;
        }
        Ok(tracker)
    }

    #[cfg(test)]
    pub fn needs_import(&self, relative_path: &str, modified_secs: u64) -> bool {
        let data = match self.data.lock() {
            Ok(d) => d,
            Err(e) => {
                log::error!("Tracker 락 획득 실패: {}", e);
                return true; // 안전한 방향: import 시도
            }
        };
        if data.retry_requested.contains(relative_path) {
            return true;
        }
        // 성공 이력 확인
        if let Some(&saved_time) = data.imported.get(relative_path) {
            if saved_time == modified_secs {
                return false;
            }
        }
        // 실패 이력 확인 (같은 수정시간이면 건너뜀, 파일이 변경되면 재시도)
        if let Some(&failed_time) = data.failed.get(relative_path) {
            if failed_time == modified_secs {
                return false;
            }
        }
        true
    }

    pub fn needs_import_fingerprint(
        &self,
        key: &str,
        fingerprint: &crate::source::Fingerprint,
    ) -> bool {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        self.needs_import_fingerprint_at(key, fingerprint, now)
    }

    fn needs_import_fingerprint_at(
        &self,
        key: &str,
        fingerprint: &crate::source::Fingerprint,
        now: u64,
    ) -> bool {
        let modified_secs = crate::source::observed_seconds(fingerprint);
        let mut guard = match self.nonpersisted_failures.lock() {
            Ok(guard) => guard,
            Err(error) => {
                log::error!("Tracker 비영속 실패 보호 락 획득 실패: {}", error);
                return false;
            }
        };
        if guard.blocks(key, fingerprint) {
            return false;
        }
        let data = match self.data.lock() {
            Ok(data) => data,
            Err(error) => {
                log::error!("Tracker 락 획득 실패: {}", error);
                return false;
            }
        };
        if data.retry_requested.contains(key) {
            return true;
        }
        let failed_match = data.failed_fingerprints.get(key).map_or_else(
            || data.failed.get(key).copied() == Some(modified_secs),
            |failed| same_content(failed, fingerprint),
        );
        if failed_match {
            return data
                .records
                .get(key)
                .and_then(|record| record.auto_retry.as_ref())
                .is_some_and(|retry| {
                    retry.attempts < MAX_AUTO_ATTEMPTS && now >= retry.next_retry_secs
                });
        }
        let installed_match = data.fingerprints.get(key).map_or_else(
            || data.imported.get(key).copied() == Some(modified_secs),
            |installed| installed == fingerprint,
        );
        if installed_match {
            return false;
        }
        true
    }

    pub fn resolve_source(
        &self,
        identity: &crate::source::SourceIdentity,
        legacy_stem: &str,
        instances_dir: &std::path::Path,
    ) -> Result<SourceMapping, String> {
        if let Ok(data) = self.data.lock() {
            if let Some(mapping) = data.source_map.get(&identity.id) {
                return Ok(mapping.clone());
            }
        }
        self.update(|data| {
            if let Some(mapping) = data.source_map.get(&identity.id) {
                return Ok(mapping.clone());
            }
            let legacy_key = data.imported.keys().chain(data.failed.keys())
                .find(|key| key.replace('\\', "/").to_lowercase() == identity.relative_path)
                .cloned();
            let claimed = data.source_map.values().any(|mapping| mapping.instance_id.eq_ignore_ascii_case(legacy_stem));
            let mut legacy_names = std::collections::HashSet::new();
            for key in data.imported.keys().chain(data.failed.keys()) {
                if std::path::Path::new(key).file_stem().is_some_and(|stem| stem.to_string_lossy().eq_ignore_ascii_case(legacy_stem)) {
                    legacy_names.insert(key.to_lowercase());
                }
            }
            let (instance_id, history_key) = if let Some(legacy_key) = legacy_key.filter(|_| !claimed) {
                if legacy_names.len() != 1 || !instances_dir.join(legacy_stem).is_dir() {
                    if let Some(record) = data.records.get_mut(&legacy_key) {
                        record.migration_pending = true;
                    }
                    return Err(format!("이전 이력의 인스턴스 매핑이 모호합니다: {}. 기존 인스턴스를 확인한 뒤 재가져오기를 지정하세요", identity.relative_path));
                }
                (legacy_stem.to_string(), legacy_key)
            } else {
                (identity.suggested_instance_id.clone(), identity.id.clone())
            };
            let mapping = SourceMapping { root: identity.root.clone(),
                relative_path: identity.relative_path.clone(), instance_id, history_key };
            data.source_map.insert(identity.id.clone(), mapping.clone());
            if let Some(record) = data.records.get_mut(&mapping.history_key) {
                record.migration_pending = false;
            }
            Ok(mapping)
        })?
    }

    #[cfg(test)]
    pub fn is_imported(&self, relative_path: &str, modified_secs: u64) -> bool {
        self.data
            .lock()
            .is_ok_and(|data| data.imported.get(relative_path).copied() == Some(modified_secs))
    }

    #[cfg(test)]
    pub fn mark_processed(&self, relative_path: &str, modified_secs: u64) -> Result<(), String> {
        self.mark_processed_with_token(relative_path, modified_secs, "test-token")
    }

    #[cfg(test)]
    pub fn mark_processed_with_token(
        &self,
        relative_path: &str,
        modified_secs: u64,
        token: &str,
    ) -> Result<(), String> {
        self.mark_processed_with_fingerprint(relative_path, modified_secs, token, None)
    }

    pub fn mark_processed_with_fingerprint(
        &self,
        relative_path: &str,
        modified_secs: u64,
        token: &str,
        fingerprint: Option<&crate::source::Fingerprint>,
    ) -> Result<(), String> {
        self.update_with_guard(relative_path, None, |data| {
            data.imported
                .insert(relative_path.to_string(), modified_secs);
            data.failed.remove(relative_path);
            data.retry_requested.remove(relative_path);
            data.commit_tokens
                .insert(relative_path.to_string(), token.to_string());
            if let Some(fingerprint) = fingerprint {
                data.fingerprints
                    .insert(relative_path.to_string(), fingerprint.clone());
            }
            data.failed_fingerprints.remove(relative_path);
            if let Some(record) = data.records.get_mut(relative_path) {
                record.auto_retry = None;
                record.last_attempt = None;
                record.launcher_warning = None;
            }
        })
    }

    pub fn is_committed(&self, relative_path: &str, modified_secs: u64, token: &str) -> bool {
        self.data.lock().is_ok_and(|data| {
            data.imported.get(relative_path).copied() == Some(modified_secs)
                && data
                    .commit_tokens
                    .get(relative_path)
                    .is_some_and(|saved| saved == token)
        })
    }

    #[cfg(test)]
    pub fn mark_failed(&self, relative_path: &str, modified_secs: u64) -> Result<(), String> {
        self.mark_failed_with_fingerprint(relative_path, modified_secs, None)
    }

    #[cfg(test)]
    pub fn mark_failed_with_fingerprint(
        &self,
        relative_path: &str,
        modified_secs: u64,
        fingerprint: Option<&crate::source::Fingerprint>,
    ) -> Result<(), String> {
        self.mark_failed_retryable_at(relative_path, modified_secs, fingerprint, false, 0)
    }

    pub fn mark_failed_retryable(
        &self,
        relative_path: &str,
        modified_secs: u64,
        fingerprint: &crate::source::Fingerprint,
    ) -> Result<(), String> {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        self.mark_failed_retryable_at(relative_path, modified_secs, Some(fingerprint), true, now)
    }

    fn mark_failed_retryable_at(
        &self,
        relative_path: &str,
        modified_secs: u64,
        fingerprint: Option<&crate::source::Fingerprint>,
        retryable: bool,
        now: u64,
    ) -> Result<(), String> {
        self.update_with_guard(relative_path, fingerprint, |data| {
            let same_failure = fingerprint.is_some_and(|current| {
                data.failed_fingerprints
                    .get(relative_path)
                    .is_some_and(|failed| same_content(failed, current))
            });
            let old_attempts = if same_failure {
                data.records
                    .get(relative_path)
                    .and_then(|record| record.auto_retry.as_ref())
                    .map_or(0, |retry| retry.attempts)
            } else {
                0
            };
            data.failed.insert(relative_path.to_string(), modified_secs);
            data.retry_requested.remove(relative_path);
            if let Some(fingerprint) = fingerprint {
                data.failed_fingerprints
                    .insert(relative_path.to_string(), fingerprint.clone());
            }
            let record = data
                .records
                .entry(relative_path.to_string())
                .or_insert_with(|| SourceRecord {
                    relative_path: relative_path.to_string(),
                    ..SourceRecord::default()
                });
            record.auto_retry = if retryable {
                let attempts = old_attempts.saturating_add(1);
                let delay = match attempts {
                    1 => 30,
                    2 => 120,
                    _ => 0,
                };
                Some(AutoRetryState {
                    attempts,
                    next_retry_secs: now.saturating_add(delay),
                })
            } else {
                None
            };
            record.last_attempt = None;
        })
    }

    pub fn mark_cancelled(
        &self,
        relative_path: &str,
        fingerprint: &crate::source::Fingerprint,
    ) -> Result<(), String> {
        let modified_secs = crate::source::observed_seconds(fingerprint);
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        self.update_with_guard(relative_path, Some(fingerprint), |data| {
            data.failed.insert(relative_path.to_string(), modified_secs);
            data.failed_fingerprints
                .insert(relative_path.to_string(), fingerprint.clone());
            data.retry_requested.remove(relative_path);
            let record = data
                .records
                .entry(relative_path.to_string())
                .or_insert_with(|| SourceRecord {
                    relative_path: relative_path.to_string(),
                    ..SourceRecord::default()
                });
            record.auto_retry = None;
            record.last_attempt = Some(AttemptState {
                modified_secs,
                fingerprint: Some(fingerprint.clone()),
                status: "cancelled".into(),
                attempted_at_secs: now,
            });
        })
    }

    pub fn mark_launcher_restart_failed(
        &self,
        relative_path: &str,
        error: &str,
    ) -> Result<(), String> {
        self.update(|data| {
            if let Some(record) = data.records.get_mut(relative_path) {
                record.launcher_warning = Some(error.to_string());
            }
        })
    }

    /// 성공/실패 상태를 포함한 이력 반환
    pub fn get_history_with_status(&self) -> Vec<HistoryItem> {
        let data = match self.data.lock() {
            Ok(d) => d,
            Err(e) => {
                log::error!("Tracker 락 획득 실패: {}", e);
                return vec![];
            }
        };
        let mut items: Vec<HistoryItem> = Vec::new();
        for record in data.records.values() {
            if record.installed.is_none() && record.last_attempt.is_none() {
                continue;
            }
            let failed = record
                .last_attempt
                .as_ref()
                .is_some_and(|attempt| attempt.status == "failed");
            let cancelled = record
                .last_attempt
                .as_ref()
                .is_some_and(|attempt| attempt.status == "cancelled");
            items.push(HistoryItem {
                path: record.relative_path.clone(),
                status: if record.migration_pending {
                    "migration_pending"
                } else if cancelled {
                    "cancelled"
                } else if failed && record.installed.is_some() {
                    "retry_failed"
                } else if failed {
                    "failed"
                } else if record.launcher_warning.is_some() && record.installed.is_some() {
                    "launcher_warning"
                } else {
                    "ok"
                }
                .to_string(),
                timestamp: record
                    .last_attempt
                    .as_ref()
                    .map(|attempt| attempt.attempted_at_secs)
                    .or_else(|| {
                        record
                            .installed
                            .as_ref()
                            .map(|installed| installed.installed_at_secs)
                    })
                    .unwrap_or(0),
                installed: record.installed.is_some(),
            });
        }
        items.sort_by_key(|item| item.timestamp);
        items
    }

    /// 재가져오기 의도를 저장하며 기존 성공 설치 기록은 유지한다.
    pub fn request_reimport(&self, relative_path: &str) -> Result<(), String> {
        self.update_with_guard(relative_path, None, |data| {
            data.retry_requested.insert(relative_path.to_string());
            if let Some(record) = data.records.get_mut(relative_path) {
                record.auto_retry = None;
            }
        })
    }

    fn update_with_guard<T>(
        &self,
        relative_path: &str,
        failure_fingerprint: Option<&crate::source::Fingerprint>,
        change: impl FnOnce(&mut ProcessedFiles) -> T,
    ) -> Result<T, String> {
        let mut failures = self
            .nonpersisted_failures
            .lock()
            .map_err(|error| format!("Tracker 비영속 실패 보호 락 획득 실패: {error}"))?;
        let result = self.update(change);
        match (&result, failure_fingerprint) {
            (Ok(_), _) => failures.remove(relative_path),
            (Err(_), Some(fingerprint)) => failures.insert(relative_path, fingerprint),
            (Err(_), None) => {}
        }
        result
    }

    fn update<T>(&self, change: impl FnOnce(&mut ProcessedFiles) -> T) -> Result<T, String> {
        let mut data = self
            .data
            .lock()
            .map_err(|e| format!("Tracker 락 획득 실패: {}", e))?;
        let mut next = data.clone();
        let result = change(&mut next);
        next.reconcile_records();
        let json = serde_json::to_string_pretty(&next).map_err(|e| e.to_string())?;
        self.atomic_write(&json)?;
        *data = next;
        Ok(result)
    }

    /// 같은 디렉터리의 고유 임시 파일을 완전히 쓴 후 교체한다.
    fn atomic_write(&self, content: &str) -> Result<(), String> {
        #[cfg(test)]
        if self.fail_writes.load(std::sync::atomic::Ordering::SeqCst) {
            return Err("테스트 이력 저장 실패".into());
        }
        if self.path.is_file() {
            let old = fs::read(&self.path).map_err(|e| format!("이전 이력 백업 읽기 실패: {e}"))?;
            let backup = self.path.with_file_name("processed.backup.json");
            Self::persist_json(&backup, &old)?;
        }
        Self::persist_json(&self.path, content.as_bytes())
    }

    fn persist_json(path: &std::path::Path, content: &[u8]) -> Result<(), String> {
        let parent = path.parent().ok_or("이력 디렉터리 없음")?;
        let mut tmp = tempfile::Builder::new()
            .prefix(".processed-")
            .tempfile_in(parent)
            .map_err(|e| format!("이력 임시 파일 만들기 실패: {}", e))?;
        tmp.write_all(content)
            .map_err(|e| format!("이력 임시 파일 쓰기 실패: {}", e))?;
        tmp.as_file()
            .sync_all()
            .map_err(|e| format!("이력 임시 파일 동기화 실패: {}", e))?;
        tmp.persist(path)
            .map_err(|e| format!("이력 파일 저장 실패: {}", e))?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    fn at(path: PathBuf) -> Tracker {
        Tracker {
            data: Mutex::new(ProcessedFiles::default()),
            nonpersisted_failures: Mutex::new(NonpersistedFailures::default()),
            path,
            fail_writes: std::sync::atomic::AtomicBool::new(false),
        }
    }

    fn fingerprint(modified_nanos: u128, sha256: &str) -> crate::source::Fingerprint {
        crate::source::Fingerprint {
            size: 7,
            modified_nanos,
            sha256: sha256.into(),
        }
    }

    #[test]
    fn concurrent_writes_keep_all_entries() {
        let dir = tempfile::tempdir().unwrap();
        let tracker = Arc::new(at(dir.path().join("processed.json")));
        let jobs: Vec<_> = (0..16)
            .map(|i| {
                let tracker = Arc::clone(&tracker);
                std::thread::spawn(move || tracker.mark_processed(&format!("pack-{i}"), i + 1))
            })
            .collect();
        for job in jobs {
            job.join().unwrap().unwrap();
        }
        let on_disk: ProcessedFiles =
            serde_json::from_slice(&fs::read(dir.path().join("processed.json")).unwrap()).unwrap();
        assert_eq!(on_disk.schema_version, 2);
        assert_eq!(on_disk.records.len(), 16);
        assert_eq!(tracker.get_history_with_status().len(), 16);
    }

    #[test]
    fn failed_write_does_not_publish_memory() {
        let dir = tempfile::tempdir().unwrap();
        let tracker = at(dir.path().join("missing").join("processed.json"));
        assert!(tracker.mark_processed("pack", 1).is_err());
        assert!(tracker.needs_import("pack", 1));
    }

    #[test]
    fn failed_replace_does_not_publish_memory() {
        let dir = tempfile::tempdir().unwrap();
        let destination = dir.path().join("processed.json");
        fs::create_dir(&destination).unwrap();
        let tracker = at(destination);
        assert!(tracker.mark_processed("pack", 1).is_err());
        assert!(tracker.needs_import("pack", 1));
    }

    #[test]
    fn failed_failure_write_blocks_old_manual_retry_until_reimport_is_saved() {
        let dir = tempfile::tempdir().unwrap();
        let tracker = at(dir.path().join("processed.json"));
        let original = fingerprint(1_000_000_000, "same-content");
        tracker
            .mark_processed_with_fingerprint("pack", 1, "token", Some(&original))
            .unwrap();
        tracker.request_reimport("pack").unwrap();

        tracker
            .fail_writes
            .store(true, std::sync::atomic::Ordering::SeqCst);
        assert!(tracker
            .mark_failed_retryable_at("pack", 1, Some(&original), true, 100)
            .is_err());
        assert!(tracker
            .data
            .lock()
            .unwrap()
            .retry_requested
            .contains("pack"));
        assert!(!tracker.needs_import_fingerprint_at("pack", &original, u64::MAX));
        assert_eq!(tracker.get_history_with_status()[0].status, "ok");

        assert!(tracker.request_reimport("pack").is_err());
        assert!(!tracker.needs_import_fingerprint_at("pack", &original, u64::MAX));

        tracker
            .fail_writes
            .store(false, std::sync::atomic::Ordering::SeqCst);
        tracker.request_reimport("pack").unwrap();
        assert!(tracker.needs_import_fingerprint_at("pack", &original, u64::MAX));
    }

    #[test]
    fn failed_failure_write_guard_uses_content_not_mtime() {
        let dir = tempfile::tempdir().unwrap();
        let tracker = at(dir.path().join("processed.json"));
        let original = fingerprint(1_000_000_000, "same-content");
        tracker
            .fail_writes
            .store(true, std::sync::atomic::Ordering::SeqCst);
        assert!(tracker
            .mark_failed_retryable_at("pack", 1, Some(&original), true, 100)
            .is_err());

        let touched = fingerprint(9_000_000_000, "same-content");
        let changed = fingerprint(9_000_000_000, "changed-content");
        assert!(!tracker.needs_import_fingerprint_at("pack", &touched, u64::MAX));
        assert!(tracker.needs_import_fingerprint_at("pack", &changed, u64::MAX));
        assert!(tracker.get_history_with_status().is_empty());
    }

    #[test]
    fn failed_cancel_write_blocks_same_content_without_publishing_history() {
        let dir = tempfile::tempdir().unwrap();
        let tracker = at(dir.path().join("processed.json"));
        let original = fingerprint(1_000_000_000, "same-content");
        tracker
            .fail_writes
            .store(true, std::sync::atomic::Ordering::SeqCst);

        assert!(tracker.mark_cancelled("pack", &original).is_err());
        assert!(tracker.get_history_with_status().is_empty());
        assert!(!tracker.needs_import_fingerprint_at(
            "pack",
            &fingerprint(2_000_000_000, "same-content"),
            u64::MAX
        ));
        assert!(tracker.needs_import_fingerprint_at(
            "pack",
            &fingerprint(2_000_000_000, "changed-content"),
            u64::MAX
        ));
    }

    #[test]
    fn poisoned_nonpersisted_guard_fails_closed() {
        let dir = tempfile::tempdir().unwrap();
        let tracker = Arc::new(at(dir.path().join("processed.json")));
        let poison = Arc::clone(&tracker);
        assert!(std::thread::spawn(move || {
            let _guard = poison.nonpersisted_failures.lock().unwrap();
            panic!("poison guard");
        })
        .join()
        .is_err());

        assert!(!tracker.needs_import_fingerprint_at(
            "pack",
            &fingerprint(1_000_000_000, "content"),
            0
        ));
    }

    #[test]
    fn history_write_failure_keeps_journal_and_backup_until_recovery() {
        let root = tempfile::tempdir().unwrap();
        let data = root.path().join("PrismData");
        let target = data.join("instances").join("pack");
        let stage = data
            .join(".auto-tong-stage-test")
            .join("instances")
            .join("pack");
        fs::create_dir_all(&target).unwrap();
        fs::create_dir_all(&stage).unwrap();
        fs::write(target.join("marker"), b"old").unwrap();
        fs::write(stage.join("marker"), b"new").unwrap();
        let receipt =
            crate::managed_install::commit_stage(&stage, &target, "source", "pack.zip", 3).unwrap();
        let tracker = at(root.path().join("missing").join("processed.json"));
        assert!(tracker.mark_processed("pack.zip", 3).is_err());
        drop(receipt);
        assert_eq!(fs::read(target.join("marker")).unwrap(), b"new");
        assert!(fs::read_dir(&data).unwrap().any(|entry| entry
            .unwrap()
            .file_name()
            .to_string_lossy()
            .starts_with(".auto-tong-backup-")));
        assert_eq!(
            crate::managed_install::recover_journals(&data, |key, modified, token| {
                tracker.is_committed(key, modified, token)
            })
            .unwrap(),
            1
        );
        assert_eq!(fs::read(target.join("marker")).unwrap(), b"old");
    }

    #[test]
    fn explicit_reimport_keeps_last_success_when_retry_fails() {
        let root = tempfile::tempdir().unwrap();
        let tracker = at(root.path().join("processed.json"));
        tracker.mark_processed("pack.zip", 10).unwrap();
        tracker.request_reimport("pack.zip").unwrap();
        assert!(tracker.needs_import("pack.zip", 10));
        tracker.mark_failed("pack.zip", 10).unwrap();
        assert!(tracker.is_imported("pack.zip", 10));
        assert!(!tracker.needs_import("pack.zip", 10));
        let stored: ProcessedFiles =
            serde_json::from_slice(&fs::read(root.path().join("processed.json")).unwrap()).unwrap();
        let record = stored.records.get("pack.zip").unwrap();
        assert_eq!(record.installed.as_ref().unwrap().modified_secs, 10);
        assert_eq!(record.last_attempt.as_ref().unwrap().status, "failed");
        let history = tracker.get_history_with_status();
        assert_eq!(history.len(), 1);
        assert_eq!(history[0].status, "retry_failed");
        assert!(history[0].installed);
    }

    #[test]
    fn old_success_with_same_timestamp_does_not_confirm_new_commit() {
        let root = tempfile::tempdir().unwrap();
        let tracker = at(root.path().join("processed.json"));
        tracker
            .mark_processed_with_token("pack.zip", 5, "old-commit")
            .unwrap();
        tracker.request_reimport("pack.zip").unwrap();
        let data = root.path().join("PrismData");
        let target = data.join("instances/pack");
        let stage = data.join(".auto-tong-stage-test/instances/pack");
        fs::create_dir_all(&target).unwrap();
        fs::create_dir_all(&stage).unwrap();
        fs::write(target.join("marker"), b"old").unwrap();
        fs::write(stage.join("marker"), b"new").unwrap();
        let receipt =
            crate::managed_install::commit_stage(&stage, &target, "source", "pack.zip", 5).unwrap();
        assert!(!tracker.is_committed("pack.zip", 5, receipt.token()));
        drop(receipt);
        assert_eq!(
            crate::managed_install::recover_journals(&data, |key, modified, token| {
                tracker.is_committed(key, modified, token)
            })
            .unwrap(),
            1
        );
        assert_eq!(fs::read(target.join("marker")).unwrap(), b"old");
    }

    #[test]
    fn source_mapping_separates_nested_names_and_preserves_unique_legacy_target() {
        let root = tempfile::tempdir().unwrap();
        let source_root = root.path().join("drive");
        let changed_root = root.path().join("other-drive");
        let instances = root.path().join("instances");
        fs::create_dir_all(&source_root).unwrap();
        fs::create_dir_all(&changed_root).unwrap();
        fs::create_dir_all(instances.join("pack")).unwrap();
        let tracker = at(root.path().join("processed.json"));
        tracker.mark_processed("a/pack.zip", 1).unwrap();
        let a = crate::source::identify(&source_root, "a/pack.zip", "pack").unwrap();
        let legacy = tracker.resolve_source(&a, "pack", &instances).unwrap();
        assert_eq!(legacy.instance_id, "pack");
        assert_eq!(legacy.history_key, "a/pack.zip");
        let b = crate::source::identify(&source_root, "b/pack.zip", "pack").unwrap();
        let nested = tracker.resolve_source(&b, "pack", &instances).unwrap();
        assert_ne!(nested.instance_id, legacy.instance_id);
        assert_ne!(nested.history_key, legacy.history_key);
        let moved = crate::source::identify(&changed_root, "a/pack.zip", "pack").unwrap();
        let changed = tracker.resolve_source(&moved, "pack", &instances).unwrap();
        assert_ne!(changed.instance_id, legacy.instance_id);
        let stored: ProcessedFiles =
            serde_json::from_slice(&fs::read(root.path().join("processed.json")).unwrap()).unwrap();
        assert_eq!(stored.source_map.len(), 3);
    }

    #[test]
    fn ambiguous_legacy_stem_is_not_assigned_to_either_source() {
        let root = tempfile::tempdir().unwrap();
        let source_root = root.path().join("drive");
        let instances = root.path().join("instances");
        fs::create_dir_all(&source_root).unwrap();
        fs::create_dir_all(instances.join("pack")).unwrap();
        let tracker = at(root.path().join("processed.json"));
        tracker.mark_processed("a/pack.zip", 1).unwrap();
        tracker.mark_processed("b/pack.zip", 2).unwrap();
        let a = crate::source::identify(&source_root, "a/pack.zip", "pack").unwrap();
        assert!(tracker.resolve_source(&a, "pack", &instances).is_err());
        let stored: ProcessedFiles =
            serde_json::from_slice(&fs::read(root.path().join("processed.json")).unwrap()).unwrap();
        assert!(stored.source_map.is_empty());
        assert!(stored.records.get("a/pack.zip").unwrap().migration_pending);
        assert!(tracker
            .get_history_with_status()
            .iter()
            .any(|item| item.path == "a/pack.zip" && item.status == "migration_pending"));
    }

    #[test]
    fn same_size_and_timestamp_new_content_needs_import() {
        let root = tempfile::tempdir().unwrap();
        let tracker = at(root.path().join("processed.json"));
        let first = crate::source::Fingerprint {
            size: 10,
            modified_nanos: 5_000_000_000,
            sha256: "a".repeat(64),
        };
        let second = crate::source::Fingerprint {
            sha256: "b".repeat(64),
            ..first.clone()
        };
        tracker
            .mark_processed_with_fingerprint("source-a", 5, "token", Some(&first))
            .unwrap();
        assert!(!tracker.needs_import_fingerprint("source-a", &first));
        assert!(tracker.needs_import_fingerprint("source-a", &second));
        tracker
            .mark_failed_with_fingerprint("source-a", 5, Some(&second))
            .unwrap();
        assert!(!tracker.needs_import_fingerprint("source-a", &second));
    }

    #[test]
    fn v1_history_is_backed_up_and_migrated_without_reimporting() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("processed.json");
        let old = br#"{"imported":{"pack.zip":42},"failed":{}}"#;
        fs::write(&path, old).unwrap();
        let tracker = Tracker::open_at(path.clone()).unwrap();
        assert_eq!(
            fs::read(root.path().join("processed.v1.backup.json")).unwrap(),
            old
        );
        assert!(!tracker.needs_import("pack.zip", 42));
        let on_disk: ProcessedFiles = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        assert_eq!(on_disk.schema_version, 2);
        assert_eq!(
            on_disk
                .records
                .get("pack.zip")
                .unwrap()
                .installed
                .as_ref()
                .unwrap()
                .modified_secs,
            42
        );
        let reopened = Tracker::open_at(path).unwrap();
        assert!(!reopened.needs_import("pack.zip", 42));
        assert_eq!(reopened.get_history_with_status().len(), 1);
    }

    #[test]
    fn corrupt_history_restores_valid_backup_or_blocks_scan() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("processed.json");
        fs::write(&path, b"{").unwrap();
        assert!(Tracker::open_at(path.clone()).is_err());
        assert_eq!(fs::read(&path).unwrap(), b"{");
        let backup = root.path().join("processed.backup.json");
        fs::write(&backup, br#"{"imported":{"pack.zip":42},"failed":{}}"#).unwrap();
        let recovered = Tracker::open_at(path.clone()).unwrap();
        assert!(!recovered.needs_import("pack.zip", 42));
        assert!(fs::read_dir(root.path()).unwrap().any(|entry| entry
            .unwrap()
            .file_name()
            .to_string_lossy()
            .starts_with("processed.corrupt-")));
    }

    #[test]
    fn v2_reopen_keeps_one_item_with_installed_and_recent_failure() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("processed.json");
        let tracker = Tracker::open_at(path.clone()).unwrap();
        tracker
            .mark_processed_with_token("source-a", 5, "commit-a")
            .unwrap();
        tracker.mark_failed("source-a", 6).unwrap();
        let reopened = Tracker::open_at(path).unwrap();
        let items = reopened.get_history_with_status();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].status, "retry_failed");
        assert!(items[0].installed);
        assert!(reopened.is_imported("source-a", 5));
    }

    #[test]
    fn injected_clock_keeps_install_time_when_only_attempt_changes() {
        let mut data = ProcessedFiles::empty_v2();
        data.imported.insert("source-a".into(), 5);
        data.commit_tokens.insert("source-a".into(), "first".into());
        data.reconcile_records_at(100);
        data.retry_requested.insert("source-a".into());
        data.reconcile_records_at(200);
        data.failed.insert("source-a".into(), 6);
        data.reconcile_records_at(300);
        let record = data.records.get("source-a").unwrap();
        assert_eq!(record.installed.as_ref().unwrap().installed_at_secs, 100);
        assert_eq!(record.last_attempt.as_ref().unwrap().attempted_at_secs, 300);
    }

    #[test]
    fn unchanged_source_retries_twice_after_backoff_then_needs_manual_request() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("processed.json");
        let tracker = Tracker::open_at(path.clone()).unwrap();
        let fingerprint = crate::source::Fingerprint {
            size: 4,
            modified_nanos: 1_000_000_000,
            sha256: "abcd".into(),
        };
        tracker
            .mark_processed_with_fingerprint("source-a", 1, "first", Some(&fingerprint))
            .unwrap();
        tracker
            .mark_failed_retryable_at("source-a", 1, Some(&fingerprint), true, 100)
            .unwrap();
        assert!(!tracker.needs_import_fingerprint_at("source-a", &fingerprint, 129));
        assert!(tracker.needs_import_fingerprint_at("source-a", &fingerprint, 130));
        let touched = crate::source::Fingerprint {
            modified_nanos: 2_000_000_000,
            ..fingerprint.clone()
        };
        tracker
            .mark_failed_retryable_at("source-a", 2, Some(&touched), true, 130)
            .unwrap();
        drop(tracker);

        let tracker = Tracker::open_at(path.clone()).unwrap();
        assert!(!tracker.needs_import_fingerprint_at("source-a", &touched, 249));
        assert!(tracker.needs_import_fingerprint_at("source-a", &touched, 250));
        let touched_again = crate::source::Fingerprint {
            modified_nanos: 3_000_000_000,
            ..fingerprint.clone()
        };
        tracker
            .mark_failed_retryable_at("source-a", 3, Some(&touched_again), true, 250)
            .unwrap();
        drop(tracker);

        let tracker = Tracker::open_at(path).unwrap();
        assert!(!tracker.needs_import_fingerprint_at("source-a", &touched_again, 1000));
        let data = tracker.data.lock().unwrap();
        let retry = data
            .records
            .get("source-a")
            .and_then(|record| record.auto_retry.as_ref())
            .unwrap();
        assert_eq!(retry.attempts, 3);
        assert_eq!(retry.next_retry_secs, 250);
        drop(data);
        tracker.request_reimport("source-a").unwrap();
        assert!(tracker.needs_import_fingerprint_at("source-a", &touched_again, 1000));
        assert_eq!(tracker.get_history_with_status()[0].status, "retry_failed");
    }

    #[test]
    fn cancelled_attempt_is_not_a_success_or_automatic_retry() {
        let root = tempfile::tempdir().unwrap();
        let tracker = Tracker::open_at(root.path().join("processed.json")).unwrap();
        let fingerprint = crate::source::Fingerprint {
            size: 7,
            modified_nanos: 1_000_000_000,
            sha256: "fixture".into(),
        };
        tracker.mark_cancelled("source-a", &fingerprint).unwrap();
        assert_eq!(tracker.get_history_with_status()[0].status, "cancelled");
        assert!(!tracker.needs_import_fingerprint_at("source-a", &fingerprint, u64::MAX));
        let touched = crate::source::Fingerprint {
            modified_nanos: 2_000_000_000,
            ..fingerprint.clone()
        };
        assert!(!tracker.needs_import_fingerprint_at("source-a", &touched, u64::MAX));
        let reopened = Tracker::open_at(root.path().join("processed.json")).unwrap();
        assert_eq!(reopened.get_history_with_status()[0].status, "cancelled");
        reopened.request_reimport("source-a").unwrap();
        assert!(reopened.needs_import_fingerprint_at("source-a", &fingerprint, 1));
    }

    #[test]
    fn launcher_restart_warning_keeps_installed_state_across_reopen() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("processed.json");
        let tracker = Tracker::open_at(path.clone()).unwrap();
        tracker
            .mark_processed_with_token("source-a", 1, "token-a")
            .unwrap();
        tracker
            .mark_launcher_restart_failed("source-a", "restart failed")
            .unwrap();
        let reopened = Tracker::open_at(path).unwrap();
        let item = &reopened.get_history_with_status()[0];
        assert_eq!(item.status, "launcher_warning");
        assert!(item.installed);
        reopened
            .mark_processed_with_token("source-a", 2, "token-b")
            .unwrap();
        assert_eq!(reopened.get_history_with_status()[0].status, "ok");
    }
}
