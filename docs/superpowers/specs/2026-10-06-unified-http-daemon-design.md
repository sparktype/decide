# 설계: daemon과 mcp를 하나의 HTTP 데몬으로 통합

## 배경

지금 Claude Code 세션 하나가 뜰 때마다(그리고 `/mcp` 재연결마다) `decide mcp`가
새 프로세스로 fork되고, 그 프로세스가 로컬 백엔드일 때 모델(Clef-flash, 수GB)을
자기 메모리에 직접 로드한다(`local::infer`를 자기 프로세스 안에서 직접 호출,
`backend.rs::decide`/`decide_many`). 세션이 여러 개면 그만큼 모델이 중복으로
로드되고, 재연결마다 처음부터 다시 로드한다(실측 콜드스타트 4~8초).

`decide gate bash-risk`는 이미 다른 패턴을 쓴다 — 매 Bash 호출마다 짧게 뜨고
죽는 CLI 프로세스가, 상주하는 `decide daemon`에 Unix 도메인 소켓(UDS)으로
질문하고 답을 받는다. 데몬이 없으면 그때 한 번 띄운다(`gate/client.rs::ask_or_start`).
모델은 그 데몬 하나에만 로드된다.

이 설계는 `decide mcp`도 같은 패턴을 쓰게 한다. 다만 MCP는 Claude Code가
자식 프로세스의 stdin/stdout으로 메시지를 주고받는 전제라, 상주 프로세스에
"붙는" 모델이 UDS가 아니라 **로컬 TCP HTTP**다 — Claude Code가 `.mcp.json`에서
MCP 서버를 `type: "http"`로 등록하면, Claude Code 자신이 그 URL에 직접
연결하므로 decide 쪽에서 매번 자식 프로세스를 fork하지 않는다.

## 결정

- **`decide daemon`이 두 가지를 모두 서빙한다**: 기존 UDS 소켓(게이트 전용,
  변경 없음)과 새 TCP HTTP 서버(MCP 전용) 둘 다 하나의 프로세스/하나의
  모델 런타임(`local::runtime()`) 위에서 돈다. `decide mcp`(stdio 서버)는
  새로 설치하는 사용자는 더 이상 쓰지 않지만, 서브커맨드 자체는 당장
  호환을 위해 코드에 남긴다("범위 밖" 참고).
- **HTTP는 MCP의 Streamable HTTP transport를 최소 범위로 구현한다**: 매
  요청이 `POST /mcp`로 와서 JSON-RPC 요청 하나를 담고, 응답으로 JSON-RPC
  결과 하나를 돌려준다. **SSE(서버가 스트리밍으로 여러 메시지를 보내는 것)는
  구현하지 않는다** — decide의 모든 요청/응답은 이미 요청 하나당 응답 하나뿐이고
  스트리밍할 중간 상태가 없다.
- **포트는 `127.0.0.1:48080`에 고정한다.** 환경변수나 config.toml로 바꾸는
  기능은 이번 설계에 넣지 않는다(범위 밖 — 필요해지면 `[gate]`처럼 나중에
  config.toml 키를 추가한다).
- **HTTP 서버 구현에 `tiny_http`(경량 blocking HTTP 서버 crate, 0.12)를
  새 의존성으로 추가한다.** TCP accept/HTTP 파싱을 직접 구현하지 않는다.
- **동시성**: UDS 쪽(`handle_connection`)과 HTTP 쪽(`tiny_http`의 요청 루프)을
  각각 자신의 스레드에서 돈다(`std::thread::spawn` 2개, `main` 스레드가 둘을
  join). 백본은 이미 `Mutex`로 감싸여 있어(`local::Runtime::backbone`)
  두 트랜스포트가 동시에 들어와도 추론 자체는 안전하게 직렬화된다.
- **idle 종료는 두 트랜스포트의 활동을 함께 본다.** 지금의 `accept_within`
  (UDS에만 적용되는 30분 idle)을 "마지막 요청 시각"(UDS든 HTTP든 상관없이
  갱신)으로 일반화해, 두 트랜스포트 모두 활동이 없을 때만 30분 뒤 종료한다.
- **`decide install`이 `.mcp.json`/`~/.claude.json`을 자동으로 `type: "http"`
  + `url: "http://127.0.0.1:48080/mcp"`로 등록한다.** 지금 `run_install`이
  `claude mcp add -- <bin> mcp`(stdio)를 호출하는 지점을 `claude mcp add
  --transport http decide http://127.0.0.1:48080/mcp`로 바꾼다. 사용자가
  직접 설정 파일을 편집할 필요가 없다.
- **HTTP 서버가 꺼져 있으면 Claude Code가 바로 연결에 실패한다** — stdio처럼
  "클라이언트가 필요하면 그때 프로세스를 띄운다"가 안 된다(Claude Code는
  URL에 연결만 시도하지 그 뒤의 서버를 띄워주지 않는다). 그래서 **`decide
  daemon`을 먼저 띄워 둬야 MCP가 동작한다** — 이 간극을 메우는 방법은
  "자동 기동" 절에서 다룬다.

## 자동 기동

HTTP는 UDS의 `ask_or_start`(연결 실패 시 그 자리에서 `spawn_daemon()`)와
같은 방식을 못 쓴다 — MCP 클라이언트(Claude Code)가 연결 실패를 감지해도
decide 쪽 코드를 실행해 주지 않기 때문이다. 대신:

- **`decide install --claude`가 데몬을 그 자리에서 한 번 띄운다**(설치
  직후 즉시 쓸 수 있게). 이미 떠 있으면(`claim_socket`과 같은 방식으로
  TCP 포트 바인드를 시도) 아무것도 안 한다.
- **macOS 로그인 시 자동 기동은 이번 설계에 넣지 않는다**(launchd plist
  등록은 범위 밖 — 필요해지면 별도 설계). 사용자가 머신을 재시작하면
  `decide daemon`을 다시 수동으로 띄워야 한다. `decide --help`나 설치
  안내에 이 제약을 명시한다.
- 데몬이 꺼진 상태에서 Claude Code가 `decide` 도구를 호출하면 연결 실패로
  도구 자체가 에러를 낸다(지금 "No such tool available... disconnected"
  류 메시지와 비슷한 사용자 경험). 이건 받아들인다 — stdio 때는 항상
  뜨던 게 HTTP에서는 수동 기동이 필요해지는 트레이드오프를, 모델 중복
  로드를 없애는 대가로 받는다.

## 구성 변경

### `crates/decide/Cargo.toml`

```toml
tiny_http = "0.12"
```

### `crates/decide/src/http.rs` (신규)

```rust
// decide daemon의 HTTP 트랜스포트: Claude Code가 MCP를 type="http"로 거는
// 창구다. POST /mcp 하나만 받고, 요청 본문의 JSON-RPC 메시지 하나를
// mcp::handle_message로 넘겨 응답 하나를 그대로 돌려준다. SSE는 쓰지 않는다
// — decide의 모든 응답은 요청 하나당 결과 하나뿐이라 스트리밍할 게 없다.
pub fn serve_http<T: Transport>(
    addr: &str,
    env: &Env,
    transport: &mut T,
    cache: &Arc<Mutex<Cache>>,
    last_activity: &Arc<Mutex<Instant>>, // UDS 쪽과 공유하는 "마지막 활동 시각"
) -> std::io::Result<()>;
```

- `tiny_http::Server::http(addr)`로 리슨, 요청마다 `request.as_reader()`로
  본문을 읽어 `serde_json::from_slice`로 파싱한다.
- `method()`가 `POST`이고 `url()`이 `/mcp`가 아니면 404.
- 파싱 실패/`handle_message`가 `None`을 돌려주면(알림류 메시지, 응답 없음)
  빈 200을 돌려준다(JSON-RPC 알림에 응답하지 않는 것과 같은 모양을 HTTP
  레벨에서도 지킨다 — 빈 바디, 상태 200).
- 성공하면 `handle_message`의 결과를 JSON 바디로, `Content-Type:
  application/json`과 상태 200으로 돌려준다.
- 매 요청 처리 전/후로 `last_activity.store(Instant::now())`.

### `crates/decide/src/daemon.rs`

- `serve_default()`를 바꿔, UDS `serve`(기존 로직을 그대로 함수로 분리—
  `serve_uds`로 이름만 바꾼다)와 새 `http::serve_http`를 각각 스레드로
  띄우고 `main`은 두 `JoinHandle`을 `join()`한다.
- `last_activity: Arc<Mutex<Instant>>`를 두 스레드가 공유한다.
  `accept_within`의 "요청이 없으면 idle 종료" 판단이 이 공유 타임스탬프를
  본다 — 한쪽 트랜스포트에만 요청이 와도 idle 타이머가 갱신된다. 둘 다
  idle이면 프로세스 전체가 종료된다(한쪽만 종료하고 다른 쪽은 살리는
  것은 하지 않는다 — 모델은 하나뿐이고 어느 트랜스포트가 쓰든 같은
  자원이다).
- `Env::from_process()`는 변경이 없는 값이라 두 스레드가 각자 자기
  스레드 안에서 한 번씩 다시 불러도(또는 `main`에서 한 번 만들어
  `Arc`로 공유해도) 결과가 같다. **`Transport`(`live_transport(&env)`가
  만드는 값)는 두 스레드가 각자 독립적으로 하나씩 만든다** — TypeSafe
  구현(`LiveTransport`)은 매 호출마다 새 HTTP 요청을 보내는 상태 없는
  값이라 두 개를 따로 둬도 동작이 같다. 로컬 백엔드 경로는 `Transport`를
  거치지 않고 `backend::decide`/`decide_many`가 `local::infer`를 직접
  부르며, 그 안의 `local::Runtime`은 `OnceLock`으로 프로세스 전체에
  하나뿐이고 `backbone`은 `Mutex`로 감싸여 있어 어느 스레드에서 불러도
  안전하게 직렬화된다 — 그래서 "`Transport`를 스레드마다 따로 둔다"가
  "모델을 두 번 로드한다"로 이어지지 않는다.
- **캐시(`Cache`, LRU 64개)는 `Arc<Mutex<Cache>>`로 두 스레드가 공유한다**
  — 각자 가지면 같은 질문이 트랜스포트에 따라 캐시 적중/미적중이 갈려
  일관성이 없어진다.

### `crates/decide/src/mcp.rs`

변경 없음. `handle_message`는 이미 `Env`/`Transport`를 받는 순수 함수라
HTTP 핸들러가 그대로 재사용한다.

### `crates/decide/src/main.rs`

- `run_mcp()`(stdio 루프)는 **삭제하지 않는다** — "범위 밖"의 호환성
  이유로 서브커맨드 `decide mcp`는 남겨 두되, 새로 설치하는 사용자는
  이걸 쓰지 않는다(`decide install`이 더 이상 이 경로로 등록하지 않음).
- `run_install()`을 바꿔 `claude mcp add -s user decide -- <bin> mcp`
  대신 `claude mcp add -s user --transport http decide
  http://127.0.0.1:48080/mcp`를 호출한다.
- `run_install_claude()`가 MCP 등록 뒤, 데몬이 안 떠 있으면
  `gate::client::spawn_daemon()`과 같은 방식으로 `decide daemon`을 한 번
  띄운다(TCP 포트 바인드 시도로 "이미 떠 있다" 판별 — UDS의 `claim_socket`
  과 같은 역할을 TCP 쪽에도 만든다, 아래 `daemon.rs` 변경 참고).

### `crates/decide/src/daemon.rs`의 포트 점유 확인

```rust
/// 48080 포트가 이미 쓰이고 있는지(다른 decide daemon이 떠 있는지) 확인한다.
/// UDS의 claim_socket과 같은 역할 — 바인드를 시도해 보고 실패하면 "이미 있다".
pub fn http_port_in_use(addr: &str) -> bool {
    std::net::TcpListener::bind(addr).is_err()
}
```

`main.rs::run_install_claude()`가 MCP 등록에 성공한 뒤 이 함수를 불러
`false`면(아무도 안 쓰고 있으면) `gate::client::spawn_daemon()`과 같은
방식으로 `decide daemon`을 자식 프로세스로 띄운다. `true`면(이미 떠
있으면) 아무것도 하지 않고 "이미 실행 중입니다"를 알린다. `serve_default()`
자신도 시작할 때 이 함수로 먼저 확인해, 이미 다른 `decide daemon`이
떠 있으면(UDS `claim_socket`이 이미 하던 체크에 더해) HTTP 포트 바인드도
건너뛰고 조용히 종료한다 — 지금 `serve`가 UDS만으로 하던 "이미 떠 있으면
새로 안 띄움" 판단을 UDS와 HTTP 둘 다 확인하도록 넓힌다.

## 테스트

- `http.rs`: `handle_message`를 가짜 `Transport`로 감싸고, `tiny_http`
  요청/응답을 흉내 낸 테스트 — 실제로는 `tiny_http::Server`를 테스트
  안에서 띄우고 `ureq`(이미 의존성에 있음, 클라이언트로 겸용 가능)로
  HTTP 호출을 보내는 통합 테스트 형태가 된다. 기존 `fake_daemon`
  (UDS용) 패턴과 짝이 되는 `fake_http_daemon`을 만든다.
  - 정상 JSON-RPC 요청 → 200 + 올바른 결과.
  - 잘못된 JSON 본문 → 지금 stdio 경로의 `-32700` 파싱 에러와 같은 모양.
  - `GET /mcp`나 다른 경로 → 404.
  - 두 번 연속 호출이 같은 모델 런타임(같은 `OnceLock`)을 쓰는지 —
    두 번째 호출의 지연이 첫 번째보다 뚜렷이 짧은지로 간접 확인(실제
    가중치가 필요해 `#[ignore]`).
- `daemon.rs`: UDS와 HTTP 양쪽에 요청을 보내고 idle 타이머가 어느 쪽
  요청으로도 갱신되는지(가짜 시계 주입 — 기존 daemon 테스트가 쓰는
  패턴을 그대로 따른다).
- `tests/cli.rs` 또는 새 통합 테스트: `decide install`이 `.mcp.json`에
  `type: "http"` + 올바른 `url`을 쓰는지(`claude` CLI 자체는 테스트에서
  모킹 — 지금 `run_install`도 실제 `claude mcp add` 실행 성공을 테스트
  환경에서 검증하지 않고 커맨드 조립만 테스트하는 방식을 따른다. 커맨드
  문자열을 만드는 부분을 함수로 뽑아 그 함수를 테스트한다).

## 문서

README/CLAUDE.md의 "`decide mcp`는 stdio로..." 설명을 "`decide daemon`이
HTTP(기본 `http://127.0.0.1:48080/mcp`)로 MCP를 서빙하고, 게이트는 같은
데몬의 UDS 소켓을 쓴다"로 고친다. `decide install` 안내에 "데몬이 꺼져
있으면 MCP 도구 호출이 실패한다. 재부팅 뒤에는 `decide daemon &`을 다시
실행하거나 `decide install --claude`를 다시 실행하라"를 추가한다.

## 범위 밖

- SSE/스트리밍 응답.
- 포트를 환경변수/config.toml로 바꾸는 기능.
- macOS 로그인 시 자동 기동(launchd).
- OAuth/인증(로컬 127.0.0.1 전용이라 지금은 불필요하다고 판단 — 외부에
  노출할 계획이 생기면 그때 다룬다).
- `decide mcp`(stdio) 서브커맨드 완전 제거 — 당장은 호환을 위해 남긴다.
- 데몬이 꺼져 있을 때 Claude Code 쪽에서 자동으로 띄우는 방법(MCP 클라이언트
  쪽 기능이 아니라 decide가 통제할 수 없는 영역).
