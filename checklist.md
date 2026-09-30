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

- [ ] 실패하는 테스트 먼저: 로컬 선택 시 로컬 주소로 호출, `Authorization` 없음
- [ ] 실패하는 테스트: 로컬 응답 → `routing.backend == "local"`
- [ ] 실패하는 테스트: 로컬에서 choice 256개가 한도 오류 없이 호출까지 감
- [ ] 실패하는 테스트: 로컬 연결 실패는 안내 문구를 내고 다른 백엔드로 안 넘어감
- [ ] `local.rs`: 기본 주소와 오류 문구
- [ ] `typesafe.rs`: `LiveTransport`가 주소와 선택적 키를 받고 오류 문구가 백엔드 이름을 받음
- [ ] `backend.rs`: `Backend::Local`이 같은 경로를 타도록 변경, NOT_READY 제거
- [ ] `DECIDE_LOCAL_URL` 읽기 (`Env`에 추가)
- [ ] `cargo test` 전체 통과

## 실행 검증 (Python 삭제 전 게이트)

- [ ] `cargo test` 통과
- [ ] 키가 있는 환경에서 TypeSafe 호출이 `routing.backend == "typesafe"`
- [ ] `jev-style serve` 기동 후 `DECIDE_BACKEND=local` 호출이 `routing.backend == "local"`, 한글 state 정상
- [ ] 데몬 소켓에 JSON 한 줄 → 같은 껍데기 응답

## 정리 (위 게이트 통과 후 같은 변경에서)

- [ ] `src/decide/`, `tests/`의 Python 테스트, `test_smoke.py`, `pyproject.toml`, `.python-version` 삭제
- [ ] `.claude/skills/decide/SKILL.md`, README, CLAUDE.md의 로컬 백엔드·pip·pytest 절 수정
- [ ] `2026-09-29` 설계 문서 첫머리에 "로컬 런타임 절은 2026-09-30 문서로 대체" 한 줄 추가
- [ ] 버전 올리고 릴리스 (formula 체크섬 포함)
