use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppConfig {
    #[serde(default)]
    pub revision: u64,
    pub drive_sync_folder: String,
    pub prismlauncher_exe: String,
    #[serde(default)]
    pub prismlauncher_data_dir: String,
    pub subscribed_tags: Vec<String>,
    pub poll_interval_secs: u64,
    pub autostart: bool,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            revision: 0,
            drive_sync_folder: String::new(),
            prismlauncher_exe: String::new(),
            prismlauncher_data_dir: String::new(),
            subscribed_tags: vec!["everyone".to_string()],
            poll_interval_secs: 60,
            autostart: true,
        }
    }
}

impl AppConfig {
    pub fn validate(&self) -> Result<(), String> {
        if self
            .subscribed_tags
            .iter()
            .any(|tag| tag.trim().trim_start_matches('@').trim().is_empty())
        {
            return Err("구독 태그는 @ 또는 공백만 입력할 수 없습니다".to_string());
        }
        Ok(())
    }
}

pub fn config_dir() -> PathBuf {
    let dir = std::env::var_os("AUTO_TONG_CONFIG_DIR")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .unwrap_or_else(|| {
            dirs::config_dir()
                .unwrap_or_else(|| PathBuf::from("."))
                .join("auto-tong")
        });
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
    save_at(&config_path(), config)
}

fn save_at(path: &std::path::Path, config: &AppConfig) -> Result<(), String> {
    config.validate()?;
    let data = serde_json::to_string_pretty(config).map_err(|e| e.to_string())?;
    let tmp_path = path.with_extension("json.tmp");
    fs::write(&tmp_path, &data).map_err(|e| format!("설정 임시 파일 쓰기 실패: {}", e))?;
    fs::rename(&tmp_path, path).map_err(|e| format!("설정 파일 저장 실패: {}", e))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn older_config_without_explicit_prism_data_dir_still_loads() {
        let raw = r#"{"drive_sync_folder":"","prismlauncher_exe":"Prism.exe","subscribed_tags":["everyone"],"poll_interval_secs":60,"autostart":true}"#;
        let config: AppConfig = serde_json::from_str(raw).unwrap();
        assert!(config.prismlauncher_data_dir.is_empty());
    }

    #[test]
    fn rejects_blank_normalized_tags() {
        for tag in ["@", "@@", "  @  ", "   "] {
            let config = AppConfig {
                subscribed_tags: vec![tag.to_string()],
                ..AppConfig::default()
            };
            assert!(config.validate().is_err(), "accepted {tag:?}");
        }
        let config = AppConfig {
            subscribed_tags: vec!["@everyone".to_string(), "한글".to_string()],
            ..AppConfig::default()
        };
        assert!(config.validate().is_ok());
    }

    #[test]
    fn revision_is_persisted_and_failed_save_keeps_previous_file() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("config.json");
        let first = AppConfig {
            revision: 1,
            ..AppConfig::default()
        };
        save_at(&path, &first).unwrap();
        let second = AppConfig {
            revision: 2,
            ..first
        };
        save_at(&path, &second).unwrap();
        let stored: AppConfig = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        assert_eq!(stored.revision, 2);
        let unavailable = root.path().join("missing").join("config.json");
        assert!(save_at(&unavailable, &second).is_err());
        let stored: AppConfig = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        assert_eq!(stored.revision, 2);
    }
}
