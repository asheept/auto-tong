use notify::{RecommendedWatcher, RecursiveMode, Watcher};
use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{mpsc, oneshot};
#[cfg(test)]
use tokio::time::sleep;
use tokio_util::sync::CancellationToken;
use walkdir::WalkDir;

use crate::config::AppConfig;
use crate::job::{self, ErrorCategory, ImportPhase, ImportProgress};
use crate::prismlauncher;
use crate::tracker::Tracker;

pub enum WatcherCommand {
    CheckNow,
    UpdateConfig(AppConfig, oneshot::Sender<Result<WatcherApply, String>>),
    CancelJob(oneshot::Sender<bool>),
}

#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WatcherApply {
    pub revision: u64,
    pub watch_mode: &'static str,
}

#[derive(Default)]
struct IssueNotifications {
    recovery: HashMap<String, HashSet<String>>,
    sources: HashMap<String, HashSet<String>>,
    commit_deferred: HashSet<String>,
    content_episodes: HashMap<String, (u64, String)>,
}

impl IssueNotifications {
    fn recovery_is_new(&mut self, root: String, error: &str) -> bool {
        Self::is_new(&mut self.recovery, root, error)
    }

    fn source_is_new(&mut self, source: String, error: &str) -> bool {
        Self::is_new(&mut self.sources, source, error)
    }

    fn is_new(issues: &mut HashMap<String, HashSet<String>>, key: String, error: &str) -> bool {
        issues.entry(key).or_default().insert(error.to_string())
    }

    fn resolve_recovery(&mut self, root: &str) {
        self.recovery.remove(root);
    }

    fn resolve_source(&mut self, source: &str) {
        self.sources.remove(source);
    }

    fn resolve_all_source_issues(&mut self, source: &str) {
        let prefix = format!("{source}\0");
        self.sources.retain(|key, _| !key.starts_with(&prefix));
    }

    fn observe_content_episode(&mut self, source: &str, size: u64, sha256: &str) -> bool {
        let episode = (size, sha256.to_string());
        if self.content_episodes.get(source) == Some(&episode) {
            return false;
        }
        self.resolve_all_source_issues(source);
        self.content_episodes.insert(source.to_string(), episode);
        true
    }

    fn retain_sources(&mut self, present: &HashSet<String>) {
        self.sources.retain(|key, _| {
            key.rsplit_once('\0')
                .is_some_and(|(source, _)| present.contains(source))
        });
        self.commit_deferred
            .retain(|source| present.contains(source));
        self.content_episodes
            .retain(|source, _| present.contains(source));
    }
}

struct FailedAttemptOutcome {
    phase: ImportPhase,
    category: ErrorCategory,
    status: &'static str,
    message: String,
    persistence_failed: bool,
}

fn record_failed_attempt_with(
    category: ErrorCategory,
    message: String,
    save: impl FnOnce() -> Result<(), String>,
) -> FailedAttemptOutcome {
    match save() {
        Ok(()) => FailedAttemptOutcome {
            phase: ImportPhase::Failed,
            category,
            status: "실패",
            message,
            persistence_failed: false,
        },
        Err(error) => FailedAttemptOutcome {
            phase: ImportPhase::RecoveryRequired,
            category: ErrorCategory::Storage,
            status: "실패 이력 저장 실패 — 저장 공간 확인 필요",
            message: error,
            persistence_failed: true,
        },
    }
}

fn record_failed_attempt(
    tracker: &Tracker,
    history_key: &str,
    modified_secs: u64,
    fingerprint: &crate::source::Fingerprint,
    category: ErrorCategory,
    message: String,
) -> FailedAttemptOutcome {
    record_failed_attempt_with(category, message, || {
        tracker.mark_failed_retryable(history_key, modified_secs, fingerprint)
    })
}

fn launcher_issue_signature(error: &str) -> &str {
    if error.contains("게임") {
        "game-running"
    } else if error.contains("조회") {
        "query-failed"
    } else if error.contains("실행 파일") {
        "missing-executable"
    } else {
        error
    }
}

async fn apply_control_command(
    command: WatcherCommand,
    config: &tokio::sync::Mutex<AppConfig>,
    scan_requested: &mut bool,
    watch_tx: &std::sync::mpsc::Sender<(std::path::PathBuf, oneshot::Sender<String>)>,
    cancellation: &CancellationToken,
    commit_gate: &AtomicU8,
) -> Option<Duration> {
    match command {
        WatcherCommand::CheckNow => {
            *scan_requested = true;
            None
        }
        WatcherCommand::UpdateConfig(new_config, acknowledgement) => {
            *scan_requested = true;
            let interval = Duration::from_secs(new_config.poll_interval_secs.max(10));
            let revision = new_config.revision;
            let folder = std::path::PathBuf::from(&new_config.drive_sync_folder);
            *config.lock().await = new_config;
            let (watch_ack_tx, watch_ack_rx) = oneshot::channel();
            let result = if watch_tx.send((folder, watch_ack_tx)).is_err() {
                Err("파일 감시 스레드가 종료되었습니다".to_string())
            } else {
                match tokio::time::timeout(Duration::from_secs(5), watch_ack_rx).await {
                    Ok(Ok(watch_mode)) => Ok(WatcherApply {
                        revision,
                        watch_mode: if watch_mode == "event" {
                            "event"
                        } else {
                            "polling"
                        },
                    }),
                    Ok(Err(_)) => Err("파일 감시 응답이 종료되었습니다".to_string()),
                    Err(_) => Err("파일 감시 적용 확인이 지연되었습니다".to_string()),
                }
            };
            acknowledgement.send(result).ok();
            Some(interval)
        }
        WatcherCommand::CancelJob(acknowledgement) => {
            let accepted =
                match commit_gate.compare_exchange(0, 1, Ordering::SeqCst, Ordering::SeqCst) {
                    Ok(_) | Err(1) => true,
                    Err(_) => false,
                };
            if accepted {
                cancellation.cancel();
                *scan_requested = false;
            }
            acknowledgement.send(accepted).ok();
            None
        }
    }
}

/// 파일이름에 태그가 포함되어 있는지 확인 (@everyone, _everyone_ 둘 다 매칭)
fn matches_tags(file_name: &str, tags: &[String]) -> bool {
    let name_lower = file_name.to_lowercase();
    tags.iter().any(|tag| {
        if tag.is_empty() {
            return false;
        }
        let clean_tag = tag.trim_start_matches('@').to_lowercase();
        if clean_tag.trim().is_empty() {
            return false;
        }
        // @everyone 형태 또는 everyone 단독 형태 둘 다 매칭
        let with_at = format!("@{}", clean_tag);
        name_lower.contains(&with_at) || name_lower.contains(&clean_tag)
    })
}

async fn cancellable<T>(
    cancellation: &CancellationToken,
    future: impl std::future::Future<Output = Result<T, String>>,
) -> Result<T, String> {
    tokio::select! {
        _ = cancellation.cancelled() => Err("사용자가 가져오기를 취소했습니다".to_string()),
        result = future => result,
    }
}

fn register_file_watcher(
    base: &Path,
    watch_tx: mpsc::Sender<WatcherCommand>,
) -> (Option<RecommendedWatcher>, &'static str) {
    match notify::recommended_watcher(move |res: Result<notify::Event, notify::Error>| {
        if let Ok(event) = res {
            let removed = matches!(event.kind, notify::EventKind::Remove(_));
            let changed_archive = matches!(
                event.kind,
                notify::EventKind::Create(_) | notify::EventKind::Modify(_)
            ) && event.paths.iter().any(|path| {
                path.extension()
                    .and_then(|ext| ext.to_str())
                    .is_some_and(|ext| {
                        ext.eq_ignore_ascii_case("zip") || ext.eq_ignore_ascii_case("mrpack")
                    })
            });
            if removed || changed_archive {
                watch_tx.try_send(WatcherCommand::CheckNow).ok();
            }
        }
    }) {
        Ok(mut watcher) if base.is_dir() => match watcher.watch(base, RecursiveMode::Recursive) {
            Ok(()) => {
                log::info!("파일 감시 시작 (재귀): {}", base.display());
                (Some(watcher), "event")
            }
            Err(error) => {
                log::warn!("파일 감시 등록 실패, 폴링 사용: {}", error);
                (None, "polling")
            }
        },
        Ok(_) => {
            log::warn!("감시 폴더가 없어 폴링 사용: {}", base.display());
            (None, "polling")
        }
        Err(error) => {
            log::warn!("파일 감시 생성 실패, 폴링 사용: {}", error);
            (None, "polling")
        }
    }
}

pub fn start(
    config: AppConfig,
    tracker: Arc<Tracker>,
    app_handle: tauri::AppHandle,
) -> mpsc::Sender<WatcherCommand> {
    let (tx, mut rx) = mpsc::channel::<WatcherCommand>(32);
    let initial_folder = config.drive_sync_folder.clone();
    let config = Arc::new(tokio::sync::Mutex::new(config));
    let issues = Arc::new(std::sync::Mutex::new(IssueNotifications::default()));

    // File system watcher -> sends CheckNow on file events
    let fs_tx = tx.clone();
    let (watch_tx, watch_rx) =
        std::sync::mpsc::channel::<(std::path::PathBuf, oneshot::Sender<String>)>();
    std::thread::spawn(move || {
        let mut current_watcher: Option<RecommendedWatcher> = None;
        let mut pending = Some((
            std::path::PathBuf::from(initial_folder),
            None::<oneshot::Sender<String>>,
        ));
        loop {
            if let Some((folder, acknowledgement)) = pending.take() {
                let base = folder.as_path();
                drop(current_watcher.take());
                let (watcher, mode) = register_file_watcher(base, fs_tx.clone());
                current_watcher = watcher;
                if let Some(ack) = acknowledgement {
                    ack.send(mode.to_string()).ok();
                }
            }
            match watch_rx.recv_timeout(Duration::from_secs(30)) {
                Ok((folder, ack)) => pending = Some((folder, Some(ack))),
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
                Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
            }
        }
    });

    tauri::async_runtime::spawn(async move {
        let mut scan_task = None;
        let mut scan_requested = true;
        let mut cancellation = CancellationToken::new();
        let mut commit_gate = Arc::new(AtomicU8::new(0));
        let mut poll_deadline = tokio::time::Instant::now();
        loop {
            if scan_requested && scan_task.is_none() {
                cancellation = CancellationToken::new();
                commit_gate = Arc::new(AtomicU8::new(0));
                let worker_cancellation = cancellation.clone();
                let worker_gate = commit_gate.clone();
                let snapshot = config.lock().await.clone();
                let worker_config = snapshot.clone();
                let worker_tracker = tracker.clone();
                let worker_app = app_handle.clone();
                let runtime = tokio::runtime::Handle::current();
                let worker_issues = issues.clone();
                scan_task = Some(tokio::task::spawn_blocking(move || {
                    runtime.block_on(scan_and_import(
                        &worker_config,
                        &worker_tracker,
                        &worker_app,
                        &worker_cancellation,
                        &worker_gate,
                        &worker_issues,
                    ));
                }));
                scan_requested = false;
                let interval = config.lock().await.poll_interval_secs.max(10);
                poll_deadline = tokio::time::Instant::now() + Duration::from_secs(interval);
            }

            tokio::select! {
                cmd = rx.recv() => {
                    match cmd {
                        Some(command) => {
                            if let Some(interval) = apply_control_command(command, &config, &mut scan_requested, &watch_tx, &cancellation, &commit_gate).await {
                                poll_deadline = tokio::time::Instant::now() + interval;
                            }
                        }
                        None => break,
                    }
                }
                result = async { scan_task.as_mut().unwrap().await }, if scan_task.is_some() => {
                    if let Err(error) = result {
                        log::error!("Import worker failed: {}", error);
                    }
                    scan_task = None;
                }
                _ = tokio::time::sleep_until(poll_deadline) => {
                    scan_requested = true;
                    let interval = config.lock().await.poll_interval_secs.max(10);
                    poll_deadline = tokio::time::Instant::now() + Duration::from_secs(interval);
                }
            }
        }
    });

    tx
}

async fn scan_and_import(
    config: &AppConfig,
    tracker: &Tracker,
    app_handle: &tauri::AppHandle,
    cancellation: &CancellationToken,
    commit_gate: &AtomicU8,
    issues: &std::sync::Mutex<IssueNotifications>,
) {
    if let Ok(instances) = prismlauncher::prism_instances_dir(
        &config.prismlauncher_exe,
        &config.prismlauncher_data_dir,
    ) {
        if let Some(root) = instances.parent() {
            let root_key = root.to_string_lossy().to_string();
            match crate::managed_install::recover_journals(root, |key, modified, token| {
                tracker.is_committed(key, modified, token)
            }) {
                Ok(count) if count > 0 => {
                    issues.lock().unwrap().resolve_recovery(&root_key);
                    log::warn!("미완료 설치 {}건 복구 후 이번 스캔을 보류합니다", count);
                    return;
                }
                Err(error) => {
                    log::error!("설치 복구 실패, 가져오기를 중단합니다: {}", error);
                    if issues.lock().unwrap().recovery_is_new(root_key, &error) {
                        send_notification(app_handle, "설치 복구 필요", &error);
                    }
                    return;
                }
                Ok(_) => issues.lock().unwrap().resolve_recovery(&root_key),
            }
        }
    }
    let base = Path::new(&config.drive_sync_folder);
    if !base.exists() {
        log::warn!(
            "Drive 폴더가 존재하지 않습니다: {}",
            config.drive_sync_folder
        );
        issues.lock().unwrap().retain_sources(&HashSet::new());
        return;
    }

    let tags = if config.subscribed_tags.is_empty() {
        vec!["everyone".to_string()]
    } else {
        config.subscribed_tags.clone()
    };
    let mut present_sources = HashSet::new();
    let source_context = format!(
        "{}\0{}",
        config.prismlauncher_exe, config.prismlauncher_data_dir
    );

    // 재귀적으로 모든 zip/mrpack 파일 탐색
    for entry in WalkDir::new(base).into_iter().filter_map(|e| e.ok()) {
        if cancellation.is_cancelled() {
            return;
        }
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
        let relative = path
            .strip_prefix(base)
            .unwrap_or(path)
            .to_string_lossy()
            .to_string();

        let legacy_stem = path
            .file_stem()
            .unwrap_or_default()
            .to_string_lossy()
            .to_string();
        let identity = match crate::source::identify(base, &relative, &legacy_stem) {
            Ok(identity) => identity,
            Err(error) => {
                log::error!("원본 식별 실패 {}: {}", relative, error);
                continue;
            }
        };
        let source_issue_key = format!("{}\0{}", identity.id, source_context);
        present_sources.insert(source_issue_key.clone());
        let prism_issue_key = format!("{source_issue_key}\0prism");
        let mapping_issue_key = format!("{source_issue_key}\0mapping");
        let preflight_issue_key = format!("{source_issue_key}\0preflight");

        // instances 폴더 찾기 (표준/portable 모두 지원)
        let instances_dir = match prismlauncher::prism_instances_dir(
            &config.prismlauncher_exe,
            &config.prismlauncher_data_dir,
        ) {
            Ok(dir) => dir,
            Err(e) => {
                log::error!("PrismLauncher instances 폴더를 찾을 수 없습니다: {}", e);
                if issues
                    .lock()
                    .unwrap()
                    .source_is_new(prism_issue_key.clone(), &e)
                {
                    send_notification(app_handle, "가져오기 실패", &e);
                }
                continue;
            }
        };
        issues.lock().unwrap().resolve_source(&prism_issue_key);
        let mapping = match tracker.resolve_source(&identity, &legacy_stem, &instances_dir) {
            Ok(mapping) => mapping,
            Err(error) => {
                log::error!("원본 매핑 보류 {}: {}", relative, error);
                if issues
                    .lock()
                    .unwrap()
                    .source_is_new(mapping_issue_key.clone(), &error)
                {
                    send_notification(app_handle, "원본 매핑 확인 필요", &error);
                }
                continue;
            }
        };
        {
            let mut state = issues.lock().unwrap();
            state.resolve_source(&mapping_issue_key);
        }
        let probe = match crate::source::probe_fingerprint(path) {
            Ok(Some(fingerprint)) => fingerprint,
            Ok(None) => continue,
            Err(error) => {
                log::warn!("원본 확인 보류 {}: {}", relative, error);
                continue;
            }
        };
        if !tracker.needs_import_fingerprint(&mapping.history_key, &probe) {
            continue;
        }
        let resume_deferred = issues
            .lock()
            .unwrap()
            .commit_deferred
            .contains(&source_issue_key);
        let preflight = if resume_deferred {
            prismlauncher::deferred_import_resume_preflight(&config.prismlauncher_exe)
        } else {
            prismlauncher::import_preflight(&config.prismlauncher_exe)
        };
        {
            let mut state = issues.lock().unwrap();
            state.observe_content_episode(&source_issue_key, probe.size, &probe.sha256);
        }
        match preflight {
            prismlauncher::ImportPreflight::Ready => {
                let mut state = issues.lock().unwrap();
                state.resolve_source(&preflight_issue_key);
                state.commit_deferred.remove(&source_issue_key);
            }
            prismlauncher::ImportPreflight::Deferred(error) => {
                let notify = issues.lock().unwrap().source_is_new(
                    preflight_issue_key.clone(),
                    launcher_issue_signature(&error),
                );
                if notify {
                    let job = ImportJob {
                        id: job::next_job_id(),
                        source_id: identity.id.clone(),
                        file_name: file_name.to_string(),
                    };
                    emit_progress(
                        app_handle,
                        &job,
                        ImportPhase::Deferred,
                        0,
                        "런처 상태로 가져오기 보류",
                        Some((ErrorCategory::Launcher, &error, true)),
                    );
                    send_notification(app_handle, "모드팩 가져오기 보류", &error);
                }
                continue;
            }
        }
        let snapshot = match crate::source::snapshot_when_stable(path).await {
            Ok(Some(snapshot)) => snapshot,
            Ok(None) => {
                log::info!("파일 동기화 중 (건너뜀): {}", relative);
                continue;
            }
            Err(error) => {
                log::warn!("원본 snapshot 보류 {}: {}", relative, error);
                continue;
            }
        };
        let history_key = &mapping.history_key;
        if !tracker.needs_import_fingerprint(history_key, &snapshot.fingerprint) {
            continue;
        }
        let modified_secs = crate::source::observed_seconds(&snapshot.fingerprint);
        let archive_path = &snapshot.archive;
        let instance_name = mapping.instance_id;
        let display_name = file_name.to_string();
        let job = ImportJob {
            id: job::next_job_id(),
            source_id: identity.id,
            file_name: display_name.clone(),
        };
        commit_gate.store(0, Ordering::SeqCst);
        emit_progress(
            app_handle,
            &job,
            ImportPhase::Preparing,
            0,
            "준비 중...",
            None,
        );
        if cancellation.is_cancelled() {
            tracker
                .mark_cancelled(&mapping.history_key, &snapshot.fingerprint)
                .ok();
            emit_progress(
                app_handle,
                &job,
                ImportPhase::Cancelled,
                0,
                "사용자가 취소했습니다",
                None,
            );
            return;
        }
        log::info!("instances 폴더: {}", instances_dir.display());

        let stage = match tempfile::Builder::new()
            .prefix(".auto-tong-stage-")
            .tempdir_in(instances_dir.parent().unwrap_or(&instances_dir))
        {
            Ok(stage) => stage,
            Err(error) => {
                let message = format!("임시 설치 폴더 생성 실패: {error}");
                let outcome = record_failed_attempt(
                    tracker,
                    &mapping.history_key,
                    modified_secs,
                    &snapshot.fingerprint,
                    ErrorCategory::Storage,
                    message,
                );
                report_failed_attempt(app_handle, &job, issues, &source_issue_key, outcome);
                continue;
            }
        };
        let stage_instances = stage.path().join("instances");
        if let Err(error) = std::fs::create_dir_all(&stage_instances) {
            let message = format!("임시 인스턴스 폴더 생성 실패: {error}");
            let outcome = record_failed_attempt(
                tracker,
                &mapping.history_key,
                modified_secs,
                &snapshot.fingerprint,
                ErrorCategory::Storage,
                message,
            );
            report_failed_attempt(app_handle, &job, issues, &source_issue_key, outcome);
            continue;
        }

        // mrpack vs zip (PrismInstance / Vanilla) 분기
        let mut import_result = if crate::mrpack::is_mrpack(archive_path) {
            let app_h = app_handle.clone();
            let download_job = &job;
            cancellable(
                cancellation,
                crate::mrpack::install_mrpack(
                    archive_path,
                    &stage_instances,
                    &instance_name,
                    |current, total, fname| {
                        let percent = (current * 100).checked_div(total).unwrap_or(0) as u32;
                        emit_progress(
                            &app_h,
                            download_job,
                            ImportPhase::Downloading,
                            percent,
                            &format!("다운로드: {}", fname),
                            None,
                        );
                    },
                ),
            )
            .await
        } else {
            // zip 형식 판별: PrismInstance vs VanillaDotMinecraft
            match prismlauncher::detect_zip_type(archive_path) {
                Ok(prismlauncher::ZipType::VanillaDotMinecraft) => {
                    log::info!("바닐라 .minecraft zip 감지 → 변환 임포트: {}", relative);
                    emit_progress(
                        app_handle,
                        &job,
                        ImportPhase::Extracting,
                        0,
                        "변환 중...",
                        None,
                    );
                    prismlauncher::import_vanilla_zip_as(
                        archive_path,
                        &stage_instances,
                        &instance_name,
                        |current, total| {
                            let percent = (current * 100).checked_div(total).unwrap_or(0) as u32;
                            emit_progress(
                                app_handle,
                                &job,
                                ImportPhase::Extracting,
                                percent,
                                "변환 중...",
                                None,
                            );
                        },
                    )
                }
                Ok(prismlauncher::ZipType::PrismInstance) => prismlauncher::import_modpack_as(
                    &stage_instances,
                    archive_path,
                    &instance_name,
                    |current, total| {
                        let percent = (current * 100).checked_div(total).unwrap_or(0) as u32;
                        emit_progress(
                            app_handle,
                            &job,
                            ImportPhase::Extracting,
                            percent,
                            "가져오는 중...",
                            None,
                        );
                    },
                ),
                Ok(prismlauncher::ZipType::Unknown) => {
                    Err("지원하지 않는 ZIP 형식입니다".to_string())
                }
                Err(e) => Err(e),
            }
        };

        if cancellation.is_cancelled() {
            import_result = Err("사용자가 가져오기를 취소했습니다".to_string());
        }

        let mut failure_category = ErrorCategory::Unknown;
        if import_result.is_ok() {
            emit_progress(
                app_handle,
                &job,
                ImportPhase::Java,
                100,
                "Java 확인 중...",
                None,
            );
            let staged_instance = stage_instances.join(&instance_name);
            import_result = if staged_instance.exists() {
                cancellable(
                    cancellation,
                    crate::java::setup_java_for_instance(&staged_instance, instances_dir.parent()),
                )
                .await
            } else {
                Err(format!(
                    "준비한 인스턴스 폴더가 없습니다: {}",
                    staged_instance.display()
                ))
            };
            if import_result.is_err() {
                failure_category = ErrorCategory::Java;
            }
        }
        let mut receipt = None;
        let mut was_running = false;
        let mut deferred = false;
        if import_result.is_ok() {
            let staged_instance = stage_instances.join(&instance_name);
            let destination = instances_dir.join(&instance_name);
            import_result = crate::managed_install::merge_into_stage(
                &staged_instance,
                &destination,
                &job.source_id,
            );
            if import_result.is_ok() {
                match prismlauncher::prepare_for_commit(&config.prismlauncher_exe).await {
                    Ok(running) => was_running = running,
                    Err(error) => {
                        import_result = Err(error);
                        deferred = true;
                        issues
                            .lock()
                            .unwrap()
                            .commit_deferred
                            .insert(source_issue_key.clone());
                        failure_category = ErrorCategory::Launcher;
                    }
                }
            }
            if import_result.is_ok() {
                if let Err(error) = prismlauncher::verify_stopped(&config.prismlauncher_exe) {
                    import_result = Err(error);
                    deferred = true;
                    issues
                        .lock()
                        .unwrap()
                        .commit_deferred
                        .insert(source_issue_key.clone());
                    failure_category = ErrorCategory::Launcher;
                }
            }
            if cancellation.is_cancelled() {
                import_result = Err("사용자가 가져오기를 취소했습니다".to_string());
            }
            if import_result.is_ok()
                && commit_gate
                    .compare_exchange(0, 2, Ordering::SeqCst, Ordering::SeqCst)
                    .is_err()
            {
                import_result = Err("사용자가 가져오기를 취소했습니다".to_string());
            }
            if import_result.is_ok() {
                match crate::managed_install::commit_stage(
                    &staged_instance,
                    &destination,
                    &job.source_id,
                    history_key,
                    modified_secs,
                ) {
                    Ok(committed) => receipt = Some(committed),
                    Err(error) => import_result = Err(error),
                }
            }
            if import_result.is_err() && !deferred {
                failure_category = ErrorCategory::Storage;
            }
        }

        match import_result {
            Ok(()) => {
                emit_progress(
                    app_handle,
                    &job,
                    ImportPhase::Recording,
                    100,
                    "이력 저장 중...",
                    None,
                );
                let token = receipt
                    .as_ref()
                    .map(|committed| committed.token())
                    .unwrap_or("");
                if let Err(error) = tracker.mark_processed_with_fingerprint(
                    history_key,
                    modified_secs,
                    token,
                    Some(&snapshot.fingerprint),
                ) {
                    log::error!("이력 저장 실패: {} - {}", relative, error);
                    emit_progress(
                        app_handle,
                        &job,
                        ImportPhase::RecoveryRequired,
                        100,
                        "설치 이력 저장 실패 — 복구 필요",
                        Some((ErrorCategory::Storage, &error, true)),
                    );
                    let persistence_key = format!("{source_issue_key}\0persistence");
                    if issues
                        .lock()
                        .unwrap()
                        .source_is_new(persistence_key, &error)
                    {
                        send_notification(
                            app_handle,
                            "이력 저장 실패",
                            &format!("{}: {}", relative, error),
                        );
                    }
                    if let Err(restart_error) = prismlauncher::restore_after_commit(
                        &config.prismlauncher_exe,
                        &config.prismlauncher_data_dir,
                        was_running,
                    )
                    .await
                    {
                        log::error!("이력 저장 실패 후 런처 재실행 실패: {}", restart_error);
                    }
                    return;
                }
                if let Some(committed) = receipt {
                    if let Err(error) = committed.finish() {
                        log::error!("설치 완료 후 백업 정리 보류: {}", error);
                        send_notification(app_handle, "설치 백업 정리 필요", &error);
                    }
                }
                match prismlauncher::restore_after_commit(
                    &config.prismlauncher_exe,
                    &config.prismlauncher_data_dir,
                    was_running,
                )
                .await
                {
                    Ok(()) => {
                        emit_progress(app_handle, &job, ImportPhase::Completed, 100, "완료", None)
                    }
                    Err(restart_error) => {
                        if let Err(error) =
                            tracker.mark_launcher_restart_failed(history_key, &restart_error)
                        {
                            log::error!("런처 재실행 경고 저장 실패: {} - {}", relative, error);
                        }
                        emit_progress(
                            app_handle,
                            &job,
                            ImportPhase::Completed,
                            100,
                            "설치 완료 — 런처 재실행 확인 필요",
                            Some((ErrorCategory::Launcher, &restart_error, true)),
                        );
                        send_notification(app_handle, "런처 재실행 확인 필요", &restart_error);
                    }
                }
                send_notification(
                    app_handle,
                    "모드팩 가져오기 완료",
                    &format!("{} 을(를) 가져왔습니다", display_name),
                );
                log::info!("가져오기 성공: {}", relative);
                let mut state = issues.lock().unwrap();
                state.resolve_all_source_issues(&source_issue_key);
                state.commit_deferred.remove(&source_issue_key);
            }
            Err(err) => {
                if cancellation.is_cancelled() {
                    if was_running {
                        prismlauncher::restore_after_commit(
                            &config.prismlauncher_exe,
                            &config.prismlauncher_data_dir,
                            true,
                        )
                        .await
                        .ok();
                    }
                    if let Err(error) = tracker.mark_cancelled(history_key, &snapshot.fingerprint) {
                        log::error!("취소 이력 저장 실패: {} - {}", relative, error);
                    }
                    emit_progress(
                        app_handle,
                        &job,
                        ImportPhase::Cancelled,
                        0,
                        "사용자가 취소했습니다",
                        None,
                    );
                    return;
                }
                if was_running {
                    if let Err(restart_error) = prismlauncher::restore_after_commit(
                        &config.prismlauncher_exe,
                        &config.prismlauncher_data_dir,
                        true,
                    )
                    .await
                    {
                        log::error!("가져오기 실패 후 런처 재실행 실패: {}", restart_error);
                    }
                }
                if deferred {
                    emit_progress(
                        app_handle,
                        &job,
                        ImportPhase::Deferred,
                        0,
                        "런처 상태로 설치 보류",
                        Some((failure_category, &err, true)),
                    );
                    if issues
                        .lock()
                        .unwrap()
                        .source_is_new(preflight_issue_key.clone(), launcher_issue_signature(&err))
                    {
                        send_notification(
                            app_handle,
                            "모드팩 가져오기 보류",
                            &format!("{}: {}", relative, err),
                        );
                    }
                } else {
                    let outcome = record_failed_attempt(
                        tracker,
                        history_key,
                        modified_secs,
                        &snapshot.fingerprint,
                        failure_category,
                        err.clone(),
                    );
                    report_failed_attempt(app_handle, &job, issues, &source_issue_key, outcome);
                }
                log::error!("가져오기 실패: {} - {}", relative, err);
            }
        }
    }
    issues.lock().unwrap().retain_sources(&present_sources);
}

struct ImportJob {
    id: u64,
    source_id: String,
    file_name: String,
}

fn report_failed_attempt(
    app_handle: &tauri::AppHandle,
    job: &ImportJob,
    issues: &std::sync::Mutex<IssueNotifications>,
    source_issue_key: &str,
    outcome: FailedAttemptOutcome,
) {
    emit_progress(
        app_handle,
        job,
        outcome.phase,
        0,
        outcome.status,
        Some((outcome.category, &outcome.message, true)),
    );
    let kind = if outcome.persistence_failed {
        "persistence"
    } else {
        "import"
    };
    let issue_key = format!("{source_issue_key}\0{kind}");
    if issues
        .lock()
        .unwrap()
        .source_is_new(issue_key, &outcome.message)
    {
        let title = if outcome.persistence_failed {
            "가져오기 이력 저장 실패"
        } else {
            "모드팩 가져오기 실패"
        };
        send_notification(app_handle, title, &outcome.message);
    }
}

fn emit_progress(
    app_handle: &tauri::AppHandle,
    job: &ImportJob,
    phase: ImportPhase,
    percent: u32,
    status: &str,
    error: Option<(ErrorCategory, &str, bool)>,
) {
    use tauri::Emitter;
    let (error_category, error, retryable) = match error {
        Some((category, message, retryable)) => (Some(category), Some(message), retryable),
        None => (None, None, false),
    };
    app_handle
        .emit(
            "import-progress",
            ImportProgress {
                job_id: job.id,
                source_id: &job.source_id,
                file_name: &job.file_name,
                phase,
                percent,
                status,
                terminal: matches!(
                    phase,
                    ImportPhase::Completed
                        | ImportPhase::Failed
                        | ImportPhase::Deferred
                        | ImportPhase::RecoveryRequired
                        | ImportPhase::Cancelled
                ),
                error_category,
                error,
                retryable,
            },
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

#[cfg(test)]
mod tests {
    use super::*;
    use sha2::{Digest, Sha512};
    use std::fs;
    use std::io::Write;
    use zip::{write::SimpleFileOptions, ZipWriter};

    #[test]
    fn issue_notifications_deduplicate_until_resolved_and_prune_deleted_sources() {
        let mut issues = IssueNotifications::default();
        assert!(issues.recovery_is_new("root-a".into(), "broken journal"));
        assert!(!issues.recovery_is_new("root-a".into(), "broken journal"));
        assert!(issues.recovery_is_new("root-a".into(), "changed journal error"));
        assert!(!issues.recovery_is_new("root-a".into(), "broken journal"));
        issues.resolve_recovery("root-a");
        assert!(issues.recovery_is_new("root-a".into(), "broken journal"));

        assert!(issues.source_is_new("source-a\0preflight".into(), "game running"));
        assert!(!issues.source_is_new("source-a\0preflight".into(), "game running"));
        // A tracker backoff/attempt-budget rejection does not call either resolver.
        // The next poll therefore remains suppressed until production observes success.
        assert!(!issues.source_is_new("source-a\0preflight".into(), "game running"));
        assert!(issues.source_is_new("source-b\0preflight".into(), "game running"));
        issues.commit_deferred.insert("source-a".into());
        issues.commit_deferred.insert("source-b".into());
        let present = HashSet::from(["source-b".to_string()]);
        issues.retain_sources(&present);
        assert!(!issues.sources.contains_key("source-a\0preflight"));
        assert!(issues.sources.contains_key("source-b\0preflight"));
        assert!(!issues.commit_deferred.contains("source-a"));
        assert!(issues.commit_deferred.contains("source-b"));
        assert!(issues.recovery.contains_key("root-a"));
    }

    #[test]
    fn failed_attempt_save_error_is_the_only_terminal_outcome() {
        let outcome =
            record_failed_attempt_with(ErrorCategory::Java, "original import error".into(), || {
                Err("history disk full".into())
            });
        assert!(matches!(outcome.phase, ImportPhase::RecoveryRequired));
        assert!(matches!(outcome.category, ErrorCategory::Storage));
        assert_eq!(outcome.message, "history disk full");
        assert!(outcome.persistence_failed);

        let outcome =
            record_failed_attempt_with(ErrorCategory::Java, "original import error".into(), || {
                Ok(())
            });
        assert!(matches!(outcome.phase, ImportPhase::Failed));
        assert!(matches!(outcome.category, ErrorCategory::Java));
        assert_eq!(outcome.message, "original import error");
        assert!(!outcome.persistence_failed);
    }

    #[test]
    fn new_content_episode_rearms_errors_without_clearing_commit_latch() {
        let source = "source-a".to_string();
        let mut issues = IssueNotifications::default();
        issues
            .content_episodes
            .insert(source.clone(), (10, "old".into()));
        issues.commit_deferred.insert(source.clone());
        assert!(issues.source_is_new(format!("{source}\0import"), "bad archive"));

        assert!(issues.observe_content_episode(&source, 10, "new"));
        assert!(issues.source_is_new(format!("{source}\0import"), "bad archive"));
        assert!(issues.commit_deferred.contains(&source));
    }

    #[test]
    fn corrupt_recovery_journal_with_absent_source_notifies_once_across_scans() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("Prism");
        fs::create_dir_all(&root).unwrap();
        fs::write(root.join(".auto-tong-journal-broken.json"), b"not-json").unwrap();
        let root_key = root.to_string_lossy().to_string();
        let mut issues = IssueNotifications::default();

        for expected_notification in [true, false] {
            let error =
                crate::managed_install::recover_journals(&root, |_, _, _| false).unwrap_err();
            assert_eq!(
                issues.recovery_is_new(root_key.clone(), &error),
                expected_notification
            );
            issues.retain_sources(&HashSet::new());
            assert!(issues.recovery.contains_key(&root_key));
        }

        fs::remove_file(root.join(".auto-tong-journal-broken.json")).unwrap();
        assert_eq!(
            crate::managed_install::recover_journals(&root, |_, _, _| false).unwrap(),
            0
        );
        issues.resolve_recovery(&root_key);
        fs::write(root.join(".auto-tong-journal-broken.json"), b"not-json").unwrap();
        let error = crate::managed_install::recover_journals(&root, |_, _, _| false).unwrap_err();
        assert!(issues.recovery_is_new(root_key, &error));
    }

    #[test]
    fn blank_tags_never_match_files() {
        for tag in ["@", "@@", "  "] {
            assert!(!matches_tags("ordinary.zip", &[tag.to_string()]));
        }
        assert!(matches_tags(
            "pack@everyone.zip",
            &["@everyone".to_string()]
        ));
        assert!(matches_tags("한글팩.zip", &["한글".to_string()]));
    }

    #[tokio::test]
    async fn scan_requests_do_not_discard_config_while_worker_is_busy() {
        let (tx, mut rx) = mpsc::channel(4);
        let config = tokio::sync::Mutex::new(AppConfig::default());
        let worker = tokio::spawn(async { sleep(Duration::from_secs(1)).await });
        let updated = AppConfig {
            drive_sync_folder: "changed-folder".to_string(),
            ..AppConfig::default()
        };
        let (watch_tx, watch_rx) =
            std::sync::mpsc::channel::<(std::path::PathBuf, oneshot::Sender<String>)>();
        std::thread::spawn(move || {
            while let Ok((_, ack)) = watch_rx.recv() {
                ack.send("event".to_string()).ok();
            }
        });
        tx.send(WatcherCommand::CheckNow).await.unwrap();
        let (ack_tx, ack_rx) = oneshot::channel();
        tx.send(WatcherCommand::UpdateConfig(updated, ack_tx))
            .await
            .unwrap();
        tx.send(WatcherCommand::CheckNow).await.unwrap();
        let mut scan_requested = false;
        let cancellation = CancellationToken::new();
        let commit_gate = AtomicU8::new(0);
        for _ in 0..3 {
            apply_control_command(
                rx.recv().await.unwrap(),
                &config,
                &mut scan_requested,
                &watch_tx,
                &cancellation,
                &commit_gate,
            )
            .await;
        }
        assert_eq!(ack_rx.await.unwrap().unwrap().watch_mode, "event");
        assert!(scan_requested);
        assert_eq!(config.lock().await.drive_sync_folder, "changed-folder");
        assert!(!worker.is_finished());
        worker.abort();
    }

    #[tokio::test]
    async fn changed_folder_receives_new_events_and_missing_folder_uses_polling() {
        let root = tempfile::tempdir().unwrap();
        let a = root.path().join("a");
        let b = root.path().join("b");
        fs::create_dir_all(&a).unwrap();
        fs::create_dir_all(&b).unwrap();
        let (tx, mut rx) = mpsc::channel(8);
        let (first, mode) = register_file_watcher(&a, tx.clone());
        assert_eq!(mode, "event");
        drop(first);
        let (_second, mode) = register_file_watcher(&b, tx.clone());
        assert_eq!(mode, "event");
        fs::write(b.join("new.mrpack"), b"fixture").unwrap();
        assert!(tokio::time::timeout(Duration::from_secs(5), rx.recv())
            .await
            .unwrap()
            .is_some());
        let (missing, mode) = register_file_watcher(&root.path().join("missing"), tx);
        assert!(missing.is_none());
        assert_eq!(mode, "polling");
    }

    #[tokio::test]
    async fn config_revisions_are_acknowledged_in_order_and_closed_watcher_is_reported() {
        let config = tokio::sync::Mutex::new(AppConfig::default());
        let (watch_tx, watch_rx) =
            std::sync::mpsc::channel::<(std::path::PathBuf, oneshot::Sender<String>)>();
        std::thread::spawn(move || {
            while let Ok((_, ack)) = watch_rx.recv() {
                ack.send("polling".to_string()).ok();
            }
        });
        let mut requested = false;
        let cancellation = CancellationToken::new();
        let commit_gate = AtomicU8::new(0);
        for revision in 1..=3 {
            let (ack_tx, ack_rx) = oneshot::channel();
            let update = AppConfig {
                revision,
                ..AppConfig::default()
            };
            apply_control_command(
                WatcherCommand::UpdateConfig(update, ack_tx),
                &config,
                &mut requested,
                &watch_tx,
                &cancellation,
                &commit_gate,
            )
            .await;
            assert_eq!(ack_rx.await.unwrap().unwrap().revision, revision);
        }
        assert_eq!(config.lock().await.revision, 3);

        let (broken_tx, broken_rx) = std::sync::mpsc::channel();
        drop(broken_rx);
        let (ack_tx, ack_rx) = oneshot::channel();
        apply_control_command(
            WatcherCommand::UpdateConfig(AppConfig::default(), ack_tx),
            &config,
            &mut requested,
            &broken_tx,
            &cancellation,
            &commit_gate,
        )
        .await;
        assert!(ack_rx.await.unwrap().is_err());
        let (cancel_ack, cancel_done) = oneshot::channel();
        apply_control_command(
            WatcherCommand::CancelJob(cancel_ack),
            &config,
            &mut requested,
            &broken_tx,
            &cancellation,
            &commit_gate,
        )
        .await;
        assert!(cancel_done.await.unwrap());
        assert!(cancellation.is_cancelled());
        assert!(!requested);
        let next = CancellationToken::new();
        commit_gate.store(2, Ordering::SeqCst);
        let (cancel_ack, cancel_done) = oneshot::channel();
        apply_control_command(
            WatcherCommand::CancelJob(cancel_ack),
            &config,
            &mut requested,
            &broken_tx,
            &next,
            &commit_gate,
        )
        .await;
        assert!(!cancel_done.await.unwrap());
        assert!(!next.is_cancelled());
    }

    #[tokio::test]
    async fn all_import_formats_can_fail_in_stage_without_touching_installed_instance() {
        let root = tempfile::tempdir().unwrap();
        let instances = root.path().join("instances");
        let installed = instances.join("pack");
        fs::create_dir_all(&installed).unwrap();
        fs::write(installed.join("marker"), b"existing user data").unwrap();
        let stage = root
            .path()
            .join(".auto-tong-stage-fixture")
            .join("instances");
        fs::create_dir_all(&stage).unwrap();

        let mrpack = root.path().join("pack.mrpack");
        let manifest = serde_json::json!({
            "formatVersion": 1, "game": "minecraft", "versionId": "fixture", "name": "fixture",
            "dependencies": {"minecraft": "1.21"},
            "files": [{"path": "mods/a.jar", "fileSize": 4,
                "hashes": {"sha512": format!("{:x}", Sha512::digest(b"good"))},
                "downloads": ["http://127.0.0.1:1/file"]}]
        });
        let mut zip = ZipWriter::new(fs::File::create(&mrpack).unwrap());
        zip.start_file("modrinth.index.json", SimpleFileOptions::default())
            .unwrap();
        zip.write_all(manifest.to_string().as_bytes()).unwrap();
        zip.finish().unwrap();
        assert!(
            crate::mrpack::install_mrpack(&mrpack, &stage, "pack", |_, _, _| {})
                .await
                .is_err()
        );

        let prism = root.path().join("pack.zip");
        let mut zip = ZipWriter::new(fs::File::create(&prism).unwrap());
        for (name, content) in [
            ("instance.cfg", "name=pack"),
            (
                "mmc-pack.json",
                r#"{"components":[{"uid":"net.minecraft","version":"1.21"}]}"#,
            ),
            ("../escape", "bad"),
        ] {
            zip.start_file(name, SimpleFileOptions::default()).unwrap();
            zip.write_all(content.as_bytes()).unwrap();
        }
        zip.finish().unwrap();
        assert!(prismlauncher::import_modpack(&stage, &prism, |_, _| {}).is_err());

        let vanilla = root.path().join("pack-vanilla.zip");
        let mut zip = ZipWriter::new(fs::File::create(&vanilla).unwrap());
        for (name, content) in [
            ("launcher_profiles.json", "{}"),
            ("versions/1.21/1.21.json", "{}"),
            ("mods/good.jar", "good"),
            ("../escape", "bad"),
        ] {
            zip.start_file(name, SimpleFileOptions::default()).unwrap();
            zip.write_all(content.as_bytes()).unwrap();
        }
        zip.finish().unwrap();
        assert!(prismlauncher::import_vanilla_zip(&vanilla, &stage, |_, _| {}).is_err());
        assert_eq!(
            fs::read(installed.join("marker")).unwrap(),
            b"existing user data"
        );

        let bad_java = stage.join("missing-java");
        assert!(
            crate::java::setup_java_for_instance(&bad_java, Some(root.path()))
                .await
                .is_err()
        );
        assert_eq!(
            fs::read(installed.join("marker")).unwrap(),
            b"existing user data"
        );
    }

    fn write_fixture_archive(path: &Path, format: &str, payload: &str) {
        let mut zip = ZipWriter::new(fs::File::create(path).unwrap());
        let entries: Vec<(&str, String)> = match format {
            "mrpack" => vec![
                (
                    "modrinth.index.json",
                    serde_json::json!({
                        "formatVersion": 1, "game": "minecraft", "versionId": payload,
                        "name": "pack", "dependencies": {"minecraft": "1.21"}, "files": []
                    })
                    .to_string(),
                ),
                ("overrides/mods/version.txt", payload.to_string()),
            ],
            "prism" => vec![
                ("instance.cfg", "name=pack".to_string()),
                (
                    "mmc-pack.json",
                    r#"{"components":[{"uid":"net.minecraft","version":"1.21"}]}"#.to_string(),
                ),
                ("mods/version.txt", payload.to_string()),
            ],
            "vanilla" => vec![
                ("launcher_profiles.json", "{}".to_string()),
                ("versions/1.21/1.21.json", "{}".to_string()),
                ("mods/version.txt", payload.to_string()),
            ],
            _ => unreachable!(),
        };
        for (name, body) in entries {
            zip.start_file(name, SimpleFileOptions::default()).unwrap();
            zip.write_all(body.as_bytes()).unwrap();
        }
        zip.finish().unwrap();
    }

    #[tokio::test]
    async fn first_and_second_import_preserve_user_data_for_all_formats() {
        for format in ["mrpack", "prism", "vanilla"] {
            let root = tempfile::tempdir().unwrap();
            let data = root.path().join("PrismData");
            let instances = data.join("instances");
            fs::create_dir_all(&instances).unwrap();
            let source = root.path().join(if format == "mrpack" {
                "pack.mrpack"
            } else {
                "pack.zip"
            });
            let target = instances.join("pack");
            for (attempt, payload) in [(1, "first"), (2, "second")] {
                write_fixture_archive(&source, format, payload);
                let stage = tempfile::Builder::new()
                    .prefix(".auto-tong-stage-")
                    .tempdir_in(&data)
                    .unwrap();
                let stage_instances = stage.path().join("instances");
                fs::create_dir_all(&stage_instances).unwrap();
                match format {
                    "mrpack" => crate::mrpack::install_mrpack(
                        &source,
                        &stage_instances,
                        "pack",
                        |_, _, _| {},
                    )
                    .await
                    .unwrap(),
                    "prism" => {
                        prismlauncher::import_modpack(&stage_instances, &source, |_, _| {}).unwrap()
                    }
                    "vanilla" => {
                        prismlauncher::import_vanilla_zip(&source, &stage_instances, |_, _| {})
                            .unwrap()
                    }
                    _ => unreachable!(),
                }
                let staged_instance = stage_instances.join("pack");
                crate::managed_install::merge_into_stage(&staged_instance, &target, format)
                    .unwrap();
                let receipt = crate::managed_install::commit_stage(
                    &staged_instance,
                    &target,
                    format,
                    "pack.zip",
                    attempt,
                )
                .unwrap();
                receipt.finish().unwrap();
                if attempt == 1 {
                    fs::create_dir_all(target.join("saves/world")).unwrap();
                    fs::write(target.join("saves/world/level.dat"), b"world").unwrap();
                    fs::write(target.join("options.txt"), b"user option").unwrap();
                }
            }
            let version = if format == "mrpack" || format == "vanilla" {
                target.join(".minecraft/mods/version.txt")
            } else {
                target.join("mods/version.txt")
            };
            assert_eq!(fs::read(version).unwrap(), b"second", "{format}");
            assert_eq!(
                fs::read(target.join("saves/world/level.dat")).unwrap(),
                b"world",
                "{format}"
            );
            assert_eq!(
                fs::read(target.join("options.txt")).unwrap(),
                b"user option",
                "{format}"
            );
        }
    }

    #[tokio::test]
    async fn archive_outputs_refuse_junctions_outside_stage_for_all_formats() {
        for format in ["mrpack", "prism", "vanilla"] {
            let root = tempfile::tempdir().unwrap();
            let stage = root.path().join("stage");
            let outside = root.path().join("outside");
            fs::create_dir_all(&stage).unwrap();
            fs::create_dir_all(&outside).unwrap();
            let source = root.path().join(if format == "mrpack" {
                "pack.mrpack"
            } else {
                "pack.zip"
            });
            write_fixture_archive(&source, format, "protected");
            let link = if format == "prism" {
                stage.join("pack/mods")
            } else {
                stage.join("pack/.minecraft/mods")
            };
            fs::create_dir_all(link.parent().unwrap()).unwrap();
            #[cfg(target_os = "windows")]
            let linked = std::os::windows::fs::symlink_dir(&outside, &link).is_ok()
                || std::process::Command::new("cmd")
                    .args(["/C", "mklink", "/J"])
                    .arg(&link)
                    .arg(&outside)
                    .output()
                    .is_ok_and(|result| result.status.success());
            #[cfg(not(target_os = "windows"))]
            let linked = std::os::unix::fs::symlink(&outside, &link).is_ok();
            if !linked {
                eprintln!("symlink/junction 생성 권한이 없어 {format} fixture를 건너뜁니다");
                continue;
            }
            let result = match format {
                "mrpack" => {
                    crate::mrpack::install_mrpack(&source, &stage, "pack", |_, _, _| {}).await
                }
                "prism" => prismlauncher::import_modpack_as(&stage, &source, "pack", |_, _| {}),
                "vanilla" => {
                    prismlauncher::import_vanilla_zip_as(&source, &stage, "pack", |_, _| {})
                }
                _ => unreachable!(),
            };
            assert!(result.is_err(), "{format} followed junction");
            assert!(
                !outside.join("version.txt").exists(),
                "{format} wrote outside stage"
            );
        }
    }

    #[tokio::test]
    async fn stalled_download_cancels_promptly_without_installing() {
        use std::io::{Read, Write};
        use std::net::TcpListener;
        let root = tempfile::tempdir().unwrap();
        let archive = root.path().join("pack.mrpack");
        let stage = tempfile::Builder::new()
            .prefix("stage-")
            .tempdir_in(root.path())
            .unwrap();
        let stage_path = stage.path().to_path_buf();
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/stalled", listener.local_addr().unwrap());
        let (ready_tx, ready_rx) = std::sync::mpsc::channel();
        let server = std::thread::spawn(move || {
            let (mut connection, _) = listener.accept().unwrap();
            let mut request = [0u8; 1024];
            assert!(connection.read(&mut request).unwrap() > 0);
            connection
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 7\r\nConnection: close\r\n\r\n")
                .unwrap();
            ready_tx.send(()).unwrap();
            std::thread::sleep(Duration::from_secs(1));
        });
        let manifest = serde_json::json!({
            "formatVersion":1, "game":"minecraft", "versionId":"fixture", "name":"fixture",
            "dependencies":{"minecraft":"1.21"}, "files":[{"path":"mods/a.jar", "fileSize":7,
                "hashes":{"sha512":format!("{:x}", Sha512::digest(b"fixture"))}, "downloads":[url]}]
        });
        let mut zip = ZipWriter::new(fs::File::create(&archive).unwrap());
        zip.start_file("modrinth.index.json", SimpleFileOptions::default())
            .unwrap();
        zip.write_all(manifest.to_string().as_bytes()).unwrap();
        zip.finish().unwrap();
        let cancellation = CancellationToken::new();
        let task_token = cancellation.clone();
        let task = tokio::spawn(async move {
            let result = cancellable(
                &task_token,
                crate::mrpack::install_mrpack(&archive, stage.path(), "pack", |_, _, _| {}),
            )
            .await;
            drop(stage);
            result
        });
        tokio::task::spawn_blocking(move || ready_rx.recv_timeout(Duration::from_secs(5)))
            .await
            .unwrap()
            .unwrap();
        let started = std::time::Instant::now();
        cancellation.cancel();
        assert!(tokio::time::timeout(Duration::from_millis(500), task)
            .await
            .unwrap()
            .unwrap()
            .is_err());
        assert!(started.elapsed() < Duration::from_millis(500));
        assert!(!stage_path.exists(), "취소 후 준비 폴더가 남았습니다");
        server.join().unwrap();
    }
}
