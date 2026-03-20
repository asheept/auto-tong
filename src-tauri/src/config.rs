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
            drive_sync_folder: "G:/공유 드라이브/잠뜰 TV 제작/Contents/통합".to_string(),
            prismlauncher_exe: "C:/Users/h2art/AppData/Local/Programs/PrismLauncher/prismlauncher.exe".to_string(),
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
        let data = fs::read_to_string(&path).unwrap_or_default();
        serde_json::from_str(&data).unwrap_or_default()
    } else {
        let config = AppConfig::default();
        save(&config).ok();
        config
    }
}

pub fn save(config: &AppConfig) -> Result<(), String> {
    let path = config_path();
    let data = serde_json::to_string_pretty(config).map_err(|e| e.to_string())?;
    fs::write(&path, data).map_err(|e| e.to_string())?;
    Ok(())
}
