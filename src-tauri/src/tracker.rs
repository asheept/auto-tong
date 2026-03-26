use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use std::sync::Mutex;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ProcessedFiles {
    pub imported: HashMap<String, u64>,
    #[serde(default)]
    pub failed: HashMap<String, u64>,
}

pub struct Tracker {
    data: Mutex<ProcessedFiles>,
    path: PathBuf,
}

impl Tracker {
    pub fn new() -> Self {
        let path = crate::config::config_dir().join("processed.json");
        let data = if path.exists() {
            let raw = fs::read_to_string(&path).unwrap_or_default();
            serde_json::from_str(&raw).unwrap_or_default()
        } else {
            ProcessedFiles::default()
        };
        Self {
            data: Mutex::new(data),
            path,
        }
    }

    pub fn needs_import(&self, relative_path: &str, modified_secs: u64) -> bool {
        let data = match self.data.lock() {
            Ok(d) => d,
            Err(e) => {
                log::error!("Tracker 락 획득 실패: {}", e);
                return true; // 안전한 방향: import 시도
            }
        };
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

    pub fn mark_processed(&self, relative_path: &str, modified_secs: u64) -> Result<(), String> {
        let json = {
            let mut data = self.data.lock()
                .map_err(|e| format!("Tracker 락 획득 실패: {}", e))?;
            data.imported.insert(relative_path.to_string(), modified_secs);
            serde_json::to_string_pretty(&*data).map_err(|e| e.to_string())?
        };
        self.atomic_write(&json)?;
        Ok(())
    }

    pub fn mark_failed(&self, relative_path: &str, modified_secs: u64) -> Result<(), String> {
        let json = {
            let mut data = self.data.lock()
                .map_err(|e| format!("Tracker 락 획득 실패: {}", e))?;
            data.failed.insert(relative_path.to_string(), modified_secs);
            serde_json::to_string_pretty(&*data).map_err(|e| e.to_string())?
        };
        self.atomic_write(&json)?;
        Ok(())
    }

    pub fn get_history(&self) -> Vec<String> {
        let data = match self.data.lock() {
            Ok(d) => d,
            Err(e) => {
                log::error!("Tracker 락 획득 실패: {}", e);
                return vec![];
            }
        };
        let mut list: Vec<String> = data.imported.keys().cloned().collect();
        list.sort();
        list
    }

    /// 가져오기 이력에서 제거 (재다운로드용)
    pub fn remove_imported(&self, relative_path: &str) -> Result<bool, String> {
        let (removed, json) = {
            let mut data = self.data.lock()
                .map_err(|e| format!("Tracker 락 획득 실패: {}", e))?;
            let removed = data.imported.remove(relative_path).is_some();
            data.failed.remove(relative_path);
            let json = serde_json::to_string_pretty(&*data).map_err(|e| e.to_string())?;
            (removed, json)
        };
        self.atomic_write(&json)?;
        Ok(removed)
    }

    /// tmp 파일에 쓰고 rename하여 atomic write (크래시 시 데이터 손상 방지)
    fn atomic_write(&self, content: &str) -> Result<(), String> {
        let tmp_path = self.path.with_extension("json.tmp");
        fs::write(&tmp_path, content)
            .map_err(|e| format!("이력 임시 파일 쓰기 실패: {}", e))?;
        fs::rename(&tmp_path, &self.path)
            .map_err(|e| format!("이력 파일 저장 실패: {}", e))?;
        Ok(())
    }
}
