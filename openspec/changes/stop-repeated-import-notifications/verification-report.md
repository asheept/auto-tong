# 2026-10-08 로컬 검증 결과

기준: master 55847ed, Auto-Tong 0.3.0. 수정 브랜치: fix/import-notification-loop.
환경: Windows, Node.js 22.20.0, Rust MSVC stable 1.97.1. 테스트 및 빌드는 메인 에이전트가 직접 실행했다.

## 통과한 검증

- npm ci: 잠금 의존성 설치 성공.
- node --check src/main.js: 성공.
- Node 회귀 테스트 8개: 성공, 실패 0개.
- node scripts/check-version.js: 0.3.0 버전 일치.
- cargo +stable-x86_64-pc-windows-msvc test --locked: Rust 테스트 100개 성공, 실패 0개. main/doc test 대상도 성공.
- cargo +stable-x86_64-pc-windows-msvc fmt --all -- --check: 성공.
- cargo +stable-x86_64-pc-windows-msvc clippy --all-targets --locked -- -D warnings: 성공.
- cargo +stable-x86_64-pc-windows-msvc build --locked: Windows 개발 빌드 성공.
- git diff --check: 성공. 빌드가 다시 쓴 생성 schema의 줄바꿈 변경은 원복했다.

## 핵심 회귀 근거

- 손상된 실제 journal을 복구 함수에 반복 전달하고 원본 목록이 비어도 같은 복구 오류 알림 결정이 한 번만 발생함을 확인했다. 복구 후 같은 오류 재발은 다시 알린다.
- 알림 상태는 원본별로 구분되며 새 콘텐츠는 오류를 다시 알린다. 이때 설치 확정 보류 보호는 해제하지 않는다.
- 런처 사전 검사는 게임/조회 실패 동안 준비 작업을 시작하지 않고, 조건이 정상화되면 진행한다. 설치 확정 중 보류 뒤에는 런처 종료까지 기다린다.
- 실패 저장 장애의 최종 단계가 RecoveryRequired/Storage이며 보통 실패 상태로 덮이지 않는다.
- 실패/취소 저장 장애 후 같은 콘텐츠가 다시 실행되지 않는다. 기존 재시도 의도가 있어도 보호가 우선하고, 저장 실패한 수동 재시도는 보호를 해제하지 않는다. 저장 정상화 후 수동 재시도는 허용한다.
- 내용은 같고 수정 시각만 바뀌어도 30초·120초 및 총 3회 한도가 유지되며 재시작 후에도 한도가 유지된다.
- 최초 journal 쓰기·공개 실패와 파일명 충돌에서 기존 인스턴스·staging·기존 기록을 보존하고 새 불완전 journal과 백업을 남기지 않는다.
- 기존 세 형식 가져오기/재가져오기, 세이브 보존, 중단 복구, Windows 파일 잠금 등 회귀 테스트도 통과했다.

## 결과와 한계

개발 빌드: src-tauri/target/debug/auto-tong.exe. 설치 파일이나 서명된 updater 릴리스를 만들거나 게시하지 않았다. 앱 본체를 실제 사용자 설정으로 실행하지 않았고 실제 Windows 알림 창 및 사용자 PC의 증상은 실환경에서 확인하지 않았다. 손상 기록·권한·저장 공간 같은 최초 오류 원인은 그대로 드러내며, 이번 수정은 확인된 반복 처리와 알림 경로를 다룬다.

실행 중 알림 및 저장 장애 보호는 재시작 시 초기화된다. 실제 운영 데이터나 기존 설치 기록을 삭제하지 않았다. 실행 로그는 %TEMP%/auto-tong-fix-verification-20261008 아래 rust-tests.log, clippy.log, windows-build.log에 있다.
개발 실행 파일 SHA-256: 1A46FB3EAFCE2600A159E78402481545BE73D23B1AF98C91C25AB8D31410F865

## 0.3.1 배포 준비

후속 배포 요청으로 Cargo·Tauri·npm 및 두 잠금 파일의 제품 버전만 0.3.1로 갱신했다. 의존성 및 위에서 검증한 제품 코드는 같다. 0.3.1 태그 일치 검사, Node 테스트 8개, cargo metadata --locked --offline --no-deps 검사가 통과했다. 원격 PR의 Windows CI와 Release workflow 결과, 공개 아티팩트 검증은 GitHub PR/릴리스에 별도로 기록한다.