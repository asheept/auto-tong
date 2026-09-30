# auto-tong OpenSpec

## 현재 상태

2026-09-30 전체 코드 점검과 개선 계획을 작성했다. 제품 수정은 시작하지 않았다. OpenSpec 1.13.2의 `spec-driven` schema와 한국어 산출물을 사용한다.

| 문서 | 내용 |
| --- | --- |
| [코드 점검 보고서](reviews/2026-09-30/code-review.md) | P1 9건·P2 12건, 코드 위치, 영향, 재현 결과 |
| [재현 안내](reviews/2026-09-30/verification/README.md) | 임시 fixture 기반 Rust/JS 진단 실행법과 한계 |
| [제안서](changes/harden-auto-tong-import-workflow/proposal.md) | 개선 이유·범위·영향 |
| [설계](changes/harden-auto-tong-import-workflow/design.md) | staging, journal, 이력 이전, 작업·업데이트 상태 |
| [구현 작업](changes/harden-auto-tong-import-workflow/tasks.md) | 의존 순서와 확인 방법을 포함한 미완료 작업 52개 |

## 요구사항

- [경로·형식 검증](changes/harden-auto-tong-import-workflow/specs/safe-import-paths/spec.md)
- [설치·다운로드 신뢰성](changes/harden-auto-tong-import-workflow/specs/reliable-modpack-install/spec.md)
- [감시·작업 제어](changes/harden-auto-tong-import-workflow/specs/import-orchestration/spec.md)
- [이력 보존](changes/harden-auto-tong-import-workflow/specs/durable-import-history/spec.md)
- [Prism·Java 연동](changes/harden-auto-tong-import-workflow/specs/launcher-runtime-integration/spec.md)
- [진행 표시·업데이트](changes/harden-auto-tong-import-workflow/specs/desktop-operation-status/spec.md)

기존 기준 명세가 없으므로 신규 요구사항은 change의 delta spec에만 작성했다. 아직 `openspec/specs/`에 구현 완료 명세로 반영하거나 archive하지 않았다.

## 상태 확인과 검증

프로젝트 루트 PowerShell에서 실행한다. CLI는 npx로 버전을 고정했으며 제품 의존성에는 추가하지 않았다. 사용 방식은 [OpenSpec 공식 저장소](https://github.com/Fission-AI/OpenSpec)를 따른다.

```powershell
$env:OPENSPEC_TELEMETRY = '0'
npx.cmd --yes @fission-ai/openspec@1.13.2 status --change harden-auto-tong-import-workflow
npx.cmd --yes @fission-ai/openspec@1.13.2 validate harden-auto-tong-import-workflow --strict
```

현재 strict 검증은 통과했다. CLI의 planning 완료 표시는 문서가 준비되었다는 뜻이다. 실제 구현 진행률은 `tasks.md`의 체크박스를 기준으로 한다.

구현을 시작할 때는 이 change에 `openspec-apply-change`를 사용하거나 적용을 요청한다. 수정한 항목만 검증 근거와 함께 완료 표시하고, 전체 요구사항 충족 후 기준 명세 반영과 archive를 진행한다.
