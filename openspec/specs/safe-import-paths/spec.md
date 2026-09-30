# safe-import-paths Specification

## Purpose

외부 ZIP과 MRPACK을 가져올 때 지원하는 형식과 모든 출력 경로를 먼저 검증하여, 선택한 설치 위치 밖의 파일과 기존 사용자 데이터가 변경되는 일을 방지한다.

## Requirements

### Requirement: 출력 경로 범위 보장

시스템은 팩 파일, overrides, Java 압축의 모든 출력 경로가 승인된 설치 준비 디렉터리 내부에 있음을 보장해야 한다(SHALL). 부모 이동, 루트·드라이브 지정, UNC, 장치 경로, 대체 데이터 스트림, Windows 예약 이름, 링크를 통한 범위 이탈은 파일을 쓰기 전에 거부해야 한다(MUST).

#### Scenario: 매니페스트 경로 이탈

- **WHEN** 다운로드 경로가 `../outside.txt`, `C:outside.txt`, `\outside.txt` 또는 `file.txt:stream`이다
- **THEN** 해당 가져오기는 경로 오류로 실패하고 준비 디렉터리 밖의 파일과 기존 인스턴스는 변경되지 않는다

#### Scenario: 기존 링크와 연결된 출력 경로

- **WHEN** 출력 경로의 상위 디렉터리가 설치 범위 밖을 가리키는 심볼릭 링크 또는 junction이다
- **THEN** 시스템은 그 경로로 쓰지 않고 오류를 표시한다

#### Scenario: 한글과 중첩 경로

- **WHEN** 지원하는 인코딩으로 저장된 정상적인 한글 파일명과 상대 하위 경로가 들어 있다
- **THEN** 범위 검증을 통과한 파일은 이름을 보존하여 가져온다

### Requirement: 설치 전 형식 검증

시스템은 지원하는 Prism 인스턴스 ZIP, 바닐라 ZIP, Minecraft MRPACK만 설치해야 한다(SHALL). MRPACK의 형식 버전, 게임, Minecraft 의존성, 파일 해시와 다운로드 정의 및 생성될 실행 메타데이터를 검증해야 한다(MUST).

#### Scenario: 지원하지 않는 매니페스트

- **WHEN** MRPACK의 형식 버전이 999이거나 게임이 Minecraft가 아니거나 Minecraft 버전이 없다
- **THEN** 지원하지 않는 형식으로 실패하며 설치와 성공 이력을 만들지 않는다

#### Scenario: 알 수 없는 ZIP

- **WHEN** ZIP에 지원 형식을 판별하고 실행 정보를 구성할 자료가 없다
- **THEN** 임의의 Prism 인스턴스로 가져오지 않고 필요한 정보가 없음을 알린다

#### Scenario: 불완전한 다운로드 정의

- **WHEN** 설치 대상 파일에 유효한 해시 또는 다운로드 URL이 없다
- **THEN** 사전 검증에서 실패하며 기존 인스턴스를 변경하지 않는다
