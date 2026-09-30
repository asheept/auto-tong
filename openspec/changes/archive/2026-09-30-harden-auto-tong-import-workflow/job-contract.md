# 가져오기 진행 이벤트 계약

백엔드는 `import-progress` 이벤트로 [`ImportProgress`](../../../../src-tauri/src/job.rs)를 보낸다. JSON 구조의 공통 예시는 [`import-progress.json`](../../../../src/fixtures/import-progress.json)이다. `job_id`는 실행 중 증가하는 작업 번호이며, 화면은 더 작은 번호의 이벤트를 무시한다. 실제 `source_id`는 정규화한 감시 루트와 상대 경로의 SHA-256을 `source-` 뒤에 붙인 값이다. Windows에서는 구분자와 대소문자를 정규화한다. 예시 fixture의 식별 문자열은 JSON 필드 계약을 확인하기 위한 값이다.

`phase`는 `preparing`, `downloading`, `extracting`, `java`, `recording`, `deferred`, `recovery_required`, `completed`, `failed`, `cancelled` 중 하나다. `percent`는 현재 단계의 진행률이며 100만으로 완료를 뜻하지 않는다. `terminal`은 `completed`, `failed`, `deferred`, `recovery_required`, `cancelled`에서 true다. 실패 이벤트는 `error_category`, `error`, `retryable`을 함께 제공한다. 가져오기 오류 분류는 `storage`, `launcher`, `java`, `unknown`이다. updater의 오류 범주는 별도 계약이다.

성공 조건은 Java 검증, 설치 확정, 이력 저장이 모두 끝나고 `completed`가 발행되는 것이다. 게임 실행 또는 조회 실패는 `deferred`, 설치 확정 후 이력 저장 실패는 `recovery_required`로 표시하며 성공으로 기록하지 않는다. 런처 재실행만 실패한 경우는 확정된 설치를 유지하고 `completed`와 런처 경고를 함께 표시한다. 화면은 오류가 없는 종료 이벤트 뒤 3초 후 진행 표시를 숨긴다. 오류와 복구 필요 상태는 자동 타이머로 숨기지 않으며, 다음 작업이 시작하면 이전 타이머를 취소한다.
