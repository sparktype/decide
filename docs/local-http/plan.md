# 계획: local 백엔드를 별도 서버 호출로 바꾼다

## 왜

local 백엔드는 지금 Clef-flash를 프로세스 안에서 돌린다(MLX 백본 + candle 헤드). 이 방식은 decide가
모델 로딩·가중치 다운로드·Metal 툴체인 빌드를 떠안고, 바이너리에 mlx.metallib을 같이 배포해야 한다.
모델 서빙은 decide의 일이 아니라고 정했다. 백엔드는 두 가지(typesafe, local)로 나누고, 둘 다
System One 형식(`POST /v1/systemone`)의 서버를 HTTP로 부르기만 한다. local 서버는 따로 띄운다.

## 결정 (사용자 확인, 2026-10-07)

- local이 부르는 서버 모델은 Kev-4B다. `docs/kev-setup.md`에서 검증됨(Clef-flash 8비트 대비 지연 3~5배 낮음, state 캐시).
- 기존 in-process 로컬 추론(MLX 백본, candle 헤드, 가중치 다운로드, parity 테스트)은 삭제하고 HTTP 호출로 바꾼다.
- 서버는 `scripts/serve-local.sh`가 띄운다. decide는 서버를 시작하지도 감시하지도 않는다.

## 범위

1. `Backend::Local`이 TypeSafe와 같은 HTTP 경로(`typesafe::execute`)를 탄다. 주소는 `DECIDE_LOCAL_URL`,
   없으면 `config.toml [local].url`, 없으면 `http://127.0.0.1:8009/v1/systemone`. `Authorization` 헤더는 보내지 않는다.
2. choice 255개 한도 검사는 TypeSafe에서만 한다(로컬은 서버가 판단한다).
3. 로컬 연결 실패는 `scripts/serve-local.sh`를 안내하는 오류로 끝나고 다른 백엔드로 넘어가지 않는다.
4. `crates/decide/src/local/`, `parity` 기능, 관련 의존성(candle, tokenizers, hf-hub, mlx), `tests/parity.rs`,
   `scripts/clef_flash_oracle.py`, 데몬 선로딩, `CLEF_WEIGHTS`와 `[local]` 가중치·HF 키를 지운다.
5. 릴리스 워크플로와 Homebrew 포뮬러에서 Metal·cmake·mlx.metallib을 뺀다.
6. 서빙 스크립트 `scripts/serve-local.sh`: kev 저장소를 고정 커밋으로 받고, `uv`로 `kev.serve`를 띄우고, 뜰 때까지 기다린 뒤
   웜업 요청을 한 번 보낸다(커널 컴파일 비용을 첫 실제 요청이 안 떠안게).
7. 문서: CLAUDE.md, README, `docs/kev-setup.md`, CHANGELOG, help.

## 성능 방침

모델은 서버 프로세스에 상주한다. 같은 state의 질문은 `decide_many`로 한 요청에 묶어 서버의 state 캐시를 쓴다
(이미 `decide_many`가 그렇게 보낸다). 웜업은 스크립트가 한다. 서버는 127.0.0.1에만 바인딩한다.

## 하지 않는 것

- 서버 자동 시작, 상태 감시, launchd 구성(decide가 서빙을 안 한다는 결정에 반한다).
- Kev 외 모델 지원, 서버 인증 키(`KEV_API_KEY`) 전달. 필요해지면 `[local].api_key`로 추가한다.
- 점수 보정. 두 백엔드의 확률은 서로 보정되어 있지 않다.

## 검증

- `cargo test --manifest-path crates/decide/Cargo.toml`(가중치·네트워크 없이 통과).
- `bash -n scripts/serve-local.sh`, 가능하면 실제로 띄워 `DECIDE_BACKEND=local decide mcp`로 `decide_many` 한 번.
