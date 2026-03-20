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
        let data = self.data.lock().unwrap();
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
        let mut data = self.data.lock().unwrap();
        data.imported.insert(relative_path.to_string(), modified_secs);
        let json = serde_json::to_string_pretty(&*data).map_err(|e| e.to_string())?;
        fs::write(&self.path, json).map_err(|e| e.to_string())?;
        Ok(())
    }

    pub fn mark_failed(&self, relative_path: &str, modified_secs: u64) -> Result<(), String> {
        let mut data = self.data.lock().unwrap();
        data.failed.insert(relative_path.to_string(), modified_secs);
        let json = serde_json::to_string_pretty(&*data).map_err(|e| e.to_string())?;
        fs::write(&self.path, json).map_err(|e| e.to_string())?;
        Ok(())
    }

    pub fn get_history(&self) -> Vec<String> {
        let data = self.data.lock().unwrap();
        let mut list: Vec<String> = data.imported.keys().cloned().collect();
        list.sort();
        list
    }
}
