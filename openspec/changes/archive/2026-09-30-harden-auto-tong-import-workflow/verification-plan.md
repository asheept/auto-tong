# 회귀 검증 배치

[`review_tests.rs`](../../../reviews/2026-09-30/verification/review_tests.rs)의 15개 테스트는 수정 전 오류를 재현한 증거다. 이 파일은 변경하지 않고 보관한다. 아래 대상의 제품 테스트는 수정 후 기대 결과를 확인한다. `구현`은 제품 회귀 테스트가 존재함을 뜻한다. 실제 앱·게임·창 검증 결과는 [검증 기록](verification-report.md)에 있다.

| 기존 진단 | 제품 회귀 테스트 위치 | 상태 |
| --- | --- | --- |
| R01 매니페스트 이탈 | `mrpack.rs` 경로 이탈 fixture, `import_path.rs` 경로 단위 테스트, `confined_output.rs` 링크 fixture | 구현 |
| R01 Windows rooted | `import_path.rs::rejects_windows_and_parent_escape` | 구현 |
| R02 HTTP 404 | `mrpack.rs::http_error_fails_required_file` | 구현 |
| R02 빈 URL | `mrpack.rs::missing_required_url_fails` | 구현 |
| R03 같은 크기 캐시 | `mrpack.rs::same_size_bad_cache_is_replaced_after_hash_check` | 구현 |
| R04 실패한 ZIP 설치 | `watcher.rs::all_import_formats_can_fail_in_stage_without_touching_installed_instance` | 구현 |
| R05 바닐라 재가져오기 | `watcher.rs::first_and_second_import_preserve_user_data_for_all_formats` | 구현 |
| R08 같은 이름의 다른 원본 | `tracker.rs::source_mapping_separates_nested_names_and_preserves_unique_legacy_target` | 구현 |
| R09 이력 쓰기 실패 | `tracker.rs::failed_write_does_not_publish_memory` | 구현 |
| R09 중복 성공·실패 이력 | `tracker.rs::explicit_reimport_keeps_last_success_when_retry_fails` | 구현 |
| R10 NeoForge UID | `launcher_meta.rs::official_prism_loader_ids` | 구현 |
| R11 불완전한 Java | `java_runtime.rs::fake_javaw_and_missing_runtime_are_rejected` | 구현 |
| R14 overrides 순서 | `mrpack.rs::override_layers_ignore_archive_entry_order` | 구현 |
| R20 미지원 매니페스트 | `mrpack.rs::unsupported_manifest_fails_before_creating_instance` | 구현 |
| R21 Minecraft 26.1 Java | `java.rs::known_versions_have_confirmed_offline_requirements` | 구현 |

| 점검 항목 | 대응 spec scenario | 제품 검증 위치 |
| --- | --- | --- |
| R01 | safe-import-paths: 매니페스트 경로 이탈, 기존 링크와 연결된 출력 경로, 한글과 중첩 경로 | `import_path.rs`, 세 importer 경로 fixture, 2.2–2.3 |
| R02–R03 | reliable-modpack-install: 필수 다운로드 실패, 같은 크기의 잘못된 파일 | `mrpack.rs` HTTP·캐시 테스트, 4.1–4.2 |
| R04–R05 | reliable-modpack-install: 준비 단계 실패, 재가져오기, 설치 확정 도중 중단 | `managed_install.rs` journal fixture와 `watcher.rs` 세 형식 fixture, 6.1–6.5 |
| R06–R07 | import-orchestration: 스캔 대기 중 설정 변경, 감시 폴더 교체, 새 폴더 감시 실패 | `watcher.rs` 제어·감시 fixture, 7.1–7.3 |
| R08 | import-orchestration: 같은 이름의 다른 팩, 루트 또는 콘텐츠 변경, 원본을 복사하는 중 | `tracker.rs` 원본 매핑 및 `source.rs` fingerprint·snapshot fixture, 3.1–3.2·7.4 |
| R09 | durable-import-history: 이력 저장 실패, 재가져오기 실패, 완료 기록과 재시도가 겹침, 설치 확정 후 이력 저장 전 중단, 손상된 이력, 같은 이름에 여러 후보 존재 | `tracker.rs` 저장·이전 fixture와 `managed_install.rs` journal fixture, 3.3–3.5·6.4 |
| R10 | launcher-runtime-integration: NeoForge 팩 | `launcher_meta.rs` 공식 UID fixture, 5.2 |
| R11 | launcher-runtime-integration: 불완전한 Java 디렉터리 | `java.rs` 런타임 검증 fixture, 5.4 |
| R12 | launcher-runtime-integration: 프로세스 조회 실패, 실행 전 상태 보존, 종료 또는 재실행 실패 | `prismlauncher.rs` 프로세스 fixture, 5.5–5.6 |
| R13 | launcher-runtime-integration: 기본 설치와 portable 설치 공존 | `prismlauncher.rs` 데이터 위치 fixture, 5.1 |
| R14 | reliable-modpack-install: 같은 경로에 여러 콘텐츠 존재 | `mrpack.rs` overrides fixture, 4.4 |
| R15 | reliable-modpack-install: 응답이 멈춘 서버, 설치 중 취소 | `mrpack.rs` 시간 제한·취소 및 `watcher.rs` 제어 fixture, 4.3·7.1 |
| R16 | desktop-operation-status: 연속 가져오기, 다운로드 후 Java 준비 | `progress-state.test.js`, `watcher.rs` 단계 fixture, 8.1–8.2 |
| R17 | desktop-operation-status: 안내 창 재사용 | Tauri ACL·창 통합 fixture, 8.5 |
| R18 | desktop-operation-status: 반복되는 네트워크 실패 | updater 오류 분류 및 화면 fixture, 8.4 |
| R19 | desktop-operation-status: 자동 설치 중 수동 검사 | updater 단일 작업 fixture, 8.3 |
| R20 | safe-import-paths: 지원하지 않는 매니페스트, 알 수 없는 ZIP, 불완전한 다운로드 정의 | `mrpack.rs`, `prismlauncher.rs` preflight fixture, 2.4 |
| R21 | launcher-runtime-integration: Minecraft 26.1, 미확인 버전 또는 메타데이터 장애 | `java.rs` resolver fixture, 5.3 |

나머지 import-orchestration의 빈 태그 저장 scenario는 `config.rs::rejects_blank_normalized_tags`와 `watcher.rs::blank_tags_never_match_files`에서 확인한다. 기능별 fixture는 임시 디렉터리와 가짜 프로세스를 사용하고 실제 사용자 Prism 데이터 폴더를 변경하지 않는다.

7.2–7.4 검증: `config.rs::revision_is_persisted_and_failed_save_keeps_previous_file`, `watcher.rs::config_revisions_are_acknowledged_in_order_and_closed_watcher_is_reported`, `watcher.rs::changed_folder_receives_new_events_and_missing_folder_uses_polling`, `tracker.rs::unchanged_source_retries_twice_after_backoff_then_needs_manual_request`, `mrpack.rs::unchanged_archive_succeeds_after_server_recovers`, `source.rs::probe_detects_same_size_same_time_change_without_copying`를 통과했다. Browser plugin이 없어 Python Playwright Chromium으로 정적 설정 화면에 Tauri 명령 mock을 주입했다. 저장 대기 중 버튼 비활성화, revision 2 적용 확인, 적용 응답 실패 표시, 긴 파일명 줄바꿈을 1100×850 및 390×844 화면에서 확인했고 브라우저 page error는 없었다. 실제 Tauri 창 수명과 Prism 실행은 별도 통합 검증 대상이다.

2.3 검증: `confined_output.rs`는 열린 디렉터리 핸들과 `open_dir_nofollow`를 사용한다. `confined_output.rs::rejects_parent_link_after_directory_was_checked`, `rejects_link_leaf_without_touching_target`, `watcher.rs::archive_outputs_refuse_junctions_outside_stage_for_all_formats`, `java.rs::java_zip_refuses_junction_to_external_bin`, `import_path.rs::normalized_names_reject_windows_case_and_separator_aliases`가 Windows 임시 디렉터리에서 통과했다. 이 환경에서는 심볼릭 링크 생성이 허용되어 권한 부족으로 건너뛴 fixture가 없었다. 핵심 API 근거: https://docs.rs/cap-std/4.0.3/cap_std/fs/struct.Dir.html 및 https://docs.rs/cap-fs-ext/4.0.3/cap_fs_ext/trait.DirExt.html.

4.3 검증: MRPACK은 연결 10초, 유휴 읽기 30초, 요청 전체 15분으로 제한한다. `mrpack.rs::stalled_body_stops_at_configured_idle_timeout`은 테스트 주입한 100ms 유휴 제한 안에 멈춘 응답을 실패로 종료했고, `watcher.rs::stalled_download_cancels_promptly_without_installing`은 취소 후 500ms 안에 종료했다. `watcher.rs::config_revisions_are_acknowledged_in_order_and_closed_watcher_is_reported`는 작업 중 제어 응답과 commit 시작 뒤 취소 거부를 확인한다. 자동 재시도는 `tracker.rs::unchanged_source_retries_twice_after_backoff_then_needs_manual_request`의 30초·120초 제한을 따른다. 설정 화면 mock 렌더링에서 취소 버튼과 이력의 취소 상태를 확인했다.

## 교체 가능한 테스트 경계

| 경계 | 주입 위치와 실패 fixture |
| --- | --- |
| 파일 시스템 | `managed_install.rs::merge_into_stage_with_available`의 여유 공간 조회, `commit_stage_with_hook`의 journal/rename 중단 지점, `tracker.rs`의 임시 경로 저장 실패. 모두 `tempfile`에만 쓴다. |
| HTTP | `mrpack.rs`와 `java_download.rs`의 `127.0.0.1` 일회성 서버에서 HTTP 오류·짧은 응답·체크섬 불일치를 만든다. |
| 프로세스 | `prismlauncher.rs::prepare_for_commit_with`와 `restore_after_commit_with`에 가짜 상태·종료·실행 함수를 주입한다. PowerShell 중첩 프로세스 fixture는 `Get-Process`와 `Get-CimInstance`를 테스트 함수로 대체한다. |
| 시계 | `tracker.rs::reconcile_records_at`에 고정 시각을 주입해 설치 시각과 재시도 시각을 분리해 검증한다. |
