# 체크리스트: 로컬 백엔드를 Jev-Style v3 서버 호출로 전환

설계는 `docs/superpowers/specs/2026-09-30-decide-local-jev-style-design.md`. 결정 기록은
`context-notes.md`.

## 조사와 결정 (완료)

- [x] Swift가 아니라 Python→Rust 전환임을 확인
- [x] 로컬 모델 선정: Jev-Style-2B-Decision-v3-MLX 8bit
- [x] 한국어 스모크 8/8 통과 (2B v3 8bit)
- [x] `mlx-rs`로 직접 구동 가능성 조사 → 지금은 불가, 서버 호출(B)로 결정
- [x] `jev-style serve`가 TypeSafe와 같은 요청/응답 형식인지 실측 확인
- [x] 새 설계 문서 작성

## 구현

- [x] 실패하는 테스트 먼저: 로컬 선택 시 로컬 주소로 호출, `Authorization` 없음
- [x] 실패하는 테스트: 로컬 응답 → `routing.backend == "local"`
- [x] 실패하는 테스트: 로컬에서 choice 256개가 한도 오류 없이 호출까지 감
- [x] 실패하는 테스트: 로컬 연결 실패는 안내 문구를 내고 다른 백엔드로 안 넘어감
- [x] `local.rs`: 기본 주소와 오류 문구
- [x] `typesafe.rs`: `LiveTransport`가 주소와 선택적 키를 받고 오류 문구가 백엔드 이름을 받음
- [x] `backend.rs`: `Backend::Local`이 같은 경로를 타도록 변경, NOT_READY 제거
- [x] `DECIDE_LOCAL_URL` 읽기 (`local::url()`로 구현, `Env`는 바꾸지 않음)
- [x] `cargo test` 전체 통과

## 실행 검증 (Python 삭제 전 게이트)

- [x] `cargo test` 통과 (31건)
- [x] 키가 있는 환경에서 TypeSafe 호출이 `routing.backend == "typesafe"` (사용자가 직접 실행: jev-1.13.0, noul 0.84, 528ms)
- [x] `jev-style serve` 기동 후 `DECIDE_BACKEND=local` 호출이 `routing.backend == "local"`, 한글 state 정상 (debug 빌드, `decide mcp` stdio로 choice/score 확인)
- [x] 데몬 소켓에 JSON 한 줄 → 같은 껍데기 응답 (임시 HOME, 로컬 백엔드+실서버: 1차 154ms, 동일 요청 `cached: true`·0ms, 검증 오류 후에도 데몬 유지)

## 정리 (위 게이트 통과 후 같은 변경에서)

- [x] `src/decide/`, `tests/`의 Python 테스트, `test_smoke.py`, `pyproject.toml`, `.python-version` 삭제 (훅 임계값 검사는 `tests/stop_hook.rs`로 이전)
- [x] README, CLAUDE.md의 로컬 백엔드·에이전트 사용 가이드 수정 (로컬 백엔드 설정, 한도 표, 에이전트 사용 지침)
- [x] README, CLAUDE.md의 pip·pytest 절 삭제
- [x] `.claude/skills/decide/SKILL.md`: 저장소에 포함되지 않음(`.gitignore`). README와 CLAUDE.md의 언급을 "에이전트가 쓸 때" 절 참조로 고침
- [x] `2026-09-29` 설계 문서 첫머리에 "로컬 런타임 절은 2026-09-30 문서로 대체" 한 줄 추가
- [ ] 버전 올리고 릴리스 (formula 체크섬 포함)

---

# 체크리스트: 에이전트 하니스 기능 추가 (영상 노트 기반)

로드맵은 `docs/superpowers/specs/2026-09-30-decide-many-design.md`의 "로드맵" 절.
기능마다 설계 → 계획 → 구현을 따로 돈다.

## 1. 다중 질문 `decide_many`

- [x] 기능 후보 선정(사용자: 4개 모두) 및 분해, 순서 합의
- [x] 인터페이스 결정: 새 도구 `decide_many`
- [x] 설계 확정(전부 성공/전부 실패, 상한은 백엔드에 맡김, `decide` 불변)
- [x] 설계 문서 작성: `2026-09-30-decide-many-design.md`
- [x] 사용자가 설계 문서 검토·승인
- [x] 구현 계획 작성(`docs/superpowers/plans/2026-09-30-decide-many.md`)
- [x] 구현, `cargo test`(단위 42 + 통합 전부), 로컬 실서버 다중 질문 실측(한글 3문항 정상. 웜업 후 다중 134ms vs 단일 3회 139ms: 로컬은 지연 절감 없음)
- [ ] TypeSafe 다중 질문 실호출 확인(키 필요, 사용자)

## 2. 인젝션 선검사 (1 이후)

- [ ] 설계

## 3. PreToolUse 위험 게이트 훅

- [ ] 설계

## 4. 스킬 캐스케이드 선택 (힌트 모드부터)

- [ ] 설계: 카탈로그 형식, 성공 기준(과거 세션의 (프롬프트, 호출된 스킬) 쌍으로 top-1/top-2 일치율)
- [ ] `/skill-doctor`와 `/doctor`로 실제 스킬 목록 비용·예산 초과 여부 측정

## 5. RAG 리랭킹

- [ ] 1번 이후 필요성 재판단
