# auto-tong 개발 안내

Windows에서 PrismLauncher의 모드팩을 동기화 폴더에서 자동으로 가져오는 Tauri 2 앱입니다. 코드 점검과 변경 계획은 [OpenSpec 안내](openspec/README.md)에 있습니다.

## 개발 환경

- Node.js 22와 npm
- Rust stable의 `stable-x86_64-pc-windows-msvc` toolchain
- Visual Studio 2022 BuildTools의 C++ 빌드 도구와 Windows SDK
- Tauri 2의 Windows 빌드 환경(WebView2 포함)

이 개발 PC에서는 기본 `stable-x86_64-pc-windows-gnu` toolchain에 `dlltool`이 없어 MSVC toolchain을 명시해야 합니다. PowerShell에서 아래 명령을 실행합니다.

```powershell
cd src-tauri
cargo +stable-x86_64-pc-windows-msvc test --locked
cargo +stable-x86_64-pc-windows-msvc fmt --all -- --check
cargo +stable-x86_64-pc-windows-msvc clippy --all-targets --locked -- -D warnings
cd ..
node --check src/main.js
node --test src/progress-state.test.js
node --test src/update-state.test.js
node --test scripts/check-version.test.js
node --test scripts/check-capabilities.test.js
npm run check:version
```

앱 실행과 패키징은 저장소 루트에서 `npm ci`, `npm run dev`, `npm run build`를 사용합니다. 앱을 실행하면 감시 폴더와 PrismLauncher에 실제 쓰기가 발생할 수 있으므로 기능 검증에는 별도 설정과 임시 Prism 데이터 폴더를 사용합니다.

## 작업 상태

[OpenSpec 구현 작업](openspec/changes/harden-auto-tong-import-workflow/tasks.md)의 체크박스는 검증이 끝난 항목만 완료로 표시합니다. 2026-09-30에 기록된 진단은 기존 오류 동작을 재현하는 자료이며, 수정 완료를 뜻하지 않습니다.

## MRPACK 다운로드 제한

운영 빌드는 HTTPS 다운로드와 HTTPS 리다이렉트만 허용합니다. 테스트 빌드의 로컬 fixture에는 `127.0.0.1` HTTP를 허용합니다. 사용자 정보가 포함된 URL은 거부합니다. 리다이렉트는 최대 5회, 연결 제한 시간은 10초, 데이터 읽기 대기는 30초, 요청 전체는 15분입니다. 파일 하나의 매니페스트 크기는 최대 512 MiB, 클라이언트 다운로드 대상 전체는 최대 4 GiB이며, 실제 수신량은 각 파일의 선언 크기를 넘을 수 없습니다. 오류는 해당 파일의 가져오기 실패로 표시합니다.

ZIP과 MRPACK 압축 파일은 대상 폴더를 만들기 전에 엔트리 20,000개, 파일 하나의 해제 크기 512 MiB, 전체 해제 크기 4 GiB 제한을 확인합니다. 초과하면 어떤 제한인지 오류에 표시합니다.

## Minecraft Java 요구사항

Java 버전은 [Prism의 Minecraft 버전 메타데이터](https://raw.githubusercontent.com/PrismLauncher/meta-launcher/master/net.minecraft/26.1.json)의 `compatibleJavaMajors`를 확인하고, UID·버전·요구 버전을 검증한 메타데이터만 캐시합니다. 네트워크 장애 시 확인된 1.16 이하, 1.17, 1.18–1.21 및 26.1 규칙을 사용합니다. Minecraft 26.1은 [공식 릴리스 노트](https://www.minecraft.net/en-us/article/minecraft-java-edition-26-1)에 따라 Java 25를 요구합니다. 알 수 없는 버전은 Java를 임의로 지정하지 않고 오류를 표시합니다.

Adoptium의 배포 메타데이터에서 SHA-256 체크섬을 확인하고, 다운로드한 압축 파일의 해시와 실행 파일 버전·아키텍처를 확인한 다음 Java 폴더를 교체합니다. Java 준비 실패는 가져오기 완료로 기록하지 않습니다. 버전을 판별할 수 없으면 팩의 Minecraft 버전 표기를 확인하고 네트워크 연결 후 이력의 **재다운로드**를 실행합니다. 다운로드 또는 Java 검증 실패면 네트워크 연결, Prism 데이터 폴더의 쓰기 권한과 남은 저장 공간을 확인한 뒤 재시도합니다.

## 가져오기 보류와 복구

가져오기는 Prism 데이터 폴더의 `instances` 밖에 있는 임시 폴더에서 준비합니다. 재가져오기에서는 이전 팩이 관리한 파일만 갱신하고 월드, 옵션, 스크린샷과 사용자가 추가한 파일을 보존합니다. 사용자가 수정한 파일과 새 팩 파일이 겹치면 자동 확정을 보류합니다. 수정본을 별도로 백업하고 충돌 파일을 확인한 뒤 재다운로드를 다시 실행하세요.

설치 중단 또는 이력 저장 실패 시 데이터 폴더에 `.auto-tong-journal-*`과 `.auto-tong-backup-*`이 남을 수 있습니다. 다음 스캔은 복구를 먼저 수행하고 그 스캔에서 새 설치를 시작하지 않습니다. 복구가 실패하면 가져오기를 중단하고 **설치 복구 필요** 알림을 보냅니다. `.auto-tong-recovery-*`에는 새 설치의 보존본이 있을 수 있으므로, 해당 폴더와 journal·backup을 임의로 삭제하기 전에 기존 인스턴스와 보존본을 확인하세요.

이력 v1을 처음 읽으면 `%APPDATA%/auto-tong/processed.v1.backup.json`에 원본을 보존하고 `processed.json`을 v2로 이전합니다. 이후 정상 저장 직전의 파일은 `processed.backup.json`에 보관합니다. 이력 JSON이 손상되면 유효한 백업에서 복구하고 손상본을 `processed.corrupt-*.json`으로 남깁니다. 백업까지 읽을 수 없으면 앱 시작을 중단하므로 자동 재가져오기가 발생하지 않습니다. 같은 이름의 이전 인스턴스가 둘 이상의 원본에 대응하면 자동 연결을 보류합니다.

이전 앱 버전으로 되돌릴 때는 앱과 PrismLauncher를 종료하고, 미완료 journal이 있으면 먼저 현재 버전으로 복구한 뒤 `processed.json`과 v2 백업을 별도로 보관합니다. 그 다음 `processed.v1.backup.json`을 `processed.json`으로 복사합니다. v1 백업이 없으면 v2 이력을 구버전 앱으로 읽히게 하지 말고 자동 감시를 중지한 상태에서 복구 대상을 확인하세요.

## PrismLauncher 데이터 위치

설정의 실행 파일 경로와 데이터 폴더는 같은 Prism 설치를 가리켜야 합니다. PrismLauncher에서 **폴더 → Launcher Root**를 열어 표시된 폴더를 데이터 폴더에 지정합니다. 설정을 비우면 실행 파일 옆의 `portable.txt`가 있는 portable 설치를 우선 확인하고, 기본 위치와 portable 위치가 함께 있으면 자동으로 선택하지 않고 데이터 폴더 지정을 요청합니다. [Prism 공식 데이터 위치 안내](https://prismlauncher.org/wiki/getting-started/data-location/)에 따르면 Windows 기본 위치는 `%APPDATA%/PrismLauncher`, portable은 실행 파일 폴더입니다. 설정한 폴더 안에 `instances`가 있어야 가져오기를 시작합니다.

## 버전과 배포

`src-tauri/Cargo.toml`, `src-tauri/tauri.conf.json`, `package.json`, `package-lock.json`의 제품 버전을 동일하게 유지합니다. npm 패키지명과 Cargo 패키지명은 `auto-tong`입니다. 버전을 바꾼 뒤 `npm install --package-lock-only`와 `cargo +stable-x86_64-pc-windows-msvc check`로 잠금 파일을 갱신하고, `npm run check:version`으로 확인합니다. 릴리스 태그는 정확히 `v<제품 버전>`이어야 하며 release workflow가 빌드 전에 검사합니다.

배포 전에는 PR 검증 workflow와 로컬 테스트를 확인합니다. 릴리스 workflow는 저장소의 `TAURI_SIGNING_PRIVATE_KEY` 및 `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` secret으로 updater 아티팩트를 서명합니다. secret이 없거나 서명이 실패하면 배포를 중단합니다. Windows 설치 파일의 별도 Authenticode 서명은 현재 workflow에 설정되어 있지 않습니다.

자동 업데이트가 실패할 때는 앱과 PrismLauncher를 종료하고 설정 파일 및 사용 중인 인스턴스를 백업한 뒤, 프로젝트의 GitHub 릴리스에서 해당 버전 설치 파일을 내려받아 수동 설치합니다. 이전 버전으로 되돌려야 하면 같은 백업을 유지하고 이전 릴리스 설치 파일로 수동 설치합니다. 앱은 시작할 때 자동 업데이트를 검사하므로, 문제 버전이 최신 릴리스인 동안에는 복구판이 배포될 때까지 네트워크를 연결하지 않은 상태에서 이전 버전을 실행합니다. 설치 파일과 백업을 삭제하기 전에 설정과 인스턴스가 정상인지 확인합니다.

업데이트 실패가 반복되면 원인을 네트워크·서명·권한·기타로 나누어 안내합니다. 네트워크 오류는 연결을 확인하고, 서명 오류는 배포본 검증 상태를 확인하며, 권한 오류는 앱 종료와 파일 잠금을 확인합니다. 실패 원인을 알 수 없으면 로그와 공식 릴리스를 확인합니다. 버전 비교는 prerelease 순서와 빌드 메타데이터를 반영합니다.

자동 및 수동 업데이트는 하나의 실행 상태를 사용합니다. 검사부터 다운로드·설치까지 다른 업데이트 요청은 중복 실행하지 않고, 설정 화면의 버튼은 실제 실행 상태가 끝날 때 다시 활성화됩니다.

## 설정 변경 중 작업

설정에 지정한 태그는 쉼표로 나눕니다. `@` 또는 공백만 입력된 태그는 저장 시 오류로 표시합니다. 저장 버튼은 revision을 저장하고 감시 폴더의 이벤트 감시 또는 주기 확인 대체 상태를 적용한 응답을 받은 뒤 완료를 표시합니다. 적용 응답이 없으면 설정이 저장되었더라도 완료라고 표시하지 않습니다. 이미 시작한 가져오기는 시작 시 설정으로 끝나며, 새 설정은 다음 스캔부터 적용합니다.

가져오기 중 **가져오기 취소**를 누르면 준비 중인 다운로드를 중단하고 임시 설치 폴더를 정리합니다. 설치 확정 단계가 시작된 뒤에는 취소를 받지 않습니다. 취소는 성공으로 기록하지 않으며, 같은 원본을 다시 가져오려면 이력에서 수동 재시도를 선택합니다. 다운로드 오류 등 실패한 원본은 내용이 바뀌지 않아도 30초와 120초 간격으로 최대 두 번 자동 재시도한 뒤 멈춥니다. 다시 시도하려면 이력의 해당 항목을 누르세요. 다운로드 대기 제한은 위의 MRPACK 다운로드 제한을 따릅니다.
