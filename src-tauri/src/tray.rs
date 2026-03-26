use tauri::{
    menu::{MenuBuilder, MenuItemBuilder},
    tray::TrayIconBuilder,
    Manager,
};
use tokio::sync::mpsc;

use crate::watcher::WatcherCommand;

pub fn create_tray(
    app: &tauri::App,
    watcher_tx: mpsc::Sender<WatcherCommand>,
) -> Result<(), Box<dyn std::error::Error>> {
    let version_label = format!("Auto-Tong v{}", env!("CARGO_PKG_VERSION"));
    let version_item = MenuItemBuilder::with_id("version", &version_label).enabled(false).build(app)?;
    let check_now = MenuItemBuilder::with_id("check_now", "지금 확인").build(app)?;
    let copy_path = MenuItemBuilder::with_id("copy_push_path", "내보내기 경로 복사").build(app)?;
    let settings = MenuItemBuilder::with_id("settings", "설정").build(app)?;
    let quit = MenuItemBuilder::with_id("quit", "종료").build(app)?;

    let menu = MenuBuilder::new(app)
        .item(&version_item)
        .separator()
        .item(&check_now)
        .separator()
        .item(&copy_path)
        .separator()
        .item(&settings)
        .item(&quit)
        .build()?;

    let _tray = TrayIconBuilder::new()
        .menu(&menu)
        .tooltip("Auto-Tong")
        .icon(tauri::include_image!("icons/icon.png"))
        .on_tray_icon_event(|tray, event| {
            if let tauri::tray::TrayIconEvent::DoubleClick { .. } = event {
                open_settings_window(tray.app_handle());
            }
        })
        .on_menu_event(move |app_handle, event| {
            let id = event.id().as_ref();
            match id {
                "check_now" => {
                    let tx = watcher_tx.clone();
                    tauri::async_runtime::spawn(async move {
                        tx.send(WatcherCommand::CheckNow).await.ok();
                    });
                }
                "copy_push_path" => {
                    let config = crate::config::load();
                    match arboard::Clipboard::new() {
                        Ok(mut clipboard) => {
                            if let Err(e) = clipboard.set_text(&config.drive_sync_folder) {
                                log::error!("클립보드 복사 실패: {}", e);
                            }
                        }
                        Err(e) => log::error!("클립보드 초기화 실패: {}", e),
                    }
                }
                "settings" => {
                    open_settings_window(app_handle);
                }
                "quit" => {
                    app_handle.exit(0);
                }
                _ => {}
            }
        })
        .build(app)?;

    Ok(())
}

fn open_settings_window(app_handle: &tauri::AppHandle) {
    if let Some(window) = app_handle.get_webview_window("settings") {
        window.show().ok();
        window.set_focus().ok();
        return;
    }

    tauri::WebviewWindowBuilder::new(
        app_handle,
        "settings",
        tauri::WebviewUrl::App("index.html".into()),
    )
    .title("Auto-Tong 설정")
    .inner_size(480.0, 520.0)
    .resizable(false)
    .center()
    .build()
    .ok();
}
