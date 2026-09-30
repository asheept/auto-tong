# auto-tong OpenSpec

## 현재 상태

2026-09-30 전체 코드 점검을 바탕으로 가져오기·복구·감시·Prism·Java·업데이트 처리를 수정하고 실제 Windows·Prism·WebView2 통합 검증을 완료했다. 요구사항 21개와 시나리오 41개를 기준 명세 6개에 반영하고 변경을 archive했다. OpenSpec 1.13.2의 `spec-driven` schema와 한국어 산출물을 사용한다.

| 문서 | 내용 |
| --- | --- |
| [코드 점검 보고서](reviews/2026-09-30/code-review.md) | P1 9건·P2 12건, 코드 위치, 영향, 재현 결과 |
| [재현 안내](reviews/2026-09-30/verification/README.md) | 임시 fixture 기반 Rust/JS 진단 실행법과 한계 |
| [제안서](changes/archive/2026-09-30-harden-auto-tong-import-workflow/proposal.md) | 개선 이유·범위·영향 |
| [설계](changes/archive/2026-09-30-harden-auto-tong-import-workflow/design.md) | staging, journal, 이력 이전, 작업·업데이트 상태 |
| [구현 작업](changes/archive/2026-09-30-harden-auto-tong-import-workflow/tasks.md) | 52/52개 작업의 검증과 완료 내역 |
| [검증 기록](changes/archive/2026-09-30-harden-auto-tong-import-workflow/verification-report.md) | Rust 88개·JS 8개, 실제 Windows·WebView2·게임 보류 검증, GitHub CI 결과 |

## 요구사항

- [경로·형식 검증](specs/safe-import-paths/spec.md)
- [설치·다운로드 신뢰성](specs/reliable-modpack-install/spec.md)
- [감시·작업 제어](specs/import-orchestration/spec.md)
- [이력 보존](specs/durable-import-history/spec.md)
- [Prism·Java 연동](specs/launcher-runtime-integration/spec.md)
- [진행 표시·업데이트](specs/desktop-operation-status/spec.md)

기준 명세는 `openspec/specs/`에 있다. 제안·설계·작업·delta spec·검증 기록은 `changes/archive/2026-09-30-harden-auto-tong-import-workflow/`에 보존했다.

## 상태 확인과 검증

프로젝트 루트 PowerShell에서 실행한다. CLI는 npx로 버전을 고정했으며 제품 의존성에는 추가하지 않았다. 사용 방식은 [OpenSpec 공식 저장소](https://github.com/Fission-AI/OpenSpec)를 따른다.

```powershell
$env:OPENSPEC_TELEMETRY = '0'
npx.cmd --yes @fission-ai/openspec@1.13.2 list --json
npx.cmd --yes @fission-ai/openspec@1.13.2 validate --specs --strict
```

기준 명세 strict 검증과 [Windows CI](https://github.com/asheept/auto-tong/actions/runs/36656073469)는 통과했다. 보관한 `tasks.md`의 체크박스와 검증 기록에서 구현 완료 근거를 확인할 수 있다.

후속 변경은 새 OpenSpec change로 제안·설계·검증 내역을 기록한다. [초안 PR #1](https://github.com/asheept/auto-tong/pull/1)에서 이번 수정 내역을 검토할 수 있다.
