# 작업 인수인계 (2026-10-08)

다른 PC에서 이어 작업하기 위한 현재 상태 정리다. 완료된 작업과 남은 작업, 그 PC에서만
있는 로컬 설정(홈 디렉터리, launchd, config.toml)을 구분해 적는다.

## 브랜치 상태

`main`에서 분기한 feature 브랜치 3개가 각각 커밋 완료, **origin에는 아직 push 전**이다.

| 브랜치 | 내용 | 커밋 |
| --- | --- | --- |
| `feature/mcp-cleanup` | `.mcp.json`에서 중복된 `decide` 서버 등록 제거(플러그인판과 중복, 바이너리 없어 ENOENT) | `5e692bf` |
| `feature/local-model-config` | local 백엔드 모델을 `config.toml`의 `[local].model`/`DECIDE_MODEL`로 설정 가능하게 함 | `7a586e7` |
| `feature/kev-launchd-install` | `scripts/install-launchd.sh` 추가 — Kev MLX 서버를 launchd 사용자 에이전트로 등록 | `f6ba728` |

세 브랜치는 서로 독립적이라 어느 순서로 머지해도 충돌이 날 일은 거의 없다(건드린 파일이
겹치지 않음: `.mcp.json` / `crates/decide/src/*.rs`+`README.md`+`CLAUDE.md` /
`scripts/install-launchd.sh`+`docs/kev-setup.md`).

다른 PC에서:
```bash
git fetch origin
git checkout feature/mcp-cleanup       # 또는 다른 두 브랜치
```

## 이번 세션에서 한 일 (시간순)

1. **`.mcp.json` 중복 decide 서버 제거** — `claude mcp list`로 `decide`(user scope,
   `/opt/homebrew/bin/decide` 가리킴, 바이너리 없음)와 `plugin:decide:decide`(정상) 중복
   발견. `claude mcp remove -s user decide`로 user-scope 등록 제거, `.mcp.json`의
   `decide` 항목도 지움.
2. **local 백엔드 모델 설정 가능하게 함** — `typesafe.rs`의 `MODEL` 상수(`jev-latest`
   고정)를 `request_body`/`request_body_many`의 매개변수로 바꾸고, `backend.rs`에
   `pick_local_model`/`resolved_local_model`/`model_for` 추가. 우선순위:
   `DECIDE_MODEL` 환경변수 → `[local].model` → 기본값 `jev-latest`. **중요한 제약**:
   Kev 서버는 요청의 `model` 필드를 그대로 echo할 뿐이라, 이 설정은 요청 바디와
   `routing.model` 표시값만 바꾸고 실제 서빙 모델(=서버 기동 시 `KEV_MODEL`)은 안
   바꾼다. README에 명시함.
3. **로컬 백엔드를 Ollama → Kev로 전환** — `~/.config/decide/config.toml`이 한때
   `http://localhost:11434/v1/systemone`(Ollama, `jev-latest` 모델)을 가리켰는데,
   Ollama의 `/v1/systemone` decision 핸들러가 `instructions` 필드에 (고정이 아니라
   질문 타입·옵션 수에 따라 계산되는) ~174~180 토큰 제한을 걸어서 decide 게이트가
   보내는 긴 Bash 명령이 매번 `HTTP 400`으로 실패했다(`decide gate stats`로 확인,
   fail-open이라 Bash는 안 막혔지만 판정 자체가 안 났음). Ollama의 `num_ctx`를 모델
   재생성(`ollama create`)으로 2048까지 올려봤지만 MLX runner가 무시하고 그대로
   512라 효과 없었음 — 모델을 원래 설정(`num_ctx 512`)으로 복원함. 조사 결과 Laya는
   GGUF가 없어 Ollama 네이티브 지원이 안 되고 공식 Ollaya는 MLX 미지원(CPU만),
   `laya-mlx`는 서버가 아닌 in-process 라이브러리라 서빙 경로가 없음(비공식 HTTP
   서빙 PR은 메인테이너가 거절). 그래서 이 저장소가 이미 공식 지원·검증한 **Kev**
   (`docs/kev-setup.md`, `max_state_tokens: 65536`)로 되돌림:
   `~/.config/decide/config.toml`을 `http://127.0.0.1:8009/v1/systemone`으로 수정.
4. **Kev 서버를 launchd로 자동 기동하게 함** — `scripts/install-launchd.sh` 작성.
   `kev.serve`를 `uv run --extra serve python -m kev.serve --run $MODEL --host
   127.0.0.1 --port $PORT`로 **plist가 직접 실행**(bash 스크립트를 거치지 않음 —
   `KeepAlive`가 재시작할 때마다 clone/`uv sync`를 다시 하지 않도록). HMG 사내망
   SSL 인터셉트 환경이라 현재 셸의 `SSL_CERT_FILE`/`REQUESTS_CA_BUNDLE`/
   `CURL_CA_BUNDLE`/`UV_CERT`/`UV_SYSTEM_CERTS`/`NODE_EXTRA_CA_CERTS`를 읽어
   plist에 심어야 HuggingFace 다운로드가 통과한다(launchd는 로그인 셸의
   환경변수를 물려받지 않음). `~/Library/LaunchAgents/dev.sparktype.kev.plist`로
   등록 완료, `launchctl print gui/501/dev.sparktype.kev`로 `state = running` 확인.
5. **게이트 응답시간 측정** — Kev 전환 후 모델 응답 평균 681ms, 중앙값 654ms,
   p95 850ms(`decide gate stats --since 1h --json`의 `latency.local`). 간헐적으로
   "데몬이 2000ms 안에 답하지 않았습니다" 타임아웃이 섞이는데, Kev 서버 직접
   호출은 늘 2초 이내(283~894ms)라 서버 지연이 아니라 데몬-서버 간 커넥션
   문제로 추정됨 — **원인 미확정, 다음 작업 후보**.

## 다른 PC에서 로컬로 다시 해야 하는 것 (저장소에 없음)

이 PC(`hmc7102758`)의 홈 디렉터리/launchd 설정이라 git에는 안 들어 있다. 다른 PC에서
`feature/kev-launchd-install`을 받은 뒤:

```bash
~/.config/decide/config.toml 에 아래 추가(Kev 쓰려면):
backend = "local"
[local]
url = "http://127.0.0.1:8009/v1/systemone"

scripts/install-launchd.sh 실행 — kev 클론·uv sync·launchd 등록·웜업까지 한 번에 함.
```

사내망 환경이면 `install-launchd.sh`를 돌리는 셸에 `SSL_CERT_FILE` 등 인증서
환경변수가 설정돼 있어야 그 값이 plist에 들어간다(`~/.claude/references/hmg-network.md`
참고).

## 알아두면 좋은 것 / 다음에 볼 것

- **"ollama 토큰 설정이 있는지" 질문의 결론**: Ollama `num_ctx`는 있지만 MLX runner가
  무시한다(실험으로 확인). Ollama 경로는 포기하고 Kev로 간 상태.
- **간헐적 2000ms 타임아웃 원인 미확정** — 서버 응답속도 문제는 아님. `decide-doctor`
  스킬로 좀 더 파볼 수 있음.
- **decide-mod/debrief-mod 쪽 작업**(AbovePrompt → toast 전환)은 별도 저장소
  `claude-plugins`에 있다 — 그쪽 `docs/handoff.md` 참고.
