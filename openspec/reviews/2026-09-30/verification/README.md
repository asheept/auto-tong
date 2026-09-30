# 점검 재현 방법

## 목적

2026-09-30, `6463876`의 현재 오류 동작을 확인하기 위한 진단이다. **아래 테스트 통과는 결함 재현 성공을 의미한다.** 수정 후 제품 회귀 테스트는 올바른 기대 동작으로 별도 작성한다. 제품이 수정되면 이 진단은 실패할 수 있다.

실제 Rust 모듈을 임시 crate에서 불러온다. 비공개 ZIP 추출기와 tracker 생성에만 테스트 wrapper를 사용하고 제품 코드를 복제하거나 수정하지 않는다. 테스트 데이터는 임시 디렉터리, 다운로드 응답은 `127.0.0.1`의 일회성 서버를 사용한다.

## 환경과 실행

프로젝트 루트 PowerShell에서 실행한다. Python 3, Node 22, Rust MSVC toolchain, Visual Studio C++ BuildTools가 필요하다. Rust 진단은 `--offline`이므로 원본 프로젝트의 의존성이 로컬 Cargo cache에 있어야 한다.

```powershell
# 프로젝트 루트
python openspec\reviews\2026-09-30\verification\run_rust_review.py
node openspec\reviews\2026-09-30\verification\review-ui.cjs
node --check src\main.js

# src-tauri 디렉터리에서 기존 제품 테스트
cargo +stable-x86_64-pc-windows-msvc test --locked
```

Python runner는 원본 Cargo.lock을 임시 crate로 복사하고 임시 crate의 lock만 해석·갱신한다. 빌드 캐시는 프로젝트 `src-tauri/target`을 재사용한다. Windows 경로와 `javaw.exe`를 검사하므로 이 진단은 Windows 전용이다. UI ACL 검사는 Tauri 빌드가 생성한 `src-tauri/gen/schemas/acl-manifests.json`을 사용한다.

## 2026-09-30 결과

Rust: **19 passed, 0 failed**. 기존 ZIP 이름 테스트 4개와 아래 진단 15개가 포함된다.

| ID | 진단 함수 suffix | 확인한 현재 동작 |
| --- | --- | --- |
| R01 | `manifest_path_escapes_minecraft_directory` | 매니페스트 경로로 .minecraft 밖에 기록 |
| R01 | `windows_rooted_path_bypasses_existing_guard` | rooted 경로가 기존 검사 통과; 실제 루트 쓰기는 안 함 |
| R02 | `http_404_is_reported_as_success` | 누락 파일을 성공 처리 |
| R02 | `missing_download_url_is_reported_as_success` | 빈 URL을 성공 처리 |
| R03 | `same_size_wrong_content_bypasses_hash_check` | 같은 크기 캐시를 검증 없이 사용 |
| R04 | `failed_zip_import_changes_existing_instance` | 실패 이후 기존 변경이 남음 |
| R05 | `vanilla_reimport_refuses_existing_instance` | 두 번째 가져오기 실패 |
| R08 | `same_name_from_another_source_overwrites_instance` | 다른 원본이 같은 인스턴스를 덮어씀 |
| R09 | `failed_history_write_still_marks_file_processed_in_memory` | 저장 실패와 메모리 성공 불일치 |
| R09 | `failure_retains_duplicate_success_history` | 같은 원본의 성공·실패 두 항목 |
| R10 | `neoforge_metadata_uses_different_uid_from_prism` | 잘못된 NeoForge UID 생성 |
| R11 | `partial_java_file_is_accepted_as_installed` | 가짜 Java 파일을 정상 캐시로 수용 |
| R14 | `override_result_depends_on_archive_entry_order` | client 내용을 base가 덮어씀 |
| R20 | `unsupported_manifest_is_accepted` | 미지원 형식·게임·빈 의존성을 성공 처리 |
| R21 | `minecraft_26_1_is_assigned_java_21` | 26.1의 요구 Java를 21로 반환 |

Node 진단 출력:

```text
R16 reproduced: a previous job timer hides the next active job.
R17 verified from generated ACL: window.close is not permitted.
```

`review-ui.cjs`는 실제 `src/main.js`를 최소 DOM·Tauri·timer mock으로 실행한다. 실제 WebView 렌더링이나 네이티브 창 클릭 검증은 아니다. R17의 숨겨진 창 재열기는 코드 분석 결과이며 이 스크립트는 닫기 ACL만 검사한다.

## 한계

- 실제 앱을 시작하지 않아 실제 Prism 프로세스, 사용자 설정·인스턴스, Java 설치 경로를 변경하지 않았다.
- 프로세스 조회 실패, 이력 동시 저장 경쟁, 실제 감시 타이밍, 업데이트 동시 설치는 정적 분석으로 확인했다.
- 제품 수정 단계에는 결함을 거부하는 회귀 테스트와 격리된 Windows 통합 검증이 필요하다.
