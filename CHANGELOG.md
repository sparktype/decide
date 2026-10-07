# Changelog

이 파일은 [Keep a Changelog](https://keepachangelog.com/ko/1.1.0/) 형식을 따른다.

## [0.8.0] - 2026-10-07

### 변경

- `local` 백엔드가 모델을 프로세스 안에서 돌리지 않고, 따로 띄운 System One 서버를 TypeSafe와 같은
  HTTP 경로로 부른다. 주소는 `DECIDE_LOCAL_URL`, 없으면 `config.toml`의 `[local].url`, 없으면
  `http://127.0.0.1:8009/v1/systemone`이다. 인증 헤더는 보내지 않는다. `routing.backend`는 `local`이고
  `routing.model`은 서버가 돌려준 값이다(Kev는 `jev-latest`).
- 로컬 서버가 꺼져 있으면 `scripts/serve-local.sh`를 안내하는 오류로 끝나고 다른 백엔드로 넘어가지 않는다.
- choice 255개·score 10등급 한도 검사는 TypeSafe에서만 호출 전에 한다. 로컬은 서버가 판단한다.
- `decide_many`는 로컬에서도 질문 전체를 한 번의 요청으로 보낸다.
- `decide install`이 MCP를 stdio(`claude mcp add -s user decide -- /opt/homebrew/bin/decide mcp`)로 등록하고 데몬을 띄우지 않는다. 이미 등록돼 있으면 그대로 두고 옮기는 방법을 알려 준다. 저장소 `.mcp.json`도 stdio로 바꿨다. 데몬은 게이트용 Unix 소켓만 서빙한다.
- 릴리스 압축 파일에는 `decide`만 들어간다(`mlx.metallib` 없음). 릴리스 워크플로에서 Xcode 선택, Metal
  툴체인, cmake 단계를 뺐다.

### 추가

- `scripts/serve-local.sh`: Kev-4B(`kev.serve`, MLX)를 고정 커밋으로 받아 띄우고, 뜰 때까지 기다린 뒤
  웜업 요청을 보낸다. `decide`는 서버를 띄우거나 감시하지 않는다.

### 삭제

- 데몬의 HTTP MCP 서버(`http.rs`, `tiny_http`, `127.0.0.1:48080/mcp`)와 `decide daemon`의 HTTP 포트 검사. MCP는 stdio(`decide mcp`)뿐이다. 이전에 HTTP로 등록한 클라이언트는 연결이 끊기므로 `claude mcp remove -s user decide` 뒤 `decide install`로 옮긴다. 부수 효과로 `tests/daemon.rs`가 실제 데몬과 포트를 다투지 않는다.
- in-process Clef-flash 추론(`mlx-rs`, candle, `local/`), 가중치 다운로드, 데몬의 모델 선로딩,
  `CLEF_WEIGHTS`, `DECIDE_LOCAL_REPO`, `DECIDE_LOCAL_TIMING`, `[local].weights`·`repo`·`hf_*`, `parity` 기능,
  `tests/parity.rs`, `scripts/clef_flash_oracle.py`. 옛 설정 키는 남아 있어도 무시한다.

## [0.0.6] - 2026-10-01

### 추가

- `decide install --claude`: MCP 등록에 더해 표시 훅(`decide hook`)을 `~/.claude/settings.json`의
  `hooks.PostToolUse`에 멱등하게 추가한다. 다른 설정은 순서까지 보존하고, 깨진 JSON이나 병합할 수
  없는 모양이면 파일을 쓰지 않으며, 바꾸기 전에 `settings.json.bak-decide`로 백업한다. 그냥
  `decide install`은 이전처럼 MCP만 등록한다.

## [0.0.5] - 2026-10-01

### 추가

- MCP 도구 `decide_many`가 한 state에 질문 여러 개를 한 번의 백엔드 호출로 판단한다.
  `questions`(id → 질문)를 받아 `answers`(입력 순서), `routing`, `latency_ms`를 돌려준다.
  하나라도 실패하면 전체가 오류다. `decide` 도구는 바뀌지 않는다.
- `decide daemon` 소켓 줄에 `questions`가 있으면 `decide_many`와 같은 모양으로 답하고 같은
  LRU 캐시를 쓴다. `type`과 `questions`를 함께 주면 오류다.
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
