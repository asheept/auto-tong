use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;
use notify::{RecommendedWatcher, RecursiveMode, Watcher};
use tokio::sync::mpsc;
use tokio::time::sleep;
use walkdir::WalkDir;

use crate::config::AppConfig;
use crate::prismlauncher;
use crate::tracker::Tracker;

pub enum WatcherCommand {
    CheckNow,
    UpdateConfig(AppConfig),
}

/// 파일이름에 @태그가 포함되어 있는지 확인
fn matches_tags(file_name: &str, tags: &[String]) -> bool {
    let name_lower = file_name.to_lowercase();
    tags.iter().any(|tag| {
        if tag.is_empty() {
            return false;
        }
        // @를 이미 포함하고 있으면 그대로, 아니면 @를 붙여서 매칭
        let clean_tag = tag.trim_start_matches('@').to_lowercase();
        let pattern = format!("@{}", clean_tag);
        name_lower.contains(&pattern)
    })
}

pub fn start(
    config: AppConfig,
    tracker: Arc<Tracker>,
    app_handle: tauri::AppHandle,
) -> mpsc::Sender<WatcherCommand> {
    let (tx, mut rx) = mpsc::channel::<WatcherCommand>(32);
    let config = Arc::new(tokio::sync::Mutex::new(config));

    // File system watcher -> sends CheckNow on file events
    let fs_tx = tx.clone();
    let fs_config = config.clone();
    std::thread::spawn(move || {
        let rt = tokio::runtime::Handle::current();
        let mut current_watcher: Option<RecommendedWatcher> = None;

        loop {
            let cfg = rt.block_on(fs_config.lock()).clone();
            let watch_tx = fs_tx.clone();

            let watcher = notify::recommended_watcher(move |res: Result<notify::Event, notify::Error>| {
                if let Ok(event) = res {
                    let dominated = matches!(
                        event.kind,
                        notify::EventKind::Create(_) | notify::EventKind::Modify(_)
                    );
                    if dominated {
                        let has_target = event.paths.iter().any(|p| {
                            let ext = p.extension()
                                .and_then(|e| e.to_str())
                                .unwrap_or("");
                            ext.eq_ignore_ascii_case("zip") || ext.eq_ignore_ascii_case("mrpack")
                        });
                        if has_target {
                            log::info!("파일 변경 감지 (fs watch)");
                            watch_tx.try_send(WatcherCommand::CheckNow).ok();
                        }
                    }
                }
            });

            if let Ok(mut w) = watcher {
                let base = Path::new(&cfg.drive_sync_folder);
                if base.exists() {
                    w.watch(base, RecursiveMode::Recursive).ok();
                    log::info!("파일 감시 시작 (재귀): {}", base.display());
                }
                current_watcher = Some(w);
            } else {
                log::warn!("파일 감시 생성 실패, 폴링만 사용합니다");
            }

            std::thread::sleep(Duration::from_secs(3600));
            drop(current_watcher.take());
        }
    });

    let refresh_pending = Arc::new(AtomicBool::new(false));

    tauri::async_runtime::spawn(async move {
        loop {
            let current_config = {
                let cfg = config.lock().await;
                cfg.clone()
            };

            scan_and_import(&current_config, &tracker, &app_handle, &refresh_pending).await;

            if refresh_pending.load(Ordering::Relaxed) {
                if prismlauncher::try_refresh(&current_config.prismlauncher_exe) {
                    refresh_pending.store(false, Ordering::Relaxed);
                    log::info!("PrismLauncher 새로고침 완료 — refresh 대기 해제");
                }
            }

            let poll_duration = Duration::from_secs(current_config.poll_interval_secs.max(10));

            tokio::select! {
                cmd = rx.recv() => {
                    match cmd {
                        Some(WatcherCommand::CheckNow) => {
                            log::info!("확인 요청 수신");
                            sleep(Duration::from_secs(2)).await;
                            while rx.try_recv().is_ok() {}
                            continue;
                        }
                        Some(WatcherCommand::UpdateConfig(new_config)) => {
                            let mut cfg = config.lock().await;
                            *cfg = new_config;
                            log::info!("설정 업데이트됨");
                            continue;
                        }
                        None => break,
                    }
                }
                _ = sleep(poll_duration) => {
                    log::info!("주기적 폴링 스캔");
                }
            }
        }
    });

    tx
}

async fn scan_and_import(config: &AppConfig, tracker: &Tracker, app_handle: &tauri::AppHandle, refresh_pending: &AtomicBool) {
    let base = Path::new(&config.drive_sync_folder);
    if !base.exists() {
        log::warn!("Drive 폴더가 존재하지 않습니다: {}", config.drive_sync_folder);
        return;
    }

    let tags = if config.subscribed_tags.is_empty() {
        vec!["everyone".to_string()]
    } else {
        config.subscribed_tags.clone()
    };

    // 재귀적으로 모든 zip/mrpack 파일 탐색
    for entry in WalkDir::new(base).into_iter().filter_map(|e| e.ok()) {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }

        let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
        let ext_lower = ext.to_lowercase();
        if ext_lower != "zip" && ext_lower != "mrpack" {
            continue;
        }

        let file_name = path.file_name().unwrap_or_default().to_string_lossy();

        // 파일이름에 구독 태그가 포함되어 있는지 확인
        if !matches_tags(&file_name, &tags) {
            continue;
        }

        // base 경로 기준 상대 경로
        let relative = path.strip_prefix(base)
            .unwrap_or(path)
            .to_string_lossy()
            .to_string();

        let modified_secs = std::fs::metadata(path)
            .and_then(|m| m.modified())
            .map(|t| t.duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_secs())
            .unwrap_or(0);

        if !tracker.needs_import(&relative, modified_secs) {
            continue;
        }

        if !is_file_stable(path).await {
            log::info!("파일 동기화 중 (건너뜀): {}", relative);
            continue;
        }

        log::info!("새 모드팩 발견: {}", relative);

        let display_name = file_name.to_string();
        emit_progress(app_handle, &display_name, 0, "가져오는 중...");

        match prismlauncher::import_modpack(&config.prismlauncher_exe, path, |current, total| {
            let percent = if total > 0 { (current * 100 / total) as u32 } else { 0 };
            emit_progress(app_handle, &display_name, percent, "가져오는 중...");
        }) {
            Ok(()) => {
                emit_progress(app_handle, &display_name, 100, "완료");
                tracker.mark_processed(&relative, modified_secs).ok();
                send_notification(
                    app_handle,
                    "모드팩 가져오기 완료",
                    &format!("{} 을(를) 가져왔습니다", display_name),
                );
                log::info!("가져오기 성공: {}", relative);
                refresh_pending.store(true, Ordering::Relaxed);
            }
            Err(err) => {
                emit_progress(app_handle, &display_name, 0, "실패");
                tracker.mark_failed(&relative, modified_secs).ok();
                send_notification(
                    app_handle,
                    "모드팩 가져오기 실패",
                    &format!("{}: {}", relative, err),
                );
                log::error!("가져오기 실패: {} - {}", relative, err);
            }
        }
    }
}

async fn is_file_stable(path: &Path) -> bool {
    let size1 = match std::fs::metadata(path) {
        Ok(m) => m.len(),
        Err(_) => return false,
    };
    sleep(Duration::from_secs(1)).await;
    let size2 = match std::fs::metadata(path) {
        Ok(m) => m.len(),
        Err(_) => return false,
    };
    size1 == size2 && size1 > 0
}

fn emit_progress(app_handle: &tauri::AppHandle, file_name: &str, percent: u32, status: &str) {
    use tauri::Emitter;
    app_handle
        .emit(
            "import-progress",
            serde_json::json!({
                "file_name": file_name,
                "percent": percent,
                "status": status,
            }),
        )
        .ok();
}

fn send_notification(app_handle: &tauri::AppHandle, title: &str, body: &str) {
    use tauri_plugin_notification::NotificationExt;
    app_handle
        .notification()
        .builder()
        .title(title)
        .body(body)
        .show()
        .ok();
}
