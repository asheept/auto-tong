# Auto-Tong 개발 가이드

## 릴리스 규칙
- 코드를 push할 때마다 `tauri.conf.json`의 `version`을 **0.0.1씩** 자동으로 올리고, 태그를 붙여서 릴리스한다.
- 예: `0.1.0` → `0.1.1` → `0.1.2` → ...
- 커밋 후 반드시: `git tag v{새버전}` → `git push origin master --tags`

## 빌드
- 로컬 빌드 시 서명 키 환경변수 필요:
  ```
  export TAURI_SIGNING_PRIVATE_KEY="$(cat ~/.tauri/auto-tong.key)"
  export TAURI_SIGNING_PRIVATE_KEY_PASSWORD=""
  ```
- GitHub Actions: `v*` 태그 push 시 자동 빌드 + 릴리스

## 프로젝트 구조
- `src-tauri/src/` — Rust 백엔드
- `src/` — HTML/CSS/JS 프론트엔드 (설정 창)
- `.github/workflows/release.yml` — CI/CD
- 빌드 타겟 디렉토리: `C:/autu-tong-target/` (한글 경로 문제 회피)
