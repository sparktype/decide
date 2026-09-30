# Changelog

이 파일은 [Keep a Changelog](https://keepachangelog.com/ko/1.1.0/) 형식을 따른다.

## [Unreleased]

### 추가

- `decide hook` 서브커맨드: Claude Code `PostToolUse` 훅 입력(`mcp__decide__decide`)을 읽어 질문, 선택과
  확률, 백엔드·모델·지연을 `systemMessage` 한 줄 요약으로 출력한다. 읽을 수 없는 입력에는 아무것도
  쓰지 않고 종료 코드 0이다. 실제 Claude Code 세션에서의 표시는 아직 확인하지 않았다.

## [0.0.4] - 2026-09-30

### 추가

- `DECIDE_BACKEND=local`(또는 키 없음)이 `jev-style serve`의 `/v1/systemone`을 호출한다.
  주소는 `DECIDE_LOCAL_URL`(기본 `http://127.0.0.1:8765/v1/systemone`)이고 인증 헤더를
  보내지 않는다. 모델은 Jev-Style-2B-Decision-v3-MLX 8bit다. 응답은 `routing.backend: "local"`이다.
- 로컬 연결 실패는 `jev-style serve` 실행을 안내하는 오류로 끝나고 다른 백엔드로 넘어가지 않는다.

### 변경

- choice 255개 한도 검사는 TypeSafe에서만 호출 전에 한다. 로컬은 서버가 422로 거절한다.
- 오류 문구의 백엔드 이름이 TypeSafe 또는 로컬로 바뀐다.
- 이전 Python 구현(`src/decide/`, `tests/`, `test_smoke.py`, `pyproject.toml`)을 지웠다.
  Stop 훅의 임계값 검사는 `crates/decide/tests/stop_hook.rs`로 옮겼다.

### 제거

- "로컬 백엔드가 아직 준비되지 않았습니다" 오류.

## [0.0.3] - 2026-09-30

### 추가

- `decide daemon`이 상주 프로세스 안에서 동일한 (state, type, instructions,
  options, criteria, 해석된 백엔드) 요청에 대한 응답을 최대 64개까지 캐싱한다.
  캐시 적중 시 `routing.cached: true`, `latency_ms: 0.0`을 반환한다. `decide mcp`는
  호출마다 새 프로세스라 캐시가 없다. Stop 훅처럼 세션당 여러 번 같은 상태로
  daemon을 호출하는 소비자가 TypeSafe 재호출 비용을 아낀다.
- `.claude/hooks/stop_verify.py`의 노울 신뢰도 임계값(기존 하드코드 0.4)을
  `DECIDE_STOP_THRESHOLD` 환경 변수로 뺐다. 값이 없거나 숫자로 파싱되지 않으면
  0.4로 되돌아간다(fail-open 원칙 유지).

### 변경

- `.gitignore`가 `.claude/`, `.cursor/`, `.gemini/`, `.grok/`를 통째로 무시하는
  가운데, `.claude/hooks/stop_verify.py`만 예외로 다시 git에 추적한다 — 이
  파일은 decide의 실제 기능(Stop 훅)이라 배포 대상이어야 하기 때문이다.

## [0.0.2] - 2026-09-30

### 변경

- Homebrew formula가 소스를 `cargo install`로 빌드하던 방식을 버리고, GitHub Release에
  올라간 사전 빌드 arm64 바이너리를 다운로드해 그대로 설치하는 방식으로 바꿨다. 설치 시
  Rust 툴체인이 필요 없다.
- `v*` 태그를 push하면 GitHub Actions(`.github/workflows/release.yml`)가
  `cargo test` → `cargo build --release` → tarball과 체크섬 생성 → GitHub Release 첨부까지
  자동으로 수행한다.
- `decide`를 인자 없이 실행하면 stdio MCP 서버를 시작하던 동작을 도움말 출력으로 바꿨다.
  MCP 서버는 이제 `decide mcp`로만 시작한다. `.mcp.json`의 `decide` 항목도
  `"args": ["mcp"]`로 맞춰 고쳤다.

### 추가

- `decide install` 서브커맨드. `claude mcp add -s user decide -- /opt/homebrew/bin/decide mcp`를
  대신 실행해 Claude Code 사용자 스코프에 `decide`를 등록한다. 이미 등록돼 있으면(`claude mcp
  add`가 "already exists"로 실패해도) 성공으로 취급해 재실행이 안전하다. `claude` CLI가 없거나
  그 외 이유로 등록이 실패하면 한국어 오류를 내고 비영 종료 코드로 끝난다.

## [0.0.1] - 2026-09-29

### 추가

- 판단 로직을 Python(`src/decide/`, Laya 기반)에서 Rust 크레이트 `crates/decide`로 옮겼다.
  실행 파일 하나가 `DECIDE_BACKEND`로 TypeSafe Jev와 로컬 백엔드 중 하나를 고른다.
- `decide`(또는 `decide mcp`)는 stdio MCP 서버, `decide daemon`은
  `~/.cache/decide/decide.sock`에서 JSON 한 줄을 받는 유휴 30분 데몬이다.
- `.claude/hooks/stop_verify.py`가 이 데몬을 호출해 세션의 코드 변경이 테스트로 검증됐는지
  확인하도록 연결했다.
- Homebrew formula(`packaging/homebrew/decide.rb`)로 소스를 받아 `cargo install`로 빌드해
  `/opt/homebrew/bin/decide`에 설치하는 최초 배포 경로를 만들었다.
- 로컬 백엔드는 게시 필드 비교(Laya 0.3.6 대비 `round(x, 4)` 일치)가 끝나기 전까지
  `로컬 백엔드가 아직 준비되지 않았습니다` 오류를 반환한다.
