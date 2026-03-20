# Auto-Tong 작업 목록

## 완료
- [x] Tauri v2 프로젝트 스캐폴드
- [x] 설정 관리 (config.rs) - Drive 경로, PrismLauncher 경로, 클라이언트 이름, 태그, 폴링 주기
- [x] 가져온 파일 추적 (tracker.rs / processed.json)
- [x] 폴더 폴링 감시 (watcher.rs) - @everyone, @이름, @태그 폴더
- [x] PrismLauncher --import 연동 (prismlauncher.rs)
- [x] 시스템 트레이 메뉴 (지금 확인, 내보내기 경로 복사, 설정, 종료)
- [x] 설정 창 UI (다크 테마, 한국어)
- [x] Windows 자동 시작 (tauri-plugin-autostart)
- [x] 첫 실행 시 설정 창 자동 열기
- [x] 빌드 및 exe 생성

## 미완료
- [ ] 첫 실행 시 기존 파일 처리 전략 (기존 파일은 건너뛰고 새 파일만 받기 옵션)
- [ ] 폴더 구조 세부 설정 (Drive 폴더 트리 디테일)
- [ ] 디자인 개선 (설정 창 UI 다듬기)
- [ ] 앱 아이콘 디자인 (현재 임시 파란색 아이콘)
- [ ] 내보내기(PUSH) 기능 개선 (현재는 경로 복사만)
- [ ] 서버 주소 재배포 (launch 명령 포함)
- [ ] notify crate 실시간 감시 추가 (현재 폴링만 사용)
- [ ] 에러 처리 개선 (Drive 폴더 없음, PrismLauncher 없음 등)
- [ ] 설정 변경 시 watcher 즉시 반영 확인
- [ ] 가져오기 이력 UI에서 날짜/시간 표시
