# 2026-09-30 검증 기록

## 환경과 실행 결과

- Windows MSVC, `rustc 1.94.1`, Node.js `22.19.0`, npm `11.18.0`, auto-tong `0.2.9`.
- `npm ci`: 통과. `cargo +stable-x86_64-pc-windows-msvc test --locked`: 88개 통과. `cargo +stable-x86_64-pc-windows-msvc build --locked`: 통과.
- `cargo +stable-x86_64-pc-windows-msvc clippy --all-targets --locked -- -D warnings` 및 `cargo fmt --all -- --check`: 통과.
- `node --check src/main.js`, Node 테스트 8개, `npm run check:version`: 통과.
- `npx --yes @fission-ai/openspec@1.13.2 validate harden-auto-tong-import-workflow --strict`: 통과.
- Python Playwright Chromium 140에서 Tauri 명령 mock을 주입한 설정 화면을 1100×850 및 390×844로 확인했다. 저장 적용/실패, 취소, Java 오류, 이력 복구 필요, 런처 재실행 경고, 긴 파일명, 이력 Enter/Tab/Escape 조작에서 page error가 없었다. 업데이트 안내 화면의 원인 변경 이벤트와 닫기 버튼의 Tauri API 호출도 확인했다. Browser plugin은 설치되어 있지 않았다. 이 검증은 실제 Tauri 창 동작을 대신하지 않는다.

## R01–R21 근거

| 진단 | 수정 후 근거 | 상태 |
| --- | --- | --- |
| R01 | `import_path.rs`, `confined_output.rs` 경로·junction fixture와 세 importer/Java 외부 쓰기 거부 | 제품 fixture 통과 |
| R02 | `mrpack.rs` 404·빈 URL 필수 파일 오류 | 제품 fixture 통과 |
| R03 | `mrpack.rs` 같은 크기 잘못된 캐시 SHA-512 재검증 | 제품 fixture 통과 |
| R04 | `watcher.rs` 세 형식 준비 실패 시 기존 인스턴스 보존, `managed_install.rs` journal 복구 | 제품 fixture 통과 |
| R05 | `watcher.rs` 세 형식 최초·두 번째 가져오기와 월드·옵션 보존 | 제품 fixture 통과 |
| R06 | `watcher.rs` 설정 revision 적용 응답과 중복 스캔 명령 처리, Playwright 적용 상태 | 제품 fixture 통과 |
| R07 | `watcher.rs` A→B 감시 재등록·B 생성 이벤트·polling 대체 | 제품 fixture 통과 |
| R08 | `source.rs` 원본 ID·동일 시각 다른 내용·복사 중 변경, `tracker.rs` 서로 다른 인스턴스 매핑 | 제품 fixture 통과 |
| R09 | `tracker.rs` 원자 저장·v1 이전·손상 복구·단일 성공/실패 이력, `managed_install.rs` 이력 저장 실패 journal | 제품 fixture 통과 |
| R10 | `launcher_meta.rs` Prism 로더 UID fixture | 제품 fixture 통과 |
| R11 | `java_runtime.rs` 가짜 `javaw.exe`·버전·아키텍처 검사 | 제품 fixture 통과 |
| R12 | `prismlauncher.rs` 프로세스 조회 실패·게임 실행·종료·재실행 fixture | 제품 fixture 통과 |
| R13 | `prismlauncher.rs` portable/기본 경로 선택 fixture | 제품 fixture 통과 |
| R14 | `mrpack.rs` override ZIP 순서 fixture | 제품 fixture 통과 |
| R15 | `mrpack.rs` 멈춘 응답 유휴 제한, `watcher.rs` 다운로드 취소·제어 명령 fixture | 제품 fixture 통과 |
| R16 | `progress-state.test.js` 이전 타이머·경고 유지, Playwright 단계·오류 화면 | 제품 fixture 통과 |
| R17 | `scripts/check-capabilities.test.js` 최소 ACL, `lib.rs` 창 show/focus/hide 로직, 실제 WebView2 버튼·제목 표시줄 닫기·재열기 | 제품 fixture와 네이티브 검증 통과 |
| R18 | `updater_policy.rs` 오류 분류, `update-blocked.html` 원인별 안내, 로컬 실패 프록시를 통한 실제 반복 실패 창 표시 | 제품 fixture와 네이티브 검증 통과 |
| R19 | `update_job.rs` 자동/수동 요청의 단일 실행 fixture | 제품 fixture 통과 |
| R20 | `mrpack.rs` 미지원 매니페스트·불완전 정의, `prismlauncher.rs` 알 수 없는 ZIP 거부 | 제품 fixture 통과 |
| R21 | `java.rs` 26.1 Java 25·미확인 버전 보류 fixture | 제품 fixture 통과 |

## 통합 검증과 남은 항목

- R18 후속 수정: 업데이트 정보를 받은 직후 실패 횟수를 초기화하던 처리를 제거했다. 버전 확인·다운로드·설치 중 발생한 연속 실패 횟수는 유지하고, 전체 작업이 성공한 뒤에만 초기화한다. 따라서 유효한 업데이트 정보를 받은 후 다운로드가 두 번 실패한 경우에도 안내 창 표시 임계값에 도달한다.
- R12·R13 후속 수정: 런처 재실행에도 검증한 데이터 루트를 `--dir` 인자로 전달한다. 설치 성공·실패·취소·이력 저장 실패 뒤의 복원 경로에 모두 같은 설정을 연결했다. `prismlauncher.rs::restart_uses_selected_data_root_and_leaves_stopped_launcher_stopped`는 기본 portable 데이터가 함께 있는 환경에서 작은따옴표·한글 경로의 선택한 루트를 인자로 전달하고, 원래 꺼진 런처는 유효하지 않은 경로여도 실행하거나 조회하지 않음을 확인했다.
- 10.1 완료: Windows에서 빌드한 `auto-tong.exe`를 `AUTO_TONG_CONFIG_DIR`로 격리 실행했다. 설치된 PrismLauncher `10.0.5.0`의 실행 파일 경로를 사용하고, Prism 데이터·설정·동기화 폴더는 `%TEMP%\auto-tong-integration-jcr9h21t` 아래로 분리했다. Java 21 런타임을 격리된 Prism 데이터에 연결했다. Minecraft `1.20.5`의 Prism ZIP, 바닐라 ZIP, MRPACK fixture를 각각 `one`으로 최초 설치하고 `two`로 재가져왔다. 세 설치에서 `saves/world/level.dat`와 `options.txt`가 유지됐다. 경로 이탈 항목이 포함된 잘못된 Prism ZIP은 거부되고 기존 `two` 설치와 외부 경로가 유지됐다. 정상 ZIP `three`으로 교체하자 재가져오기에 성공했다. `processed.json`에는 세 원본의 기록이 남았다. 앱 로그와 세 인스턴스는 해당 임시 디렉터리에 보존했다. 이 검증에서는 Prism GUI 자체를 실행하지 않았다.
- 10.2 중 실제 Windows 파일 잠금 검증: 격리된 Prism 인스턴스의 `instance.cfg`를 `CreateFileW` 공유 모드 0으로 열고 Prism ZIP을 `four`로 갱신했다. 실행 중인 앱은 OS 오류 32를 기록하고 기존 `three` 설치와 세이브를 유지했다. 잠금을 해제한 뒤 기록된 자동 재시도 시각에 앱을 다시 실행하자 약 8초 안에 `four` 설치가 완료되고 세이브가 유지됐다. 첫 검증 스크립트는 두 번의 잠금 실패로 늘어난 재시도 간격보다 짧은 90초 제한 때문에 실패했으며, 추적 파일의 다음 재시도 시각을 확인해 후속 실행으로 복구를 검증했다.
- 8.5·8.6 완료: 일반 사용자 세션에서 앱을 실행하고 실제 WebView2에 Playwright CDP로 연결했다. 반복 updater 실패 뒤 안내 창 표시, 버튼 닫기, Windows `WM_CLOSE`를 통한 제목 표시줄 닫기, 같은 HWND의 숨김·재표시를 확인했다. 자동·수동 updater 동시 요청 두 건 중 하나는 실행 중 오류로 거절됐고 로컬 프록시에는 요청 한 건만 전달됐다. 네 번의 실제 updater 네트워크 실패 요청으로 창 표시와 재사용을 확인했으며 페이지 오류는 없었다. 스크린샷과 앱 로그는 `%TEMP%\auto-tong-native-2df17f6382c348c6864eff2ca7eaf7e3`에 보존했다. 앞서 수행한 진행 표시·긴 파일명·키보드 렌더링 검증과 함께 8.6 결과로 기록한다. 10.2 중 실제 Minecraft 게임 실행 중 보류는 아직 미검증이다.
- 안내 창 포커스도 후속 실행으로 검증했다. 설정 창을 실제 활성화한 뒤 updater를 요청했으며, 최초 안내 창과 재열기 두 번 모두 `GetForegroundWindow()`가 안내 창 HWND와 일치했다. 버튼·제목 표시줄 닫기·재사용·중복 요청 거절도 함께 다시 통과했다. 이 실행의 로그와 스크린샷은 `%TEMP%\auto-tong-native-f8614a32bd0f43c28ddc2cfecfdadbfb`에 보존했다.
- Tauri `test` feature를 이용한 mock runtime 실행을 시도했지만 이 Windows 환경에서 테스트 실행 파일이 `STATUS_ENTRYPOINT_NOT_FOUND`로 시작하지 않았다. 해당 실험 코드와 feature 설정을 되돌렸고 일반 테스트 87개 재실행은 통과했다.
- 네이티브 검증 초기 실패 원인: 관리자 Python 프로세스가 `tempfile.mkdtemp`로 만든 디렉터리는 Administrators·SYSTEM만 접근할 수 있었다. 일반 사용자 및 WebView2가 프로필 디렉터리에 접근하지 못해 `0x80080005`(서버 실행 실패)가 발생했다. 사용자 권한을 상속하는 임시 디렉터리를 만들고 일반 사용자 세션에서 실행하자 검증이 통과했다. 시스템 WebView2나 제품 보안 설정은 변경하지 않았다. 초기 실패 로그도 보존했다. CDP 연결 방식은 [Microsoft Playwright WebView2 문서](https://github.com/microsoft/playwright/blob/main/docs/src/webview2.md)를 따랐다.
- 9.1 완료: CI 검증용 [초안 PR #1](https://github.com/asheept/auto-tong/pull/1)을 생성했다. [첫 실행](https://github.com/asheept/auto-tong/actions/runs/36655495357)은 npm 잠금 설치·JavaScript 검사·Rust 형식 검사를 통과했지만 GitHub의 Rust 1.98 clippy에서 `unnecessary_sort_by` 1건과 `manual_checked_ops` 3건으로 실패했다. `sort_by_key`와 `checked_div`로 수정한 커밋 `6679f1d9e021f6d7a99c8ed2243916b2931e4cb6`의 [후속 실행](https://github.com/asheept/auto-tong/actions/runs/36656073469)은 npm 잠금 설치·JavaScript 테스트·버전 검사·Rust 형식 검사·clippy·MSVC Rust 테스트·Windows 빌드를 모두 통과했다. CI의 Rust `1.98.1`에서도 Rust 테스트 88개가 통과했다.
- 게임 실행 상태 확인: 사용자가 실행한 공식 Minecraft 런처의 `javaw.exe`는 확인했지만, 설정된 PrismLauncher 실행 파일의 프로세스는 없었다. PrismLauncher 전용 조회 결과는 `stopped`였으며, 이 실행은 10.2의 실제 Prism 게임 검증 근거로 사용하지 않았다. PrismLauncher에서 시작한 게임으로 후속 검증한다.
- 완료되지 않은 항목은 `tasks.md`에 체크하지 않는다. OpenSpec archive와 기준 명세 반영(10.4)은 위 통합 검증 뒤에 진행한다.
