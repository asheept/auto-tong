# auto-tong 전체 코드 점검

## 요약

- 기준: 2026-09-30, `master` / `6463876`, 앱 버전 0.2.9.
- 결과: **수정 대상 21건 — P1 9건, P2 12건**. P1은 데이터·실행 안전 또는 핵심 설치 기능에 직접 영향을 주므로 우선 수정할 항목이다. P2는 특정 조건의 기능·상태·운영 신뢰성 문제다. 사고 발생 횟수나 CVSS 점수는 아니다.
- 우선순위: 경로 검증 → 원본 식별·이력 → 다운로드·실행 환경 → 설치 복구 → 작업 제어·화면 → 업데이트·배포.
- 산출물: [제안서](../../changes/harden-auto-tong-import-workflow/proposal.md), [설계](../../changes/harden-auto-tong-import-workflow/design.md), [52개 구현 작업](../../changes/harden-auto-tong-import-workflow/tasks.md), 요구사항 6개 영역.
- 현재 상태는 점검·계획 완료다. 제품 동작은 수정하지 않았으며 구현 작업은 모두 미완료다.

## 범위와 현재 구조

| 영역 | 확인 대상 |
| --- | --- |
| 앱 수명·트레이·업데이트 | `lib.rs`, `main.rs`, `tray.rs` |
| 설정·폴더 감시·작업 실행 | `config.rs`, `watcher.rs` |
| 성공·실패 기록 | `tracker.rs` |
| Prism ZIP·바닐라 ZIP·MRPACK | `prismlauncher.rs`, `mrpack.rs`, `zip_util.rs` |
| Java·런처 프로세스 | `java.rs`, Prism 프로세스 함수 |
| 화면 | `src/index.html`, `main.js`, `style.css`, `update-blocked.html` |
| 빌드·배포 | Cargo/npm 잠금 파일, Tauri 설정·권한, release workflow, 진단 스크립트 |

현재 흐름은 파일 감지 → 태그·수정 시간 확인 → 실제 인스턴스에 추출/다운로드 → Java 설정 → 성공 이력 → 런처 새로고침이다. 중간 실패를 되돌리는 단계와 작업별 일관된 상태 계약이 없다.

## 검증 결과

| 검증 | 결과 | 해석 |
| --- | --- | --- |
| MSVC `cargo test --locked` | 기존 단위 테스트 4개 통과 | ZIP 이름 디코딩만 검증; main/doc 테스트 0개 |
| 별도 Rust 진단 harness | 19개 통과: 진단 15개 + 기존 4개 | **현재 결함 재현 성공이며 수정 완료가 아님** |
| 실제 `main.js`의 Node VM 실행 | R16 재현 | 이전 타이머가 다음 작업을 숨김 |
| 생성된 Tauri ACL 검사 | R17 권한 누락 확인 | 네이티브 창 클릭 시험은 수행하지 않음 |
| `node --check src/main.js` | 통과 | 구문 검사 |
| package/Tauri/capability JSON 3개 | 파싱 통과 | 설정 문법 검사 |
| OpenSpec change strict 검증 | 통과 | 계획 문서 구조 검증 |

기본 Rust GNU toolchain은 `dlltool` 부재로 실패했으나 설치된 MSVC toolchain과 Visual Studio BuildTools로 빌드·테스트에 성공했다. 컴파일러 경고 6개는 남아 있다. 정확한 실행 명령과 진단은 [verification 안내](verification/README.md)에 있다.

테스트는 임시 디렉터리와 loopback HTTP를 사용했다. 실제 앱 시작, 실제 Prism 종료·실행, 게임 실행, 사용자 인스턴스 변경, 설치 프로그램·서명 업데이트, 화면 렌더링 검증은 수행하지 않았다. 의존성 취약점 공지 전체를 감사한 결과도 아니다.

## 우선순위와 작업 연결

| ID | 우선순위 | 문제 | 확인 방식 | 구현 작업 |
| --- | --- | --- | --- | --- |
| R01 | P1 | 설치 범위 밖 경로에 쓰기 가능 | Rust 2개 재현 | 2.1–2.3 |
| R02 | P1 | 다운로드 누락·404를 성공 처리 | Rust 2개 재현 | 4.1 |
| R03 | P1 | 같은 크기 캐시의 해시 검사 생략 | Rust 재현 | 4.2 |
| R04 | P1 | 설치 실패 후 기존 변경이 남음 | Rust 재현 | 6.1–6.4 |
| R06 | P1 | 스캔 debounce가 설정 명령 폐기 | 정적 제어 흐름 | 7.1–7.2 |
| R08 | P1 | 다른 원본이 같은 대상·이력으로 충돌 | 덮어쓰기 재현·이력 정적 확인 | 3.1–3.2, 7.4 |
| R10 | P1 | NeoForge component UID 오류 | 생성 결과·공식 자료 대조 | 5.2 |
| R12 | P1 | 프로세스 조회 실패를 미실행으로 취급 | 정적 제어 흐름 | 5.5–5.6, 6.6 |
| R21 | P1 | Minecraft 26.1에 Java 21 지정 | 반환값·공식 자료 대조 | 5.3 |
| R05 | P2 | 바닐라 재가져오기 실패 | Rust 재현 | 6.5 |
| R07 | P2 | 감시 폴더 교체 최대 한 시간 지연 | 정적 제어 흐름 | 7.3 |
| R09 | P2 | 이력 저장과 현재 상태 불일치 | 저장 실패·중복 상태 재현 | 3.3–3.5, 6.4 |
| R11 | P2 | 불완전한 Java를 정상 캐시로 수용 | 가짜 파일 재현 | 5.4, 6.1 |
| R13 | P2 | portable보다 기본 폴더 우선 | 정적 조건 분기 | 5.1 |
| R14 | P2 | overrides 결과가 ZIP 순서에 의존 | Rust 재현 | 4.4 |
| R15 | P2 | 네트워크 대기가 제어 처리를 막음 | 앱·reqwest 소스 | 4.3, 4.5, 7.1 |
| R16 | P2 | 진행 표시 타이머·완료 시점 오류 | JS 재현·발행 위치 | 8.1–8.2 |
| R17 | P2 | 안내 창 닫기 권한·재표시 누락 | ACL·창 수명 코드 | 8.5 |
| R18 | P2 | 실패 원인과 업데이트 안내 불일치 | 코드·HTML | 8.4 |
| R19 | P2 | 자동·수동 updater 중복 실행 가능 | 정적 제어 흐름 | 8.3 |
| R20 | P2 | 미지원 팩의 성공 처리 | MRPACK 재현·ZIP 분기 | 2.4 |

## 상세 발견 사항

### R01 · P1 · 출력 경로의 설치 범위 검증 누락

- 위치: [mrpack.rs](../../../src-tauri/src/mrpack.rs) 175행, [prismlauncher.rs](../../../src-tauri/src/prismlauncher.rs) 173·726행, [java.rs](../../../src-tauri/src/java.rs) 280행.
- 조건·영향: MRPACK 파일 경로는 검사 없이 `join` 후 기록한다. 다른 추출기의 `..` 문자열·`is_absolute()` 검사는 Windows `\name` 같은 rooted 경로를 막지 못한다. 앱 권한 범위에서 설치 밖 파일 변경이 가능하다.
- 증거: 임시 인스턴스의 `.minecraft` 밖에 `../escaped.txt`가 기록됐다. rooted 경로는 실제 루트에 쓰지 않고 경로 연산만으로 우회를 확인했다.
- 조치·완료 기준: 공통 component·링크 검증을 모든 쓰기에 적용한다. 비정상 경로는 거부되고 기존 설치와 범위 밖 파일은 변경되지 않아야 한다. [Modrinth 공식 형식 문서](https://support.modrinth.com/en/articles/8802351-modrinth-modpack-format-mrpack)도 인스턴스 밖 경로 방지를 명시한다.

### R02 · P1 · 다운로드 실패를 성공으로 반환

- 위치: [mrpack.rs](../../../src-tauri/src/mrpack.rs) 167·201·236행, [watcher.rs](../../../src-tauri/src/watcher.rs) 263행.
- 조건·영향: URL 누락과 HTTP 오류를 `continue`로 건너뛴 뒤 메타데이터와 성공 결과를 만든다. 누락된 팩이 성공 이력에 남아 자동 재시도에서도 제외된다.
- 증거: 빈 URL과 loopback 서버의 404 각각에서 설치 성공과 모드 파일 누락을 확인했다.
- 조치·완료 기준: 선택된 필수 파일을 모두 검증한 경우에만 성공한다. 모든 다운로드 후보가 실패하면 원인과 재시도를 제공하고 성공 이력을 남기지 않는다.

### R03 · P1 · 같은 크기 파일의 해시 검증 생략

- 위치: [mrpack.rs](../../../src-tauri/src/mrpack.rs) 181행.
- 조건·영향: 기존 파일 크기가 `fileSize`와 같으면 해시 검증 전에 건너뛴다. 손상되거나 다른 내용의 모드가 남는다.
- 증거: 같은 길이의 잘못된 바이트를 기록해도 다운로드 없이 성공했다.
- 조치·완료 기준: 캐시도 해시를 검증하고 불일치하면 재확보 또는 실패 처리한다.

### R04 · P1 · 실패한 설치가 기존 인스턴스를 변경

- 위치: [prismlauncher.rs](../../../src-tauri/src/prismlauncher.rs) 693·742행, [mrpack.rs](../../../src-tauri/src/mrpack.rs) 89·127·226행.
- 조건·영향: 실제 인스턴스에 순차 덮어쓰기하므로 뒤쪽 파일에서 실패해도 앞선 변경이 남는다.
- 증거: 첫 파일 덮어쓰기 후 파일/디렉터리 충돌을 발생시킨 ZIP에서 오류 반환 후 기존 파일의 변경을 확인했다.
- 조치·완료 기준: staging, 사용자 파일 보존, journal 확정·복구를 도입한다. 준비 단계 실패 시 기존 인스턴스 해시가 유지되어야 한다.

### R05 · P2 · 바닐라 재가져오기 거부

- 위치: [lib.rs](../../../src-tauri/src/lib.rs) 144·161행, [prismlauncher.rs](../../../src-tauri/src/prismlauncher.rs) 124행.
- 조건·영향: 재가져오기는 이력을 지우고 스캔만 요청하지만 바닐라 importer는 기존 `instance.cfg`가 있으면 실패한다.
- 증거: 같은 fixture의 첫 설치는 성공, 두 번째 설치는 실패했다.
- 조치·완료 기준: 재가져오기 의도를 작업으로 전달하고 사용자 데이터를 보존하는 교체를 수행한다. 성공 이력 삭제만으로 재설치를 표현하지 않는다.

### R06 · P1 · 설정 변경 명령의 유실

- 위치: [watcher.rs](../../../src-tauri/src/watcher.rs) 115·118행, [lib.rs](../../../src-tauri/src/lib.rs) 102행.
- 조건·영향: `CheckNow` 뒤 `while rx.try_recv().is_ok() {}`가 `UpdateConfig`까지 버린다. 디스크 저장은 완료돼도 실행 중 설정은 이전 상태로 남을 수 있다.
- 확인: 코드 제어 흐름으로 확인했으며 실제 watcher 타이밍 실험은 수행하지 않았다.
- 조치·완료 기준: 스캔 신호만 합치고 설정은 revision으로 적용 확인한다. 교차 명령 fixture에서 유실이 없어야 한다.

### R07 · P2 · 이벤트 감시 폴더 전환 지연

- 위치: [watcher.rs](../../../src-tauri/src/watcher.rs) 45·84행.
- 조건·영향: 감시 thread가 등록 후 3,600초 잠든다. 설정을 변경해도 그동안 이벤트는 이전 폴더를 감시하며 새 폴더는 폴링에 의존한다.
- 조치·완료 기준: 설정 적용 시 즉시 unwatch/watch하고 실패 시 폴링 대체 상태를 표시한다.

### R08 · P1 · 원본·대상 이름과 이력의 충돌

- 위치: [watcher.rs](../../../src-tauri/src/watcher.rs) 174·181·196행, [prismlauncher.rs](../../../src-tauri/src/prismlauncher.rs) 641행.
- 조건·영향: 대상 이름이 파일 stem뿐이라 `a/pack.zip`과 `b/pack.zip`이 충돌한다. 이력은 루트 없는 상대 경로와 초 단위 수정 시간뿐이어서 루트 변경·같은 시각의 내용 변경도 놓칠 수 있다.
- 증거: 서로 다른 두 `pack.zip`이 모두 성공하며 같은 인스턴스가 두 번째 내용으로 바뀌었다. 이력 식별 문제는 정적 확인이다.
- 조치·완료 기준: 루트+상대 경로 원본 ID, 고유 인스턴스 매핑, 내용 fingerprint를 사용한다. 이전 이력의 모호한 매핑은 자동 확정하지 않는다.

### R09 · P2 · 이력 저장과 메모리 상태 불일치

- 위치: [tracker.rs](../../../src-tauri/src/tracker.rs) 28·64·76·126·142행, [watcher.rs](../../../src-tauri/src/watcher.rs) 263·274행.
- 조건·영향: 메모리를 먼저 바꾸고 lock 해제 후 공유 `.json.tmp`에 저장한다. 저장 실패를 watcher가 무시하며 동시 저장은 충돌·오래된 snapshot 덮어쓰기가 가능하다. 실패 기록 시 성공 map도 남는다. 읽기·파싱 실패를 빈 이력으로 바꾸어 불필요한 재설치도 가능하다.
- 증거: 저장할 부모 폴더가 없으면 저장은 실패하지만 `needs_import`는 false다. 성공 후 실패를 기록하면 같은 원본이 두 항목으로 표시된다. 동시 저장 경쟁 자체를 강제 재현하지는 않았다.
- 조치·완료 기준: 지속 저장까지 직렬화하고 저장 성공 후 메모리를 반영한다. 한 원본의 설치와 최근 시도를 구분하며 백업·이전·복구를 검증한다.

### R10 · P1 · NeoForge component UID 오류

- 위치: [mrpack.rs](../../../src-tauri/src/mrpack.rs) 294행; 같은 프로젝트의 [prismlauncher.rs](../../../src-tauri/src/prismlauncher.rs) 535행과 다르다.
- 조건·영향: `net.neoforged.neoforge`를 생성하나 Prism의 UID는 `net.neoforged`다. 정상 component 조회에 맞지 않는 메타데이터가 생성된다. [Prism 공식 목록](https://raw.githubusercontent.com/PrismLauncher/meta-launcher/master/net.neoforged/index.json).
- 증거: NeoForge MRPACK의 생성 JSON에서 잘못된 UID를 확인했다. 실제 게임 실행은 하지 않았다.
- 조치·완료 기준: importer의 로더 매핑을 공통화하고 공식 fixture와 대조한다.

### R11 · P2 · 불완전한 Java를 정상 런타임으로 수용

- 위치: [java.rs](../../../src-tauri/src/java.rs) 77·85·212행, [watcher.rs](../../../src-tauri/src/watcher.rs) 250행.
- 조건·영향: 최종 Java 폴더에 직접 추출하고 실행 파일 존재만 검사한다. 중단 후 남은 파일도 캐시로 수용하고 Java 준비 실패도 경고 뒤 전체 성공으로 처리한다.
- 증거: 실행할 수 없는 문자열 파일 `javaw.exe`를 `ensure_java`가 정상 반환했다.
- 조치·완료 기준: 임시 설치, checksum, 실행 버전·아키텍처를 검증하고 실패한 Java 준비를 전체 완료와 구분한다.

### R12 · P1 · 프로세스 조회 실패를 안전한 종료 조건으로 취급

- 위치: [prismlauncher.rs](../../../src-tauri/src/prismlauncher.rs) 778·810·846·867·887행.
- 조건·영향: Java 자식 조회 실행 실패는 false가 되고 조회 종료 코드도 무시한다. 이때 게임이 없다고 판단해 런처에 `taskkill /F`를 진행할 수 있다. 종료·재실행 오류도 무시한다. exe 경로를 PowerShell 작은따옴표 문자열에 넣으므로 작은따옴표 경로의 조회도 깨진다.
- 확인: 정적 분석이며 실제 프로세스를 종료하지 않았다. 현재 명령에 `/T`는 없으므로 게임 프로세스 트리까지 종료된다고 단정하지 않는다.
- 조치·완료 기준: 조회 오류를 별도 상태로 두고 종료·확정을 보류한다. 경로와 코드를 분리하고 종료/시작 결과를 검사한다.

### R13 · P2 · portable 선택과 다른 데이터 위치 사용

- 위치: [prismlauncher.rs](../../../src-tauri/src/prismlauncher.rs) 598행.
- 조건·영향: 기본 `%APPDATA%/PrismLauncher/instances`가 존재하면 선택한 portable exe 위치보다 먼저 반환한다. 두 설치 공존 시 다른 런처에 팩을 설치할 수 있다.
- 조치·완료 기준: exe와 데이터 위치를 함께 확정하고 공존 fixture에서 선택한 위치만 변경되는지 확인한다.

### R14 · P2 · overrides 결과가 ZIP 순서에 의존

- 위치: [mrpack.rs](../../../src-tauri/src/mrpack.rs) 92·102·139행.
- 조건·영향: 두 overrides 경로를 한 번의 순회로 처리해 뒤 엔트리가 이긴다. 다운로드도 overlays 뒤에 수행한다.
- 증거: client 파일 뒤에 base 파일을 넣으면 base가 최종 내용이었다. 공식 형식은 client overrides를 공통 overrides 다음 계층으로 취급한다. [Modrinth 공식 형식 문서](https://support.modrinth.com/en/articles/8802351-modrinth-modpack-format-mrpack).
- 조치·완료 기준: 다운로드→공통→client 순으로 적용하고 ZIP 엔트리 순서를 바꿔도 결과가 같아야 한다.

### R15 · P2 · 네트워크 대기로 제어 루프 정지

- 위치: [mrpack.rs](../../../src-tauri/src/mrpack.rs) 153행, [java.rs](../../../src-tauri/src/java.rs) 185행, [watcher.rs](../../../src-tauri/src/watcher.rs) 98·112행.
- 조건·영향: client에 요청·연결·읽기 timeout을 지정하지 않고 `bytes()`로 응답 전체를 적재한다. 설치가 끝나야 명령을 다시 받으므로 멈춘 응답은 설정 처리를 지연시키고 큰 응답은 메모리를 소비한다.
- 확인: 로컬 reqwest 0.12.28 소스에서 관련 timeout 기본값 `None`을 대조했다. 무한 대기 실험은 실행하지 않았다.
- 조치·완료 기준: 유한 timeout·재시도·전송량과 스트리밍을 적용하고 제어 루프를 분리한다. 축소된 제한값의 fixture로 종료 시간을 검증한다.

### R16 · P2 · 이전 타이머와 이른 완료 표시

- 위치: [src/main.js](../../../src/main.js) 119·132행, [watcher.rs](../../../src-tauri/src/watcher.rs) 243·262행.
- 조건·영향: 다음 비종료 이벤트에서 이전 hide timer를 취소하지 않는다. Java 준비 전에 100% 이벤트를 보내 후속 단계가 완료처럼 처리되며 이력 저장보다 화면 갱신이 먼저 일어난다.
- 증거: 실제 JS에 첫 작업 완료→다음 작업 10%→이전 타이머 실행 순으로 전달하자 진행 창이 숨겨졌다.
- 조치·완료 기준: 작업 ID·종료 상태를 사용하고 Java·이력 저장 이후 전체 완료를 표시한다.

### R17 · P2 · 안내 창 닫기 권한·재표시 누락

- 위치: [update-blocked.html](../../../src/update-blocked.html) 118행, [default.json](../../../src-tauri/capabilities/default.json), [lib.rs](../../../src-tauri/src/lib.rs) 33·353행.
- 조건·영향: 커스텀 닫기 버튼의 `window.close()` 권한이 없다. 전역 CloseRequested는 창을 숨기며 기존 안내 창 재사용은 focus만 하므로 숨겨진 창이 다시 표시되지 않을 수 있다. 제목 표시줄 닫기까지 불가능하다는 뜻은 아니다.
- 증거: 생성 ACL에서 `core:window:allow-close`가 기본 권한에 포함되지 않음을 확인했다. [Tauri 창 API](https://v2.tauri.app/reference/javascript/api/namespacewindow/), [기본 권한](https://v2.tauri.app/reference/acl/core-permissions/).
- 조치·완료 기준: 최소 권한 또는 전용 명령과 show/focus 처리를 추가하고 실제 창 닫기·재열기를 검증한다.

### R18 · P2 · 업데이트 실패 원인과 안내 불일치

- 위치: [lib.rs](../../../src-tauri/src/lib.rs) 22행, [update-blocked.html](../../../src/update-blocked.html) 86·90행.
- 조건·영향: 오류 종류와 관계없이 두 번 실패하면 보안 차단 안내와 Smart App Control 해제를 제시한다. 네트워크·서명·디스크 오류에도 원인과 무관한 조치를 안내한다.
- 조치·완료 기준: 확인 가능한 오류 범주·원문 요약·재시도·공식 수동 다운로드를 제공한다. 횟수로 원인을 판단하지 않는다.

### R19 · P2 · updater 설치 중복 실행 가능

- 위치: [lib.rs](../../../src-tauri/src/lib.rs) 175·189·319행, [src/main.js](../../../src/main.js) 156행.
- 조건·영향: 수동 검사는 설치 task를 시작한 뒤 먼저 반환해 버튼이 다시 활성화된다. 자동 updater도 별도 경로이며 공유 lock이 없다.
- 조치·완료 기준: 자동·수동 요청이 단일 작업 상태를 사용하게 한다. 동시 요청 시 설치 호출은 한 번이어야 한다.

### R20 · P2 · 미지원 팩의 성공 처리

- 위치: [watcher.rs](../../../src-tauri/src/watcher.rs) 233행, [mrpack.rs](../../../src-tauri/src/mrpack.rs) 13·14·83행, [java.rs](../../../src-tauri/src/java.rs) 332행.
- 조건·영향: 알 수 없는 ZIP도 `Ok(_)`로 추출한다. MRPACK의 형식 버전·게임·Minecraft 의존성을 검증하지 않아 실행 정보 없는 인스턴스를 만들 수 있다.
- 증거: `formatVersion=999`, 다른 게임, 빈 dependencies인 MRPACK이 성공하고 빈 components를 생성했다.
- 조치·완료 기준: 실제 인스턴스를 생성하기 전에 지원 형식과 필수 실행 정보를 검증한다.

### R21 · P1 · Minecraft 26.1의 Java 요구사항 오판

- 위치: [java.rs](../../../src-tauri/src/java.rs) 43·349행.
- 조건·영향: 1.x 구간 이외에는 Java 21을 반환한다. 26.1도 Java 21 경로를 지정하지만 공식 `compatibleJavaMajors`는 `[25]`다. [Prism Minecraft 26.1 메타데이터](https://raw.githubusercontent.com/PrismLauncher/meta-launcher/master/net.minecraft/26.1.json).
- 증거: 현재 함수에 `26.1`을 입력하면 21을 반환했다. 실제 게임 실행은 하지 않았다.
- 조치·완료 기준: 검증된 버전별 메타데이터·캐시로 요구사항을 해석하고 26.1→25 및 미확인 버전의 실패를 검증한다.

## 추가 품질 개선

위 21건과 별도로 다음 유지보수 항목을 작업 9번에 묶었다.

- 기존 테스트는 이름 디코딩 4개뿐이다. 설치·이력·제어·업데이트 실패 fixture가 필요하다.
- release workflow는 태그 빌드·배포만 있고 PR 테스트가 없으며 `npm install`을 사용한다. 잠금 설치와 Windows 검증을 배포 전에 수행하도록 정리한다.
- npm 이름 `autu-tong`, 버전 1.0.0은 Cargo/Tauri 0.2.9와 다르다. 버전 기준을 통일한다.
- 경고 6개: Java `image_type`, MRPACK `mc_ver`, 미사용 format/game/summary 필드, sha1, server, MC 버전 함수. 검증에 필요한 필드는 실제 검증에 사용하고 나머지 의도를 정리한다.
- `check-prism.ps1`은 특정 사용자의 절대 경로와 0.2.9 동작에 맞지 않는 재실행 안내를 포함한다.
- `lib.rs` 224행에서 단일 실행 판정보다 먼저 로그를 `File::create`하여 중복 실행으로 기존 로그를 지울 수 있다.
- 빈 태그 필터와 prerelease 버전 비교는 각각 작업 7.5, 8.4에 포함했다.

## 변경 범위와 후속 기준

이번 작업은 `openspec/` 문서·진단과 CLI가 생성한 `.agents/skills/`를 추가했다. 제품 소스·잠금 파일·사용자 설정·실제 Prism 인스턴스는 수정하지 않았다. 테스트 빌드 캐시는 생성됐다.

작업 전부터 Git 수정 상태였던 `src-tauri/Cargo.toml`, `src-tauri/gen/schemas/desktop-schema.json`, `src-tauri/gen/schemas/windows-schema.json`은 보존했다. 최종 tracked diff에는 내용 차이가 없고 Git은 LF/CRLF 경고를 출력한다. 커밋·배포·archive는 수행하지 않았다.

OpenSpec `isPlanningComplete=true`는 문서 준비 완료를 뜻한다. 실제 구현 완료는 [작업 목록](../../changes/harden-auto-tong-import-workflow/tasks.md)의 개별 검증과 격리된 Windows 통합 검증이 끝난 뒤 판정한다.
