mod config;
mod java;
mod mrpack;
mod prismlauncher;
mod tracker;
mod tray;
mod watcher;

use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use tauri::Manager;
use tauri_plugin_autostart::MacosLauncher;
use tauri_plugin_updater::UpdaterExt;

/// 연속 업데이트 실패 횟수
static UPDATE_FAIL_COUNT: AtomicU32 = AtomicU32::new(0);
/// 이 횟수 이상 연속 실패하면 UI에 안내 표시
const UPDATE_FAIL_THRESHOLD: u32 = 2;

/// 업데이트 실패 시 스마트앱컨트롤/보안 차단 안내 윈도우 표시
fn emit_update_blocked(app_handle: &tauri::AppHandle, error: &str) {
    let count = UPDATE_FAIL_COUNT.fetch_add(1, Ordering::Relaxed) + 1;
    log::error!("업데이트 설치 실패 ({}/{}): {}", count, UPDATE_FAIL_THRESHOLD, error);

    if count >= UPDATE_FAIL_THRESHOLD {
        log::warn!("업데이트 반복 실패 — Windows 보안 차단 의심");
        show_update_blocked_window(app_handle);
    }
}

/// 업데이트 차단 안내 윈도우 열기
fn show_update_blocked_window(app_handle: &tauri::AppHandle) {
    use tauri::WebviewWindowBuilder;
    // 이미 열려있으면 포커스만
    if let Some(win) = app_handle.get_webview_window("update-blocked") {
        win.set_focus().ok();
        return;
    }
    WebviewWindowBuilder::new(
        app_handle,
        "update-blocked",
        tauri::WebviewUrl::App("update-blocked.html".into()),
    )
    .title("업데이트 설치 차단됨")
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

/// semver 비교: remote가 current보다 높으면 true
fn is_newer_version(current: &str, remote: &str) -> bool {
    let parse = |v: &str| -> Vec<u32> {
        v.trim_start_matches('v')
            .split('.')
            .filter_map(|p| p.parse().ok())
            .collect()
    };
    let cur = parse(current);
    let rem = parse(remote);
    for i in 0..cur.len().max(rem.len()) {
        let c = cur.get(i).copied().unwrap_or(0);
        let r = rem.get(i).copied().unwrap_or(0);
        if r > c { return true; }
        if r < c { return false; }
    }
    false
}

#[tauri::command]
fn get_config() -> config::AppConfig {
    config::load()
}

#[tauri::command]
fn save_config(
    app_handle: tauri::AppHandle,
    new_config: config::AppConfig,
) -> Result<(), String> {
    // Handle autostart
    use tauri_plugin_autostart::ManagerExt;
    let autostart = app_handle.autolaunch();
    if new_config.autostart {
        autostart.enable().map_err(|e| e.to_string())?;
    } else {
        autostart.disable().map_err(|e| e.to_string())?;
    }

    config::save(&new_config)?;

    // Notify watcher of config change
    if let Some(tx) = app_handle.try_state::<WatcherTx>() {
        let tx = tx.0.clone();
        tauri::async_runtime::spawn(async move {
            tx.send(watcher::WatcherCommand::UpdateConfig(new_config))
                .await
                .ok();
        });
    }

    Ok(())
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
fn get_version() -> String {
    env!("CARGO_PKG_VERSION").to_string()
}

#[tauri::command]
fn get_push_path() -> String {
    let config = config::load();
    config.drive_sync_folder.clone()
}

#[tauri::command]
fn get_import_history(app_handle: tauri::AppHandle) -> Vec<String> {
    if let Some(tracker) = app_handle.try_state::<Arc<tracker::Tracker>>() {
        tracker.get_history()
    } else {
        vec![]
    }
}

#[tauri::command]
async fn reimport(app_handle: tauri::AppHandle, relative_path: String) -> Result<String, String> {
    // 경로 탐색 공격 차단
    if relative_path.contains("..") || std::path::Path::new(&relative_path).is_absolute() {
        return Err("잘못된 경로입니다".to_string());
    }

    let cfg = config::load();
    let base = std::path::Path::new(&cfg.drive_sync_folder);
    let full_path = base.join(&relative_path);

    // 원본 파일 존재 확인
    if !full_path.exists() {
        return Err(format!("원본 파일을 찾을 수 없습니다: {}", relative_path));
    }

    // tracker에서 이력 제거
    if let Some(tracker) = app_handle.try_state::<Arc<tracker::Tracker>>() {
        tracker.remove_imported(&relative_path)?;
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
    let updater = app_handle.updater().map_err(|e| e.to_string())?;
    let current_version = env!("CARGO_PKG_VERSION");
    match updater.check().await {
        Ok(Some(update)) => {
            if !is_newer_version(current_version, &update.version) {
                log::info!("원격 버전 v{}이 현재 v{}보다 높지 않음 — 건너뜀", update.version, current_version);
                return Ok("최신 버전입니다".to_string());
            }
            let version_for_spawn = update.version.clone();
            let version_for_return = update.version.clone();
            log::info!("업데이트 발견: v{} → v{}", current_version, version_for_return);
            tauri::async_runtime::spawn(async move {
                use tauri_plugin_notification::NotificationExt;
                match update.download_and_install(|_, _| {}, || {}).await {
                    Ok(()) => {
                        reset_update_fail_count();
                        log::info!("업데이트 설치 완료: {}", version_for_spawn);
                        app_handle.notification()
                            .builder()
                            .title("Auto-Tong 업데이트 완료")
                            .body(&format!("v{} 설치 완료. 앱을 재시작해주세요.", version_for_spawn))
                            .show()
                            .ok();
                    }
                    Err(e) => {
                        emit_update_blocked(&app_handle, &e.to_string());
                    }
                }
            });
            Ok(format!("v{} 업데이트를 설치합니다...", version_for_return))
        }
        Ok(None) => Ok("최신 버전입니다".to_string()),
        Err(e) => Err(format!("업데이트 확인 실패: {}", e)),
    }
}

struct WatcherTx(tokio::sync::mpsc::Sender<watcher::WatcherCommand>);

#[cfg_attr(mobile, tauri::mobile_entry_point)]
fn init_logger() {
    use simplelog::*;
    use std::fs::File;

    let log_path = config::config_dir().join("auto-tong.log");
    let mut loggers: Vec<Box<dyn SharedLogger>> = vec![
        TermLogger::new(LevelFilter::Info, Config::default(), TerminalMode::Mixed, ColorChoice::Auto),
    ];

    if let Ok(file) = File::create(&log_path) {
        loggers.push(WriteLogger::new(LevelFilter::Info, Config::default(), file));
    }

    CombinedLogger::init(loggers).ok();
    log::info!("로그 파일: {}", log_path.display());
}

pub fn run() {
    init_logger();

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
            let config = config::load();
            let tracker = Arc::new(tracker::Tracker::new());

            // Store tracker in app state
            app.manage(tracker.clone());

            // Start watcher
            let watcher_tx =
                watcher::start(config.clone(), tracker, app.app_handle().clone());

            // Store watcher tx in app state
            app.manage(WatcherTx(watcher_tx.clone()));

            // Create system tray
            tray::create_tray(app, watcher_tx)
                .map_err(|e| format!("트레이 생성 실패: {}", e))?;

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
                use tauri_plugin_notification::NotificationExt;
                let current_ver = env!("CARGO_PKG_VERSION");
                let updater = match handle.updater() {
                    Ok(u) => u,
                    Err(e) => {
                        log::warn!("업데이터 초기화 실패: {}", e);
                        return;
                    }
                };
                match updater.check().await {
                    Ok(Some(update)) => {
                        if !is_newer_version(current_ver, &update.version) {
                            log::info!("원격 버전 v{}이 현재 v{}보다 높지 않음 — 건너뜀", update.version, current_ver);
                            return;
                        }
                        let ver = update.version.clone();
                        log::info!("업데이트 발견: v{} → v{}", current_ver, ver);
                        handle.notification()
                            .builder()
                            .title("Auto-Tong 업데이트")
                            .body(&format!("v{} 업데이트를 설치합니다...", ver))
                            .show()
                            .ok();

                        match update.download_and_install(|_, _| {}, || {}).await {
                            Ok(()) => {
                                reset_update_fail_count();
                                log::info!("업데이트 설치 완료: {}", ver);
                                handle.notification()
                                    .builder()
                                    .title("Auto-Tong 업데이트 완료")
                                    .body(&format!("v{} 설치 완료. 앱을 재시작해주세요.", ver))
                                    .show()
                                    .ok();
                            }
                            Err(e) => {
                                emit_update_blocked(&handle, &e.to_string());
                            }
                        }
                    }
                    Ok(None) => log::info!("최신 버전입니다"),
                    Err(e) => log::warn!("업데이트 확인 실패: {}", e),
                }
            });

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            get_config,
            save_config,
            check_now,
            check_update,
            get_version,
            get_push_path,
            get_import_history,
            reimport,
        ])
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                // 설정 창 닫기 시 앱 종료 대신 창만 숨김
                api.prevent_close();
                window.hide().ok();
            }
        })
        .run(tauri::generate_context!())
        .expect("Auto-Tong 실행 오류");
}
