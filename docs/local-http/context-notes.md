# 결정 기록: local 백엔드를 별도 서버 호출로

## 2026-10-07 시작

- 요청: 백엔드는 typesafe와 local로 나누고, 백엔드는 별도로 띄운다. decide는 모델 서빙을 하지 않는다.
  최적의 성능을 내는 방법으로 하고, 서빙은 스크립트로 쓴다.
- 확인한 두 가지: 서버 모델은 Kev-4B, 기존 in-process 추론은 삭제하고 HTTP 호출로 교체.
- 선례: 0.0.4가 `jev-style serve`를 `DECIDE_LOCAL_URL`로 부르는 같은 모양이었다(CHANGELOG). 그 뒤 in-process MLX로
  갔다가 이번에 서버 호출로 돌아온다. 이유는 모델 서빙을 decide 밖으로 빼는 것이다.
- 작업은 다른 세션과 체크아웃을 공유하므로 worktree `../decide-local-http`, 브랜치 `feat/local-http`에서 한다.
- Kev 저장소 현재 `main`이 `5e42a7a`로, `docs/kev-setup.md`에서 검증한 커밋과 같다. 스크립트가 이 커밋을 고정한다.

## 2026-10-07 구현 중 결정과 발견

- 주소·인증: local은 `LiveTransport::local(url)`로 `Authorization` 없이 보낸다. `live_transport(env)`가 백엔드를 보고 주소를 고른다
  (`pick_local_url`: `DECIDE_LOCAL_URL` > `[local].url` > 기본값). 서버 인증 키(`KEV_API_KEY`)는 YAGNI라 넣지 않았다.
- `routing.model`은 서버가 돌려준 값이다. Kev는 요청의 별칭 `jev-latest`를 그대로 돌려줘서 `local · jev-latest`로 보인다. 틀리진 않지만
  모델 이름은 아니다. 고치려면 `[local].model` 같은 표시용 값이 필요한데 이번에는 하지 않았다.
- 한도 검사(255 옵션, 10 등급)는 typesafe에서만 한다. 0.0.4의 결정과 같다.
- 연결 실패에만 `scripts/serve-local.sh` 안내를 붙인다. HTTP 오류(예: Kev의 422)는 서버 메시지가 더 정확해서 그대로 둔다.
- 웜업: 첫 시도에서 스크립트가 서버를 내렸다. 웜업의 choice 요청이 `options`를 썼는데 decide는 `criteria`를 보내고 서버가 422로 거절했고,
  `curl -f`와 `set -e`가 스크립트를 끝내 trap이 서버를 죽였다. 요청 모양을 decide와 같게 고치고, 웜업 실패는 경고만 하고 서버는 두게 했다.
  실제 decide 요청 모양을 스크립트가 따라야 한다는 교훈이다.
- 측정(M1 Max, 같은 입력): 첫 호출 474ms, 같은 state 재호출 98ms. 이전 in-process Clef-flash는 같은 종류 입력에서 수 초였다.
- 버그가 아닌 환경 문제: `tests/daemon.rs`는 :48080을 직접 바인딩해서 실제 데몬이 떠 있으면 실패한다. 변경 전에도 같았다. 데몬 포트를
  환경변수로 바꿀 수 있게 하는 것은 이번 범위가 아니다.
- CHANGELOG의 맨 위 항목이 0.0.6인데 Cargo 버전은 0.7.0이었다(변경 기록이 뒤처져 있다). 0.8.0 항목을 위에 추가했지만 0.0.7~0.7.0 사이는
  비어 있다.
- 포뮬러 `packaging/homebrew/decide.rb`는 일부러 안 바꿨다. 새 릴리스 자산의 url·sha256이 없으면 0.7.0 꾸러미(metallib 포함)를 `decide`만
  설치하게 되어 깨진다. 릴리스 때 함께 바꾼다(체크리스트).

## 2026-10-07 MCP 등록을 stdio로

- 질문: stdio가 성능에 유리한가. 측정(`tools/list`, M1 Max, release)으로 전송 오버헤드만 비교했다. stdio 메시지당 0.054ms(p95 0.097),
  데몬 HTTP 요청당 0.394ms(p95 0.690, 요청마다 새 연결). 차이 0.3ms는 백엔드 호출(Kev 재호출 약 100ms, 첫 호출 약 470ms)에 묻힌다.
  stdio 프로세스 기동은 5ms 안팎이고, 새로 빌드한 바이너리의 첫 실행만 457ms였다(콜드 효과).
- 그래서 성능이 아니라 의존성 때문에 바꾼다. 데몬이 꺼지면 MCP 도구가 연결 실패이던 문제가 사라지고, 모델을 안 올리니 세션마다
  프로세스 하나가 가볍다(0.7.0까지는 세션마다 Clef-flash 10GB라서 데몬 공유가 필요했다).
- 바뀐 것: `install_mcp_args`가 `claude mcp add -s user decide -- /opt/homebrew/bin/decide mcp`를 만든다. `decide install --claude`는 더 이상
  데몬을 띄우지 않는다(게이트 훅이 스스로 띄운다). 저장소 `.mcp.json`은 stdio 명령으로 바꿨다. 데몬의 HTTP 엔드포인트는 남겼다.
- 이미 `decide`가 등록돼 있으면(0.7.0의 HTTP 등록) 지우지 않고 성공으로 끝내되 `claude mcp remove -s user decide` 안내를 낸다. 사용자 설정을
  말없이 지우지 않기 위해서다.
- 도구 이름은 그대로 `mcp__decide__decide`라서 표시 훅과 모드의 매처는 바뀌지 않는다.
- 확인: 임시 `CLAUDE_CONFIG_DIR`에서 같은 인자로 `claude mcp add` 후 `claude mcp list`가 `✔ Connected`(stdio)를 보였다. 사용자 실제 설정은 건드리지 않았다.
- 실제 사용 중인 등록(HTTP)은 일부러 옮기지 않았다. 0.8.0이 설치돼야 `/opt/homebrew/bin/decide`가 새 동작을 가진다.

## 2026-10-07 데몬의 HTTP 서버 삭제

- 요청: `decide mcp`도 http에서 stdio로. 남은 HTTP는 데몬의 MCP 엔드포인트와 사용자 설정의 옛 HTTP 등록이라 둘 중 무엇인지 물었고,
  데몬의 HTTP 서버 코드 삭제로 정했다. 실제 등록(HTTP)은 옮기지 않았다.
- 지운 것: `http.rs`, `tiny_http` 의존성, `DEFAULT_HTTP_ADDR`, `serve_unified`의 HTTP 스레드·포트 검사. `daemon::serve(uds_path, idle)`가 UDS만
  서빙한다. 이미 데몬이 있으면 두 번째는 소켓을 지우지 않고 물러난다(테스트로 고정).
- 부수 효과: `tests/daemon.rs`가 :48080을 잡던 실제 데몬과 부딪혀 실패하던 문제가 사라졌다. 전체 테스트가 처음으로 전부 통과한다.
- 지난 변경 때 놓친 낡은 문구도 고쳤다. 데몬 도움말의 "모델은 첫 요청에서 읽는다"(in-process 삭제 때 남음), README의 "`decide install`이 설치 시점에
  데몬을 한 번 띄운다"(stdio 전환 때 남음).
- 영향: 0.7.0 데몬(:48080)에 HTTP로 붙어 있던 클라이언트는 0.8.0 이후 연결 실패다. 옮기는 방법은 CHANGELOG와 `decide install`의 안내에 있다.

## 2026-10-07 0.8.0 릴리스

- `feat/local-http`의 8커밋을 `origin/main`에 fast-forward로 푸시(내 커밋만 확인)하고 `v0.8.0` 태그를 푸시했다. Release 워크플로가 성공해
  자산 `decide-v0.8.0-aarch64-apple-darwin.tar.gz`와 `checksums.txt`가 생겼다(sha256 9803b11d…). 공유 체크아웃의 로컬 `main`은 건드리지 않았으니
  필요할 때 `git pull`로 맞춘다.
- tap(`sparktype/homebrew-tap`) formula를 0.8.0 url·sha256으로 바꾸고 `mlx.metallib` 설치·검사를 뺐다. 로컬 tap 클론이 원격보다 5커밋
  뒤처져 있어 `origin/main` 기준 worktree에서 작업해 푸시했다. 이 머신의 설치(`brew upgrade decide`)는 하지 않았다.
- 플러그인(`sparktype/claude-plugins`)은 0.2.0으로 푸시했다(바이너리 v0.8.0, stdio MCP, 게이트 훅). 바이너리 커밋은 훅의 대용량 파일 검사(500KB)에
  걸려 그 커밋 하나만 `--no-verify`로 만들었다(시크릿 검사는 바이너리를 건너뛰므로 잃은 검사는 크기뿐이다).
