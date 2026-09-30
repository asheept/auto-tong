mod config;
mod confined_output;
mod import_path;
mod java;
mod java_download;
mod java_runtime;
mod job;
mod launcher_meta;
mod managed_install;
mod mrpack;
mod prismlauncher;
mod source;
mod tracker;
mod tray;
mod update_job;
mod updater_policy;
mod watcher;
mod zip_util;

use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;
use tauri::Manager;
use tauri_plugin_autostart::MacosLauncher;
use tauri_plugin_updater::UpdaterExt;

/// 연속 업데이트 실패 횟수
static UPDATE_FAIL_COUNT: AtomicU32 = AtomicU32::new(0);
/// 이 횟수 이상 연속 실패하면 UI에 안내 표시
const UPDATE_FAIL_THRESHOLD: u32 = 2;

fn report_update_failure(app_handle: &tauri::AppHandle, error: &tauri_plugin_updater::Error) {
    report_update_issue(
        app_handle,
        updater_policy::classify(error),
        &error.to_string(),
    );
}

fn report_update_issue(
    app_handle: &tauri::AppHandle,
    kind: updater_policy::UpdateFailureKind,
    error: &str,
) {
    let count = next_update_failure_count(&UPDATE_FAIL_COUNT);
    log::error!(
        "업데이트 실패 ({}/{}; {:?}): {} — {}",
        count,
        UPDATE_FAIL_THRESHOLD,
        kind,
        error,
        kind.guidance()
    );

    if count >= UPDATE_FAIL_THRESHOLD {
        show_update_error_window(app_handle, kind);
    }
}

fn next_update_failure_count(counter: &AtomicU32) -> u32 {
    counter.fetch_add(1, Ordering::Relaxed) + 1
}

fn show_update_error_window(
    app_handle: &tauri::AppHandle,
    kind: updater_policy::UpdateFailureKind,
) {
    use tauri::Emitter;
    use tauri::WebviewWindowBuilder;
    if let Some(win) = app_handle.get_webview_window("update-blocked") {
        win.show().ok();
        win.set_focus().ok();
        win.emit("update-failure-kind", kind.as_str()).ok();
        return;
    }
    WebviewWindowBuilder::new(
        app_handle,
        "update-blocked",
        tauri::WebviewUrl::App(format!("update-blocked.html?category={}", kind.as_str()).into()),
    )
    .title("업데이트 실패 안내")
    .inner_size(420.0, 340.0)
    .resizable(false)
    .center()
    .build()
    .ok();
}

/// 업데이트 성공 시 실패 카운터 초기화
fn reset_update_fail_count() {
    UPDATE_FAIL_COUNT.store(0, Ordering::Relaxed);
}

#[tauri::command]
fn get_config() -> config::AppConfig {
    config::load()
}

#[tauri::command]
async fn save_config(
    app_handle: tauri::AppHandle,
    mut new_config: config::AppConfig,
) -> Result<watcher::WatcherApply, String> {
    static SAVE_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
    let _guard = SAVE_LOCK.lock().await;
    new_config.validate()?;
    new_config.revision = config::load()
        .revision
        .checked_add(1)
        .ok_or("설정 revision 범위 초과")?;
    // Handle autostart
    use tauri_plugin_autostart::ManagerExt;
    let autostart = app_handle.autolaunch();
    let was_enabled = autostart.is_enabled().map_err(|e| e.to_string())?;
    if new_config.autostart != was_enabled {
        if new_config.autostart {
            autostart.enable().map_err(|e| e.to_string())?;
        } else {
            autostart.disable().map_err(|e| e.to_string())?;
        }
    }
    if let Err(error) = config::save(&new_config) {
        if new_config.autostart != was_enabled {
            let rollback = if was_enabled {
                autostart.enable()
            } else {
                autostart.disable()
            };
            if let Err(rollback_error) = rollback {
                return Err(format!(
                    "설정 저장 실패: {error}; 자동 실행 복구 실패: {rollback_error}"
                ));
            }
        }
        return Err(error);
    }

    // Notify watcher of config change
    let tx = app_handle
        .try_state::<WatcherTx>()
        .ok_or("설정은 저장됐지만 감시기가 실행 중이지 않습니다")?;
    let (ack_tx, ack_rx) = tokio::sync::oneshot::channel();
    tx.0.send(watcher::WatcherCommand::UpdateConfig(new_config, ack_tx))
        .await
        .map_err(|_| "설정은 저장됐지만 감시기에 전달하지 못했습니다".to_string())?;
    ack_rx
        .await
        .map_err(|_| "설정은 저장됐지만 감시기의 적용 확인을 받지 못했습니다".to_string())?
}

#[tauri::command]
async fn check_now(app_handle: tauri::AppHandle) -> Result<String, String> {
    if let Some(tx) = app_handle.try_state::<WatcherTx>() {
        tx.0.send(watcher::WatcherCommand::CheckNow)
            .await
            .map_err(|e| e.to_string())?;
        Ok("가져오기 요청을 보냈습니다".to_string())
    } else {
        Err("Watcher가 실행 중이 아닙니다".to_string())
    }
}

#[tauri::command]
async fn cancel_import(app_handle: tauri::AppHandle) -> Result<String, String> {
    let tx = app_handle
        .try_state::<WatcherTx>()
        .ok_or("Watcher가 실행 중이 아닙니다")?;
    let (ack_tx, ack_rx) = tokio::sync::oneshot::channel();
    tx.0.send(watcher::WatcherCommand::CancelJob(ack_tx))
        .await
        .map_err(|e| e.to_string())?;
    let accepted = ack_rx
        .await
        .map_err(|_| "취소 요청의 적용 확인을 받지 못했습니다".to_string())?;
    Ok(if accepted {
        "가져오기 취소를 요청했습니다"
    } else {
        "설치 확정 단계가 시작되어 취소할 수 없습니다"
    }
    .to_string())
}

#[tauri::command]
fn get_version() -> String {
    env!("CARGO_PKG_VERSION").to_string()
}

#[tauri::command]
fn get_push_path() -> String {
    let config = config::load();
    config.drive_sync_folder.clone()
}

#[tauri::command]
fn get_import_history(app_handle: tauri::AppHandle) -> Vec<tracker::HistoryItem> {
    if let Some(tracker) = app_handle.try_state::<Arc<tracker::Tracker>>() {
        tracker.get_history_with_status()
    } else {
        vec![]
    }
}

#[tauri::command]
async fn reimport(app_handle: tauri::AppHandle, relative_path: String) -> Result<String, String> {
    import_path::safe_relative(&relative_path)?;

    let cfg = config::load();
    let base = std::path::Path::new(&cfg.drive_sync_folder);
    let full_path = import_path::safe_join(base, &relative_path)?;

    // 원본 파일 존재 확인
    if !full_path.exists() {
        return Err(format!("원본 파일을 찾을 수 없습니다: {}", relative_path));
    }

    // 재가져오기를 예약하며 기존 설치 이력은 유지한다.
    if let Some(tracker) = app_handle.try_state::<Arc<tracker::Tracker>>() {
        let stem = full_path
            .file_stem()
            .ok_or("원본 파일 이름 없음")?
            .to_string_lossy();
        let identity = source::identify(base, &relative_path, &stem)?;
        let instances = prismlauncher::prism_instances_dir(
            &cfg.prismlauncher_exe,
            &cfg.prismlauncher_data_dir,
        )?;
        let mapping = tracker.resolve_source(&identity, &stem, &instances)?;
        tracker.request_reimport(&mapping.history_key)?;
    }

    // watcher에 CheckNow 전송하여 재스캔
    if let Some(tx) = app_handle.try_state::<WatcherTx>() {
        tx.0.send(watcher::WatcherCommand::CheckNow)
            .await
            .map_err(|e| e.to_string())?;
    }

    Ok(format!("{} 재다운로드를 시작합니다", relative_path))
}

#[tauri::command]
async fn check_update(app_handle: tauri::AppHandle) -> Result<String, String> {
    run_update(&app_handle).await
}

#[tauri::command]
fn get_update_status(app_handle: tauri::AppHandle) -> update_job::UpdateStatus {
    app_handle.state::<update_job::UpdateCoordinator>().status()
}

fn set_update_status(
    app_handle: &tauri::AppHandle,
    phase: &str,
    message: impl Into<String>,
    running: bool,
) {
    use tauri::Emitter;
    let coordinator = app_handle.state::<update_job::UpdateCoordinator>();
    let status = coordinator.set_status(phase, message, running);
    app_handle.emit("update-status", status).ok();
}

async fn run_update(app_handle: &tauri::AppHandle) -> Result<String, String> {
    let coordinator = app_handle.state::<update_job::UpdateCoordinator>();
    coordinator
        .run(async {
            set_update_status(app_handle, "checking", "업데이트 확인 중...", true);
            let outcome = perform_update(app_handle).await;
            match &outcome {
                Ok(message) => {
                    reset_update_fail_count();
                    set_update_status(app_handle, "completed", message, false);
                }
                Err(error) => set_update_status(app_handle, "failed", error, false),
            }
            outcome
        })
        .await
        .map_err(str::to_string)?
}

async fn perform_update(app_handle: &tauri::AppHandle) -> Result<String, String> {
    use tauri_plugin_notification::NotificationExt;
    let updater = app_handle.updater().map_err(|e| e.to_string())?;
    let current_version = env!("CARGO_PKG_VERSION");
    match updater.check().await {
        Ok(Some(update)) => {
            let newer = updater_policy::is_newer_version(current_version, &update.version)
                .inspect_err(|error| {
                    report_update_issue(
                        app_handle,
                        updater_policy::UpdateFailureKind::Unknown,
                        error,
                    );
                })?;
            if !newer {
                log::info!(
                    "원격 버전 v{}이 현재 v{}보다 높지 않음 — 건너뜀",
                    update.version,
                    current_version
                );
                return Ok("최신 버전입니다".to_string());
            }
            let version = update.version.clone();
            log::info!("업데이트 발견: v{} → v{}", current_version, version);
            set_update_status(
                app_handle,
                "installing",
                format!("v{version} 다운로드 및 설치 중..."),
                true,
            );
            if let Err(error) = update.download_and_install(|_, _| {}, || {}).await {
                report_update_failure(app_handle, &error);
                return Err(format!("업데이트 설치 실패: {error}"));
            }
            log::info!("업데이트 설치 완료: {}", version);
            app_handle
                .notification()
                .builder()
                .title("Auto-Tong 업데이트 완료")
                .body(format!("v{} 설치 완료. 앱을 재시작해주세요.", version))
                .show()
                .ok();
            Ok(format!("v{} 설치 완료. 앱을 재시작해주세요.", version))
        }
        Ok(None) => Ok("최신 버전입니다".to_string()),
        Err(e) => {
            report_update_failure(app_handle, &e);
            Err(format!("업데이트 확인 실패: {}", e))
        }
    }
}

struct WatcherTx(tokio::sync::mpsc::Sender<watcher::WatcherCommand>);

#[cfg_attr(mobile, tauri::mobile_entry_point)]
fn init_logger() {
    use simplelog::*;

    let log_path = config::config_dir().join("auto-tong.log");
    let mut loggers: Vec<Box<dyn SharedLogger>> = vec![TermLogger::new(
        LevelFilter::Info,
        Config::default(),
        TerminalMode::Mixed,
        ColorChoice::Auto,
    )];

    if let Ok(file) = open_log_file(&log_path) {
        loggers.push(WriteLogger::new(LevelFilter::Info, Config::default(), file));
    }

    CombinedLogger::init(loggers).ok();
    log::info!("로그 파일: {}", log_path.display());
}

fn open_log_file(path: &std::path::Path) -> std::io::Result<std::fs::File> {
    std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
}

pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            use tauri_plugin_notification::NotificationExt;
            app.notification()
                .builder()
                .title("Auto-Tong")
                .body("이미 실행 중입니다")
                .show()
                .ok();
            // 기존 설정 창이 있으면 포커스
            if let Some(win) = app.get_webview_window("settings") {
                win.set_focus().ok();
            }
        }))
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_autostart::init(
            MacosLauncher::LaunchAgent,
            Some(vec![]),
        ))
        .setup(|app| {
            init_logger();
            app.manage(update_job::UpdateCoordinator::new());
            let config = config::load();
            let tracker = Arc::new(
                tracker::Tracker::new().map_err(|error| format!("이력 복구 실패: {error}"))?,
            );

            // Store tracker in app state
            app.manage(tracker.clone());

            // Start watcher
            let watcher_tx = watcher::start(config.clone(), tracker, app.app_handle().clone());

            // Store watcher tx in app state
            app.manage(WatcherTx(watcher_tx.clone()));

            // Create system tray
            tray::create_tray(app, watcher_tx).map_err(|e| format!("트레이 생성 실패: {}", e))?;

            // If drive_sync_folder is empty (first run), open settings
            if config.drive_sync_folder.is_empty() {
                if let Some(handle) = app.get_webview_window("settings") {
                    handle.set_focus().ok();
                } else {
                    tauri::WebviewWindowBuilder::new(
                        app,
                        "settings",
                        tauri::WebviewUrl::App("index.html".into()),
                    )
                    .title("Auto-Tong 설정")
                    .inner_size(480.0, 520.0)
                    .resizable(false)
                    .center()
                    .build()?;
                }
            }

            // 백그라운드 업데이트 체크
            let handle = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                if let Err(error) = run_update(&handle).await {
                    log::warn!("자동 업데이트 확인 실패: {}", error);
                }
            });

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            get_config,
            save_config,
            check_now,
            cancel_import,
            check_update,
            get_update_status,
            get_version,
            get_push_path,
            get_import_history,
            reimport,
        ])
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                if matches!(window.label(), "settings" | "update-blocked") {
                    api.prevent_close();
                    window.hide().ok();
                }
            }
        })
        .run(tauri::generate_context!())
        .expect("Auto-Tong 실행 오류");
}

#[cfg(test)]
mod log_tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn second_start_preserves_previous_log() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("auto-tong.log");
        writeln!(open_log_file(&path).unwrap(), "first start").unwrap();
        writeln!(open_log_file(&path).unwrap(), "second start").unwrap();
        assert_eq!(
            std::fs::read_to_string(path).unwrap(),
            "first start\nsecond start\n"
        );
    }
}

#[cfg(test)]
mod updater_count_tests {
    use super::*;

    #[test]
    fn repeated_failures_reach_notice_threshold() {
        let counter = AtomicU32::new(0);
        assert_eq!(next_update_failure_count(&counter), 1);
        assert_eq!(next_update_failure_count(&counter), UPDATE_FAIL_THRESHOLD);
        counter.store(0, Ordering::Relaxed);
        assert_eq!(next_update_failure_count(&counter), 1);
    }
}
