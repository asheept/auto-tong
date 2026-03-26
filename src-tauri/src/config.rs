use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppConfig {
    pub drive_sync_folder: String,
    pub prismlauncher_exe: String,
    pub subscribed_tags: Vec<String>,
    pub poll_interval_secs: u64,
    pub autostart: bool,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            drive_sync_folder: String::new(),
            prismlauncher_exe: String::new(),
            subscribed_tags: vec!["everyone".to_string()],
            poll_interval_secs: 60,
            autostart: true,
        }
    }
}

pub fn config_dir() -> PathBuf {
    let dir = dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("auto-tong");
    fs::create_dir_all(&dir).ok();
    dir
}

pub fn config_path() -> PathBuf {
    config_dir().join("config.json")
}

pub fn load() -> AppConfig {
    let path = config_path();
    if path.exists() {
        let data = match fs::read_to_string(&path) {
            Ok(d) => d,
            Err(e) => {
                log::error!("설정 파일 읽기 실패: {} — 기본값 사용", e);
                return AppConfig::default();
            }
        };
        match serde_json::from_str(&data) {
            Ok(config) => config,
            Err(e) => {
                log::error!("설정 파일 손상: {} — 기본값 사용", e);
                AppConfig::default()
            }
        }
    } else {
        let config = AppConfig::default();
        save(&config).ok();
        config
    }
}

pub fn save(config: &AppConfig) -> Result<(), String> {
    let path = config_path();
    let data = serde_json::to_string_pretty(config).map_err(|e| e.to_string())?;
    let tmp_path = path.with_extension("json.tmp");
    fs::write(&tmp_path, &data)
        .map_err(|e| format!("설정 임시 파일 쓰기 실패: {}", e))?;
    fs::rename(&tmp_path, &path)
        .map_err(|e| format!("설정 파일 저장 실패: {}", e))?;
    Ok(())
}
