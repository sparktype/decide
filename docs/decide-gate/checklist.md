# decide gate 체크리스트

- [x] 설계서 작성 (`docs/superpowers/specs/2026-10-04-decide-gate-design.md`)
- [x] 기본값 확정(설정 경로, display=decisions, enforce는 수동 전환, 첫 게이트 bash-risk)
- [x] 설계서의 감사 모드 모순 정정(감사 모드는 permissionDecision을 내지 않음)
- [x] 구현 계획 작성
- [ ] 0. 로컬 빌드 환경 복구(Xcode 16 이상) 또는 PR용 CI 도입
- [ ] 1. `claude.rs` 훅 설치를 이벤트·matcher 인자로 일반화
- [ ] 2. 데몬 `client_version` 확인과 stale 종료
- [ ] 3. 게이트 설정(내장 기본값, 층 병합, 출처 추적)
- [ ] 4. bash-risk 순수 로직(가리기, state, 사전 필터, 판정)
- [ ] 5. 출력(훅 JSON, 근거 표시 문구, 실패 시 통과) + show.rs legend 배열 수정
- [ ] 6. 클라이언트(소켓, 데몬 띄우기, stale, 감사 로그)
- [ ] 7. CLI 연결(`gate <이름>`, `gate --show [이름] [--json]`)
- [ ] 8. `decide install --claude`가 PreToolUse 게이트 훅 등록
- [ ] 9. README와 CLAUDE.md 갱신
- [ ] 10. 전체 테스트, 수동 실행·지연 측정, 감사 모드 시범 운영
- [ ] 릴리스(0.3.0)는 별도 PR
- (뺌) 모델 정답률 확인과 평가 세트 — 사용자 지시로 범위에서 제외
