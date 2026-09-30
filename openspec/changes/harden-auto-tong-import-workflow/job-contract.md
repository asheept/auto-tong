# 가져오기 진행 이벤트 계약

백엔드는 `import-progress` 이벤트로 [`ImportProgress`](../../../src-tauri/src/job.rs)를 보낸다. JSON 구조의 공통 예시는 [`import-progress.json`](../../../src/fixtures/import-progress.json)이다. `job_id`는 실행 중 증가하는 작업 번호이며, 화면은 더 작은 번호의 이벤트를 무시한다. `source_id`는 감시 루트와 상대 경로를 연결한 원본 식별 문자열이다. 이후 원본 식별자 정규화 작업(3.1)에서 이 문자열의 생성 규칙을 확정한다.

`phase`는 `preparing`, `downloading`, `extracting`, `java`, `recording`, `completed`, `failed` 중 하나다. `percent`는 현재 단계의 진행률이며 100만으로 완료를 뜻하지 않는다. `terminal`은 `completed` 또는 `failed`일 때만 true다. 실패 이벤트는 `error_category`, `error`, `retryable`을 함께 제공한다. 현재 오류 분류는 `storage`, `launcher`, `unknown`이며 세부 실패 분류는 8.4에서 확장한다.

성공 조건은 설치, Java 준비, 이력 저장이 모두 끝나고 `completed`가 발행되는 것이다. 현재 설치 commit과 Java 실패 처리에는 별도 미완료 작업(6.1–6.6, 8.2)이 있어, 이 계약의 성공 조건을 모두 충족하도록 후속 수정이 필요하다. 화면은 종료 이벤트 뒤 3초 후 진행 표시를 숨기며, 새 작업의 이벤트나 비종료 이벤트를 받으면 기존 타이머를 취소한다.
