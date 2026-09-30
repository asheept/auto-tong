# auto-tong OpenSpec

## 현재 상태

2026-09-30 전체 코드 점검을 바탕으로 가져오기·복구·감시·Prism·Java·업데이트 처리를 수정했다. 검증된 작업 50/52개를 완료 표시했다. 실제 PrismLauncher 게임 실행 중 보류 확인과 최종 명세 반영·archive가 남았다. OpenSpec 1.13.2의 `spec-driven` schema와 한국어 산출물을 사용한다.

| 문서 | 내용 |
| --- | --- |
| [코드 점검 보고서](reviews/2026-09-30/code-review.md) | P1 9건·P2 12건, 코드 위치, 영향, 재현 결과 |
| [재현 안내](reviews/2026-09-30/verification/README.md) | 임시 fixture 기반 Rust/JS 진단 실행법과 한계 |
| [제안서](changes/harden-auto-tong-import-workflow/proposal.md) | 개선 이유·범위·영향 |
| [설계](changes/harden-auto-tong-import-workflow/design.md) | staging, journal, 이력 이전, 작업·업데이트 상태 |
| [구현 작업](changes/harden-auto-tong-import-workflow/tasks.md) | 검증된 작업 50개와 남은 작업 2개 |
| [검증 기록](changes/harden-auto-tong-import-workflow/verification-report.md) | Rust 88개·JS 8개, 실제 Windows·WebView2 검증, GitHub CI 결과 |

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

현재 strict 검증과 [Windows CI](https://github.com/asheept/auto-tong/actions/runs/36656073469)는 통과했다. CLI의 planning 완료 표시는 문서가 준비되었다는 뜻이다. 실제 구현 진행률은 `tasks.md`의 체크박스를 기준으로 한다.

남은 검증은 이 change의 `openspec-apply-change`로 이어간다. 검증한 항목만 근거와 함께 완료 표시하고, 전체 요구사항 충족 후 기준 명세 반영과 archive를 진행한다.
