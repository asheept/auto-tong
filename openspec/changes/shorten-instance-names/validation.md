# 검증 결과

2026-10-26, Windows에서 `fix/short-instance-names` 브랜치로 검증했다. 기준 커밋은 `b2b1947`이며 앱 버전은 0.3.1을 유지한다.

## 동작과 검토

- 새 이름의 `@`를 보존하며 접미사를 기존 12자리에서 소문자 16진수 4자리로 줄였다. 원본 경로의 전체 SHA-256 식별자는 유지한다.
- 예시: `2610_뜰릿밸리4화_@everyone_v01-0b7d`. 실제 접미사는 원본 경로와 이미 사용 중인 이름에 따라 정해진다.
- 저장된 다른 매핑과 실제 인스턴스 위치의 모든 직접 하위 이름을 대소문자 구분 없이 확인한다. 충돌하면 다음 4자리 값을 선택하며 `ffff` 다음에는 `0000`으로 순환한다.
- 기존 매핑은 그대로 재사용한다. 디렉터리 확인 실패와 접미사 전체 소진은 오류로 처리하며 새 매핑을 게시하지 않는다.
- 코드 리뷰와 회귀 검사에서 충돌, 재시작 후 매핑 유지, 기존 파일 보존, 이전 12자리 매핑 유지, 순환 탐색, 읽기 실패 및 값 소진을 확인했다.

## 실행한 검사

- `node --check src/main.js`: 통과.
- `node --test src/progress-state.test.js src/update-state.test.js scripts/check-version.test.js scripts/check-capabilities.test.js`: 8개 통과.
- `npm run check:version`: 0.3.1 일치, 통과.
- `cargo +stable-x86_64-pc-windows-msvc fmt --all -- --check`: 통과.
- `cargo +stable-x86_64-pc-windows-msvc clippy --offline --all-targets --locked -- -D warnings`: 통과.
- `cargo +stable-x86_64-pc-windows-msvc test --offline --locked`: 107개 통과.
- `cargo +stable-x86_64-pc-windows-msvc build --offline --locked`: Windows 디버그 빌드 통과.
- `openspec validate shorten-instance-names --strict --no-interactive`: 통과.
- `git diff --check`: 통과.

기본 GNU 도구 체인에는 rustfmt가 없어 설치되어 있는 MSVC 도구 체인을 명시적으로 사용했다. 빌드가 만든 스키마 파일의 줄바꿈 변경은 내용 차이가 없음을 확인한 뒤 복원했다.

## 적용 범위

소스와 로컬 검증을 완료했다. 기존 설치 폴더 및 실제 사용자 이력은 변경하지 않았다. 실행 중인 앱 재시작, 설치본 교체, 커밋, 원격 푸시 및 릴리스 발행은 수행하지 않았다. 새 규칙은 이 코드로 만든 앱에서 새 매핑을 생성할 때 적용된다.
