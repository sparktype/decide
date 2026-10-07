# 체크리스트: local 백엔드를 별도 서버 호출로

계획은 `plan.md`, 결정 기록은 `context-notes.md`.

## 테스트 먼저

- [x] local이 로컬 주소로 보내고 `routing.backend == "local"`, model은 서버 응답 값
- [x] local choice 256개가 한도 오류 없이 전송까지 간다(typesafe는 전송 전 거절)
- [x] local 연결 실패는 안내 문구, 다른 백엔드로 안 넘어감, 재시도 없음
- [x] local `decide_many`가 질문 전체를 한 번의 요청으로 보낸다
- [x] 주소 우선순위: `DECIDE_LOCAL_URL` > `[local].url` > 기본값
- [x] 옛 `[local]` 키(weights, repo, hf_*)가 있어도 설정이 깨지지 않는다

## 구현

- [x] `config.rs`: `local_url`만 남기고 가중치·HF 키 제거
- [x] `typesafe.rs`: `LiveTransport::local(url)`(인증 헤더 없음)
- [x] `backend.rs`: Local이 같은 경로를 탐, 한도 검사는 typesafe만, 연결 실패 안내, `live_transport`가 백엔드별 주소 선택
- [x] `daemon.rs`: 선로딩 제거
- [x] `local/`, `lib.rs`의 `mod local`, `parity` 기능, 의존성 제거
- [x] `tests/parity.rs`, `tests/parity_fixtures/`, `scripts/clef_flash_oracle.py` 제거
- [x] 통합 테스트(`stdio.rs`, `daemon.rs`, `gate_eval.rs`)를 서버 호출 기준으로 수정
- [x] `help.rs`: `CLEF_WEIGHTS` → `DECIDE_LOCAL_URL`
- [x] `scripts/serve-local.sh`
- [x] `release.yml`, `packaging/homebrew/decide.rb`에서 Metal·cmake·metallib 제거
- [x] 버전 0.8.0, CHANGELOG
- [x] CLAUDE.md, README, `docs/kev-setup.md` 갱신

## 검증

- [x] `cargo test` 통과(모델 서버·네트워크 없이). 단 `tests/daemon.rs`는 실제 `decide daemon`이 :48080을 잡고 있어 이 머신에서 실행하지 못했다(변경 전 코드도 같은 이유로 실패). 같은 경로는 `daemon::tests`와 `tests/stdio.rs`가 검증한다
- [x] `cargo build --release` 성공(cmake·Metal 없이 13초)
- [x] `bash -n`과 스크립트 실제 기동: 서버가 뜨고 웜업 두 건이 200으로 끝나 `준비됨`이 나온다
- [x] 실서버로 `decide_many` 한 번: `routing.backend == "local"`, 첫 호출 474ms, 같은 state 재호출 98ms, 서버를 끄면 안내 문구

## 릴리스 때 할 일 (이번 변경에 넣지 않았다)

- [ ] v0.8.0 태그를 푸시해 릴리스 자산을 만든다
- [ ] `packaging/homebrew/decide.rb`와 tap의 formula를 0.8.0 url·sha256으로 바꾸고 `mlx.metallib` 설치·검사를 뺀다
- [ ] 실행 중인 옛 데몬(0.7.0)을 `pkill -f "decide daemon"`으로 끄고 `/mcp`로 다시 연결한다
- [ ] 모드(`sparktype/claude-plugins`의 decide)가 읽는 `CLEF_WEIGHTS`·`DECIDE_LOCAL_REPO`와 백엔드 박스의 가중치 표시를 `DECIDE_LOCAL_URL` 기준으로 바꾼다

## 추가: MCP 등록을 stdio로 (2026-10-07)

- [x] `install_mcp_args` 테스트와 구현, 데몬 자동 기동 제거
- [x] 기존 등록이 있을 때 이전 방법 안내(테스트 포함)
- [x] `.mcp.json`, `tests/project_config.rs`, `tests/install_claude.rs`, 도움말
- [x] 임시 설정 폴더에서 `claude mcp add` 후 `✔ Connected`
- [ ] 릴리스 뒤 실제 등록을 옮긴다: `claude mcp remove -s user decide && decide install`
