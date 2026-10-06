# daemon과 mcp를 하나의 HTTP 데몬으로 통합 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** `decide daemon`이 기존 UDS 소켓(게이트)과 새 TCP HTTP(MCP)를 하나의
프로세스/하나의 모델 런타임에서 동시에 서빙하게 해, 세션/재연결마다 모델이
중복 로드되는 문제를 없앤다. `decide install`이 `.mcp.json`을 자동으로
`type: "http"`로 등록한다.

**Architecture:** `tiny_http`(신규 의존성)로 `POST /mcp` 하나만 받는 최소
HTTP 서버를 만들어 `mcp::handle_message`(이미 `Env`/`Transport`를 받는 순수
함수)를 그대로 재사용한다. `decide daemon`이 UDS 서빙 스레드와 HTTP 서빙
스레드를 각각 띄우고 `main` 스레드가 둘을 join한다. 두 트랜스포트는 같은
`local::Runtime`(`OnceLock` + 내부 `Mutex<MlxBackbone>`)을 공유해 모델이
한 번만 로드되고, idle 타이머와 캐시도 두 트랜스포트가 공유한다.

**Tech Stack:** Rust, `tiny_http` 0.12(신규), 기존 `std::os::unix::net`,
`std::thread`, `std::sync::{Arc, Mutex}`.

**Spec:** `docs/superpowers/specs/2026-10-06-unified-http-daemon-design.md`

## Global Constraints

- HTTP 포트는 `127.0.0.1:48080`에 고정한다. 환경변수/config.toml로 바꾸는
  기능은 이 플랜에 없다.
- SSE/스트리밍 응답은 구현하지 않는다. `POST /mcp` 요청 하나 → 응답 하나.
- `decide mcp`(stdio) 서브커맨드는 **삭제하지 않는다** — 코드에 그대로
  남긴다. `decide install`만 더 이상 그 경로로 등록하지 않는다.
- idle 종료(30분)는 UDS와 HTTP 두 트랜스포트의 활동을 합쳐서 판단한다 —
  한쪽에만 요청이 와도 타이머가 갱신되고, 둘 다 idle이어야 종료한다.
- 모델 캐시(`Cache`, LRU 64개)는 두 트랜스포트가 공유한다 — 각자 가지면
  안 된다(같은 질문의 캐시 적중률이 트랜스포트에 따라 갈리면 안 됨).
- `Transport`(`live_transport(&env)`가 만드는 `LiveTransport`)는 두
  스레드가 각자 독립적으로 하나씩 만든다 — 상태 없는 HTTP 클라이언트라
  공유할 필요가 없다. 로컬 백엔드 경로(`backend::decide`/`decide_many`가
  `local::infer`를 직접 호출)는 `Transport`를 거치지 않으므로 이 구분과
  무관하다.
- macOS 로그인 시 자동 기동(launchd)은 이 플랜에 없다. `decide install
  --claude`가 설치 시점에 데몬을 한 번 띄우는 것으로 충분하다 — 재부팅
  뒤에는 사용자가 수동으로 다시 띄워야 한다.

## Review Focus

- **`decide daemon`을 두 번 띄우려는 경우(이미 하나가 떠 있을 때)** — UDS
  소켓 점유 확인(`claim_socket`)과 HTTP 포트 점유 확인(새 `http_port_in_use`)
  둘 다 "이미 떠 있다"로 판정해야 한다. 하나만 확인하면, UDS가 비어 있어도
  HTTP가 이미 떠 있는 상황(또는 그 반대)에서 두 번째 프로세스가 포트
  바인드에 실패해 패닉하거나 조용히 죽을 수 있다.
- **HTTP로 들어온 요청의 본문이 깨진 JSON이거나 빈 바디인 경우** — UDS
  경로가 이미 처리하는 "파싱 실패 시 JSON-RPC 에러 응답"과 같은 수준의
  처리가 HTTP 경로에도 있어야 한다. 처리 없이 패닉하면 그 요청 하나가
  서버 스레드 전체를 죽일 위험이 있다.
- **`POST /mcp`가 아닌 다른 메서드/경로로 요청이 오는 경우**(헬스체크,
  브라우저의 우연한 접속 등) — 404를 깔끔히 돌려줘야 하고, 이 요청 때문에
  idle 타이머가 갱신되거나 서버가 죽으면 안 된다.
- **UDS 요청과 HTTP 요청이 거의 동시에 들어오는 경우**(두 스레드가 동시에
  `local::infer`를 호출) — 백본의 `Mutex`가 직렬화하므로 결과가 틀리면
  안 되고, 한쪽이 다른 쪽을 영원히 블로킹해 데드락이 나면 안 된다.
- **`decide install`을 두 번 실행하는 경우** — `claude mcp add`가 멱등하게
  동작해야 하고(기존 `run_install`이 이미 "already exists"를 성공으로
  처리하는 패턴을 그대로 따름), 데몬을 중복으로 띄우려 하면 안 된다
  (포트 점유 확인으로 막힘).

---

## File Structure

| 파일 | 역할 |
| --- | --- |
| `crates/decide/src/http.rs` (신규) | HTTP 트랜스포트: `POST /mcp` 요청을 받아 `mcp::handle_message`로 넘기고 응답을 돌려준다. |
| `crates/decide/src/daemon.rs` (수정) | UDS 서빙 로직을 `serve_uds`로 이름을 바꾸고, 공유 idle 타이머/캐시를 두 트랜스포트가 쓰도록 `serve_default`를 재구성. 포트 점유 확인 함수 추가. |
| `crates/decide/src/main.rs` (수정) | `run_install`이 HTTP 등록을 쓰도록, `run_install_claude`가 데몬을 띄우도록 변경. |
| `crates/decide/Cargo.toml` (수정) | `tiny_http = "0.12"` 추가. |
| `README.md`, `CLAUDE.md` (수정) | mcp/daemon 아키텍처 설명 갱신. |

---

## Task 1: `tiny_http` 의존성 추가와 최소 HTTP 핸들러

**Files:**
- Modify: `crates/decide/Cargo.toml`
- Create: `crates/decide/src/http.rs`
- Modify: `crates/decide/src/lib.rs` (모듈 선언 추가)

**Interfaces:**
- Consumes: `crate::mcp::handle_message(message: &Value, env: &Env, transport: &mut T) -> Option<Value>`
  (이미 존재, 변경 없음). `crate::backend::Env`, `crate::typesafe::Transport`.
- Produces:
  - `pub fn handle_http_body<T: Transport>(body: &[u8], env: &Env, transport: &mut T) -> (u16, String)`
    — HTTP 바디 바이트를 받아 `(status, response_body)`를 돌려주는 순수 함수.
    이후 태스크(`serve_http`)가 이 함수를 네트워크 I/O에 연결한다.
  - 상수 `pub const MCP_PATH: &str = "/mcp";`

- [ ] **Step 1: Cargo.toml에 의존성 추가**

`crates/decide/Cargo.toml`의 `[dependencies]`에 추가:

```toml
tiny_http = "0.12"
```

- [ ] **Step 2: 실패하는 테스트 먼저 — handle_http_body의 세 가지 경로**

`crates/decide/src/http.rs`를 새로 만들고 테스트부터 쓴다:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::Env;
    use crate::typesafe::{RawResponse, Transport};
    use serde_json::json;
    use std::cell::Cell;

    struct Script {
        calls: Cell<usize>,
    }

    impl Transport for Script {
        fn post_json(&mut self, _body: &serde_json::Value) -> Result<RawResponse, String> {
            self.calls.set(self.calls.get() + 1);
            Ok(RawResponse {
                status: 200,
                body: r#"{"model":"jev-1.13.0","answers":{"q":{"type":"noul","noul":0.8}}}"#.into(),
            })
        }
    }

    fn env_typesafe() -> Env {
        Env {
            backend: None,
            api_key: Some("k".into()),
        }
    }

    #[test]
    fn valid_rpc_body_returns_200_and_the_jsonrpc_result() {
        let mut script = Script { calls: Cell::new(0) };
        let body = json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "tools/call",
            "params": {
                "name": "decide",
                "arguments": {"state": "s", "type": "noul", "instructions": "참인가?"}
            }
        })
        .to_string();
        let (status, response) = handle_http_body(body.as_bytes(), &env_typesafe(), &mut script);
        assert_eq!(status, 200);
        let parsed: serde_json::Value = serde_json::from_str(&response).unwrap();
        assert_eq!(parsed["result"]["isError"], false);
        assert_eq!(script.calls.get(), 1);
    }

    #[test]
    fn invalid_json_body_returns_200_with_a_parse_error_payload() {
        // stdio 경로(main.rs::run_mcp)가 파싱 실패에 -32700을 그대로 돌려주는 것과 같은 모양 —
        // JSON-RPC 레벨 에러는 HTTP 상태 코드가 아니라 응답 바디로 표현한다.
        let mut script = Script { calls: Cell::new(0) };
        let (status, response) = handle_http_body(b"not json", &env_typesafe(), &mut script);
        assert_eq!(status, 200);
        let parsed: serde_json::Value = serde_json::from_str(&response).unwrap();
        assert_eq!(parsed["error"]["code"], -32700);
        assert_eq!(script.calls.get(), 0);
    }

    #[test]
    fn a_notification_with_no_id_returns_200_with_an_empty_body() {
        // handle_message가 알림류(id 없음)에 None을 돌려준다 — HTTP 레벨에서는
        // 빈 200으로 답한다(JSON-RPC가 알림에 응답하지 않는 것과 같은 뜻).
        let mut script = Script { calls: Cell::new(0) };
        let body = json!({"jsonrpc": "2.0", "method": "notifications/initialized"}).to_string();
        let (status, response) = handle_http_body(body.as_bytes(), &env_typesafe(), &mut script);
        assert_eq!(status, 200);
        assert_eq!(response, "");
        assert_eq!(script.calls.get(), 0);
    }
}
```

- [ ] **Step 2.5: 모듈 선언 추가**

`crates/decide/src/lib.rs`에 `pub mod http;`를 다른 `pub mod` 선언들과
알파벳 순서로 추가(`help` 다음, `local` 전):

```rust
pub mod help;
pub mod http;
pub mod local;
```

- [ ] **Step 3: 테스트 실행해 실패 확인**

Run: `cargo test --manifest-path crates/decide/Cargo.toml http:: 2>&1 | tail -30`
Expected: FAIL — `handle_http_body`, `MCP_PATH`가 아직 없어 컴파일 오류.

- [ ] **Step 4: 최소 구현 작성**

`crates/decide/src/http.rs`의 테스트 모듈 위에 추가:

```rust
// decide daemon의 HTTP 트랜스포트: Claude Code가 MCP를 type="http"로 거는
// 창구다. POST /mcp 하나만 받고, mcp::handle_message로 넘겨 응답을 그대로
// 돌려준다. SSE는 쓰지 않는다 — decide의 모든 응답은 요청 하나당 결과
// 하나뿐이라 스트리밍할 게 없다.
use crate::backend::Env;
use crate::mcp::handle_message;
use crate::typesafe::Transport;
use serde_json::Value;

pub const MCP_PATH: &str = "/mcp";

/// HTTP 요청 바디(JSON-RPC 메시지 하나)를 처리해 `(상태 코드, 응답 바디)`를 돌려준다.
/// JSON 파싱 실패나 핸들러의 JSON-RPC 레벨 에러는 HTTP 상태 코드가 아니라 응답
/// 바디(JSON-RPC error 객체)로 표현한다 — stdio 경로(main.rs::run_mcp)와 같은 관례.
pub fn handle_http_body<T: Transport>(
    body: &[u8],
    env: &Env,
    transport: &mut T,
) -> (u16, String) {
    let message: Value = match serde_json::from_slice(body) {
        Ok(message) => message,
        Err(err) => {
            let error = serde_json::json!({
                "jsonrpc": "2.0",
                "id": null,
                "error": {"code": -32700, "message": err.to_string()},
            });
            return (200, error.to_string());
        }
    };
    match handle_message(&message, env, transport) {
        Some(response) => (200, response.to_string()),
        None => (200, String::new()),
    }
}
```

- [ ] **Step 5: 테스트 실행해 통과 확인**

Run: `cargo test --manifest-path crates/decide/Cargo.toml http:: 2>&1 | tail -30`
Expected: PASS — 3개 테스트 모두 통과.

- [ ] **Step 6: 커밋**

```bash
git add crates/decide/Cargo.toml crates/decide/Cargo.lock crates/decide/src/http.rs crates/decide/src/lib.rs
git commit -m "feat(http): HTTP 바디를 처리하는 순수 핸들러 handle_http_body를 추가한다"
```

---

## Task 2: `tiny_http` 서버 루프 — `serve_http`

**Files:**
- Modify: `crates/decide/src/http.rs`

**Interfaces:**
- Consumes: Task 1의 `handle_http_body`, `MCP_PATH`. `tiny_http::{Server, Request, Response}`
  (외부 crate).
- Produces:
  - `pub fn serve_http<T: Transport>(addr: &str, env: &Env, transport: &mut T, last_activity: &std::sync::Arc<std::sync::Mutex<std::time::Instant>>, idle: std::time::Duration) -> std::io::Result<()>`
    — Task 3(`daemon.rs`)이 스레드 안에서 호출한다.
  - `pub fn http_port_in_use(addr: &str) -> bool` — Task 4가 포트 점유 확인에 쓴다.

- [ ] **Step 1: 실패하는 테스트 먼저 — 실제 HTTP 요청/응답 왕복**

`crates/decide/src/http.rs`의 `mod tests`에 추가(이 테스트들은 실제 TCP
소켓을 열므로 포트 충돌을 피하려고 `0`(OS가 빈 포트를 고름)을 쓰고,
`Server::server_addr()`로 실제 바인드된 포트를 읽는다):

```rust
    use std::sync::{Arc, Mutex};
    use std::time::{Duration, Instant};

    fn start_test_server() -> (String, std::thread::JoinHandle<()>) {
        let server = tiny_http::Server::http("127.0.0.1:0").unwrap();
        let addr = format!("127.0.0.1:{}", server.server_addr().to_ip().unwrap().port());
        let last_activity = Arc::new(Mutex::new(Instant::now()));
        let handle = std::thread::spawn(move || {
            let mut script = Script { calls: Cell::new(0) };
            serve_on(server, &env_typesafe(), &mut script, &last_activity, Duration::from_millis(500));
        });
        (addr, handle)
    }

    #[test]
    fn a_post_to_mcp_path_returns_the_jsonrpc_result() {
        let (addr, _handle) = start_test_server();
        let body = json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "tools/call",
            "params": {
                "name": "decide",
                "arguments": {"state": "s", "type": "noul", "instructions": "참인가?"}
            }
        });
        let response = ureq::post(&format!("http://{addr}/mcp"))
            .send_json(body)
            .unwrap();
        assert_eq!(response.status(), 200);
        let parsed: serde_json::Value = response.into_json().unwrap();
        assert_eq!(parsed["result"]["isError"], false);
    }

    #[test]
    fn a_get_or_wrong_path_returns_404() {
        let (addr, _handle) = start_test_server();
        let response = ureq::get(&format!("http://{addr}/other")).call();
        let status = match response {
            Ok(resp) => resp.status(),
            Err(ureq::Error::Status(status, _)) => status,
            Err(err) => panic!("예상치 못한 전송 오류: {err}"),
        };
        assert_eq!(status, 404);
    }

    #[test]
    fn http_port_in_use_detects_a_bound_port() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = format!("127.0.0.1:{}", listener.local_addr().unwrap().port());
        assert!(http_port_in_use(&addr));
        drop(listener);
        // 포트를 닫은 뒤 OS가 즉시 재사용 가능하게 두는 보장은 없어, 바인드 재시도를 몇 번 허용한다.
        let freed = (0..20).any(|_| {
            let ok = !http_port_in_use(&addr);
            if !ok {
                std::thread::sleep(Duration::from_millis(20));
            }
            ok
        });
        assert!(freed, "포트가 해제되지 않았다");
    }
```

이 플랜은 `serve_http`가 내부적으로 "이미 만들어진 `tiny_http::Server`를
받아 서빙하는" `serve_on` 헬퍼로 쪼개지는 것을 전제로 테스트를 짰다 —
`serve_http(addr, ...)`는 `Server::http(addr)`로 서버를 만든 뒤 `serve_on`을
호출하는 공개 함수이고, 테스트는 포트 `0`(임의 할당)을 쓰기 위해 `serve_on`을
직접 부른다. Step 4에서 이 구조로 구현한다.

- [ ] **Step 2: 테스트 실행해 실패 확인**

Run: `cargo test --manifest-path crates/decide/Cargo.toml http:: 2>&1 | tail -30`
Expected: FAIL — `serve_on`, `http_port_in_use`가 아직 없어 컴파일 오류.

- [ ] **Step 3: ureq를 dev-dependency로도 쓸 수 있는지 확인**

`ureq`는 이미 `[dependencies]`에 있으므로(`crates/decide/Cargo.toml`) 테스트
코드에서 그냥 쓸 수 있다 — 추가 설정이 필요 없다.

- [ ] **Step 4: 구현 작성**

`crates/decide/src/http.rs`에 `handle_http_body` 다음에 추가:

```rust
/// 48080(또는 지정한 주소) 포트가 이미 쓰이고 있는지 확인한다. UDS의
/// claim_socket과 같은 역할 — 바인드를 시도해 보고 실패하면 "이미 있다".
pub fn http_port_in_use(addr: &str) -> bool {
    std::net::TcpListener::bind(addr).is_err()
}

/// `addr`에서 HTTP 서버를 새로 만들어 `serve_on`으로 넘긴다.
pub fn serve_http<T: Transport>(
    addr: &str,
    env: &Env,
    transport: &mut T,
    last_activity: &std::sync::Arc<std::sync::Mutex<std::time::Instant>>,
    idle: std::time::Duration,
) -> std::io::Result<()> {
    let server = tiny_http::Server::http(addr)
        .map_err(|err| std::io::Error::other(err.to_string()))?;
    serve_on(server, env, transport, last_activity, idle);
    Ok(())
}

/// 이미 만들어진 서버로 요청을 받아 처리한다. `recv_timeout`이 idle 시간 안에 요청이
/// 없으면 `Ok(None)`을 돌려주므로, 그때마다 공유 idle 타이머가 충분히 오래됐는지 본다
/// — 두 트랜스포트(UDS/HTTP) 중 하나라도 최근에 활동했으면 계속 돈다.
fn serve_on<T: Transport>(
    server: tiny_http::Server,
    env: &Env,
    transport: &mut T,
    last_activity: &std::sync::Arc<std::sync::Mutex<std::time::Instant>>,
    idle: std::time::Duration,
) {
    loop {
        match server.recv_timeout(std::time::Duration::from_secs(1)) {
            Ok(Some(request)) => {
                handle_one(request, env, transport);
                *last_activity.lock().unwrap() = std::time::Instant::now();
            }
            Ok(None) => {
                let elapsed = last_activity.lock().unwrap().elapsed();
                if elapsed >= idle {
                    break;
                }
            }
            Err(_) => break,
        }
    }
}

fn handle_one<T: Transport>(mut request: tiny_http::Request, env: &Env, transport: &mut T) {
    if request.method() != &tiny_http::Method::Post || request.url() != MCP_PATH {
        let _ = request.respond(tiny_http::Response::empty(404));
        return;
    }
    let mut body = Vec::new();
    if std::io::Read::read_to_end(request.as_reader(), &mut body).is_err() {
        let _ = request.respond(tiny_http::Response::empty(400));
        return;
    }
    let (status, payload) = handle_http_body(&body, env, transport);
    let response = tiny_http::Response::from_string(payload)
        .with_status_code(status)
        .with_header(
            "Content-Type: application/json".parse::<tiny_http::Header>().unwrap(),
        );
    let _ = request.respond(response);
}
```

`recv_timeout`을 1초로 짧게 잡는 이유: idle 데드라인(30분) 자체는 이
타임아웃 값이 아니라 `last_activity`와 `idle` 비교로 결정되고, 이 짧은
타임아웃은 "주기적으로 깨어나 idle 여부를 재확인하는 간격"일 뿐이다 —
UDS의 `accept_within`이 매번 "남은 시간"을 정확히 계산해 쓰는 것과
달리, 두 트랜스포트가 공유하는 타이머를 다루므로 이 쪽이 더 단순하다.

- [ ] **Step 5: 테스트 실행해 통과 확인**

Run: `cargo test --manifest-path crates/decide/Cargo.toml http:: -- --test-threads=1 2>&1 | tail -40`
Expected: PASS — Task 1의 3개 + 이번 3개 = 6개 통과. `--test-threads=1`은
`http_port_in_use_detects_a_bound_port`가 포트를 점유/해제하는 타이밍이
다른 병렬 테스트와 겹치지 않게 하기 위함(이 저장소의 UDS 소켓 테스트들도
같은 이유로 짧은 임시 이름을 쓴다).

- [ ] **Step 6: 커밋**

```bash
git add crates/decide/src/http.rs
git commit -m "feat(http): tiny_http로 /mcp POST 요청을 받는 서버 루프를 추가한다"
```

---

## Task 3: `daemon.rs` — UDS와 HTTP를 한 프로세스에서 함께 서빙

**Files:**
- Modify: `crates/decide/src/daemon.rs`

**Interfaces:**
- Consumes: Task 2의 `http::serve_http`, `http::http_port_in_use`. 기존
  `daemon::claim_socket`, `daemon::Cache`, `backend::{Env, live_transport}`.
- Produces: `pub fn serve_default() -> std::io::Result<()>`의 동작 변경(시그니처
  동일). 새 공개 상수 `pub const DEFAULT_HTTP_ADDR: &str = "127.0.0.1:48080";`.
  Task 4가 이 상수를 쓴다.

- [ ] **Step 1: 기존 `serve`를 `serve_uds`로 이름만 바꾼다(동작 변경 없음)**

`crates/decide/src/daemon.rs`에서 `pub fn serve(path: &Path, idle: Duration)`를
`pub fn serve_uds(path: &Path, idle: Duration, cache: Arc<Mutex<Cache>>)`로
바꾼다 — 지금 `serve` 내부가 로컬 변수로 만들던 `Cache::default()`를
파라미터로 받게 바꾸는 것 외에는 로직을 그대로 둔다:

```rust
pub fn serve_uds(path: &Path, idle: Duration, cache: Arc<Mutex<Cache>>) -> std::io::Result<()> {
    if !claim_socket(path)? {
        eprintln!("daemon already running at {}", path.display());
        return Ok(());
    }
    let listener = UnixListener::bind(path)?;
    let _guard = SocketGuard(path.to_path_buf());
    listener.set_nonblocking(true)?;
    let env = Env::from_process();
    let mut transport = live_transport(&env);
    loop {
        match accept_within(&listener, idle)? {
            Some(stream) => {
                if !handle_connection_shared(stream, &env, &mut transport, &cache) {
                    break;
                }
            }
            None => break,
        }
    }
    Ok(())
}
```

`handle_connection`도 `cache: &mut Cache` 대신 `cache: &Arc<Mutex<Cache>>`를
받는 `handle_connection_shared`로 바꾼다(내부에서 `cache.lock().unwrap()`로
잠깐 잠그고 `call`에 `&mut *locked`를 넘긴다). 이 리팩터링은 동작을 바꾸지
않으므로 새 테스트를 추가하지 않고, 기존 테스트(`line_protocol_returns_one_json_object`
등)가 모두 그대로 통과하는지로 검증한다. `Arc`/`Mutex`는 파일 상단
`use std::sync::{Arc, Mutex};`로 가져온다.

이 플랜의 Review Focus(동시 접근)를 지키려면 `call`을 부르는 동안 락을
오래 쥐지 않는 게 이상적이지만, 백본 자체가 이미 `Mutex`로 직렬화되므로
캐시 락을 요청 처리 전체에 걸쳐 쥐어도(한 트랜스포트가 처리 중일 때
다른 트랜스포트의 캐시 조회가 잠깐 기다리는 정도) 교착 상태는 생기지
않는다 — 캐시 락과 백본 락 사이에 순환 대기가 없다(캐시 락 안에서
백본 락을 또 요청하는 코드가 없다).

- [ ] **Step 2: 변경 후 기존 테스트가 그대로 통과하는지 확인**

Run: `cargo test --manifest-path crates/decide/Cargo.toml daemon:: 2>&1 | tail -60`
Expected: PASS — 기존 데몬 테스트 전부(`line_protocol_returns_one_json_object`,
`identical_requests_hit_the_cache_and_skip_the_transport`,
`claim_replaces_a_dead_file_and_keeps_a_live_socket`,
`a_matching_client_version_is_answered_normally_and_keeps_serving`,
`a_different_client_version_gets_a_stale_reply_without_a_backend_call`,
`a_request_without_a_client_version_behaves_as_before`,
`serve_replies_stale_then_exits_and_unlinks_the_socket`,
`a_client_that_sends_its_request_late_still_gets_an_answer`,
`idle_timeout_unlinks_the_socket`, `questions_line_returns_answers_and_hits_the_cache`,
`reordered_questions_are_a_different_cache_entry`,
`type_together_with_questions_is_rejected_without_a_call`,
`single_and_many_with_the_same_content_do_not_share_a_cache_entry`)가
`serve`를 직접 호출하던 지점을 `serve_uds`로 바꿔야 컴파일된다 — 이
스텝에서 테스트 코드 안의 호출부도 함께 고친다(`serve(&path, dur)` →
`serve_uds(&path, dur, Arc::new(Mutex::new(Cache::default())))`).

- [ ] **Step 3: 실패하는 테스트 먼저 — `serve_default`가 UDS와 HTTP를 함께 뜬다**

같은 파일 `mod tests`에 추가:

```rust
    #[test]
    fn serve_default_binds_both_uds_and_http_and_both_work() {
        // 기본 소켓/포트 경로를 쓰면 다른 테스트나 실제 데몬과 충돌하므로, 이 테스트는
        // serve_default가 아니라 그 내부 로직을 그대로 쓰는 임시 경로/포트로 돈다.
        let dir = std::env::temp_dir().join(format!(
            "decide-unified-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let socket_path = dir.join("d.sock");
        let http_addr = "127.0.0.1:0"; // 임의 포트 — 이 테스트는 serve_unified를 직접 호출한다.

        // serve_unified(uds_path, http_addr, idle) -> std::io::Result<()>는 이 Step에서 구현한다.
        // 테스트는 그 함수가 "두 트랜스포트 모두에서 답한다"를 검증한다. 실제 포트 번호를
        // 알아내려면 serve_unified가 바인드한 TcpListener/Server의 주소를 돌려줄 통로가
        // 필요하므로, 테스트 전용으로 포트를 고정값(예: 127.0.0.1:48099, 48080과 겹치지
        // 않는 값)으로 둔다 — 실제 운영에서 쓰는 DEFAULT_HTTP_ADDR과는 다른 값이라
        // 동시에 떠 있는 실제 데몬과 충돌하지 않는다.
        let http_addr = "127.0.0.1:48099";
        let socket_for_thread = socket_path.clone();
        let server = std::thread::spawn(move || {
            serve_unified(&socket_for_thread, http_addr, Duration::from_secs(5))
        });

        let started = Instant::now();
        while !socket_path.exists() {
            assert!(started.elapsed() < Duration::from_secs(5), "UDS 소켓이 생기지 않았다");
            std::thread::sleep(Duration::from_millis(10));
        }
        // UDS 쪽: 기존 line protocol로 확인.
        let mut stream = UnixStream::connect(&socket_path).unwrap();
        writeln!(stream, "{}", with_client_version("0.0.1")).unwrap();
        let mut reply = String::new();
        BufReader::new(stream).read_line(&mut reply).unwrap();
        assert_eq!(
            serde_json::from_str::<Value>(&reply).unwrap(),
            json!({"stale": true, "version": VERSION})
        );

        // HTTP 쪽: 포트가 열릴 때까지 재시도.
        let mut http_ok = false;
        for _ in 0..50 {
            if let Ok(response) = ureq::post(&format!("http://{http_addr}/mcp")).send_json(json!({
                "jsonrpc": "2.0", "id": 1, "method": "tools/list"
            })) {
                let parsed: Value = response.into_json().unwrap();
                assert_eq!(parsed["result"]["tools"][0]["name"], "decide");
                http_ok = true;
                break;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        assert!(http_ok, "HTTP 트랜스포트가 응답하지 않았다");

        server.join().unwrap().unwrap();
        let _ = std::fs::remove_dir_all(&dir);
    }
```

- [ ] **Step 4: 테스트 실행해 실패 확인**

Run: `cargo test --manifest-path crates/decide/Cargo.toml daemon::tests::serve_default_binds 2>&1 | tail -30`
Expected: FAIL — `serve_unified`가 아직 없어 컴파일 오류.

- [ ] **Step 5: `serve_unified` 구현**

`crates/decide/src/daemon.rs`에서 `pub fn serve_default()` 바로 앞에 추가,
`serve_default`도 함께 바꾼다:

```rust
pub const DEFAULT_HTTP_ADDR: &str = "127.0.0.1:48080";

/// UDS(게이트)와 HTTP(MCP)를 같은 프로세스에서 함께 서빙한다. 둘 다 같은
/// Env/캐시를 공유하고, 둘 중 하나라도 최근에 활동했으면 idle 데드라인이
/// 갱신된다 — 두 트랜스포트 모두 `idle` 동안 조용해야 프로세스가 끝난다.
pub fn serve_unified(
    uds_path: &Path,
    http_addr: &str,
    idle: Duration,
) -> std::io::Result<()> {
    if !claim_socket(uds_path)? {
        eprintln!("daemon already running (uds)");
        return Ok(());
    }
    if crate::http::http_port_in_use(http_addr) {
        eprintln!("daemon already running (http)");
        // UDS는 이미 claim했으므로 소켓 가드를 명시적으로 치운다.
        let _ = std::fs::remove_file(uds_path);
        return Ok(());
    }
    let cache = Arc::new(Mutex::new(Cache::default()));
    let last_activity = Arc::new(Mutex::new(Instant::now()));

    let uds_cache = cache.clone();
    let uds_activity = last_activity.clone();
    let uds_path_owned = uds_path.to_path_buf();
    let uds_handle = std::thread::spawn(move || {
        serve_uds_with_activity(&uds_path_owned, idle, uds_cache, uds_activity)
    });

    let http_cache = cache.clone();
    let http_activity = last_activity.clone();
    let http_addr_owned = http_addr.to_string();
    let http_handle = std::thread::spawn(move || {
        let env = Env::from_process();
        let mut transport = live_transport(&env);
        crate::http::serve_http(&http_addr_owned, &env, &mut transport, &http_activity, idle)
    });

    uds_handle.join().unwrap()?;
    http_handle.join().unwrap()?;
    let _ = cache; // 두 Arc 클론 중 마지막 참조가 여기서 자연히 drop된다.
    Ok(())
}

pub fn serve_default() -> std::io::Result<()> {
    serve_unified(&default_socket_path(), DEFAULT_HTTP_ADDR, Duration::from_secs(30 * 60))
}
```

`serve_uds_with_activity`는 Step 1의 `serve_uds`에 "공유 idle 타이머"
파라미터를 더한 변형이다 — `accept_within`이 지금은 자체 데드라인
(`Instant::now() + idle`)을 쓰는데, 이걸 공유 타이머 기준으로 바꾼다:

```rust
fn serve_uds_with_activity(
    path: &Path,
    idle: Duration,
    cache: Arc<Mutex<Cache>>,
    last_activity: Arc<Mutex<Instant>>,
) -> std::io::Result<()> {
    let listener = UnixListener::bind(path)?;
    let _guard = SocketGuard(path.to_path_buf());
    listener.set_nonblocking(true)?;
    let env = Env::from_process();
    let mut transport = live_transport(&env);
    loop {
        match accept_within(&listener, Duration::from_secs(1))? {
            Some(stream) => {
                if !handle_connection_shared(stream, &env, &mut transport, &cache) {
                    break;
                }
                *last_activity.lock().unwrap() = Instant::now();
            }
            None => {
                if last_activity.lock().unwrap().elapsed() >= idle {
                    break;
                }
            }
        }
    }
    Ok(())
}
```

이 함수는 `claim_socket`을 호출하지 않는다(이미 `serve_unified`가 먼저
확인했다) — `UnixListener::bind`만 한다. `accept_within`의 타임아웃을
1초로 짧게 주는 것은 Task 2의 HTTP 쪽과 같은 이유(주기적 재확인 간격)다.

**Task 1의 `serve_uds`(Step 1에서 만든 것)는 이 Step에서 `serve_uds_with_activity`로
대체되므로 삭제한다** — 두 버전을 모두 남겨 두면 어느 쪽이 실제로
쓰이는지 혼란스럽다. Step 1~2에서 그 함수로 옮겼던 기존 테스트 호출부
(`line_protocol_returns_one_json_object` 등)는 다시 고쳐, 각 테스트가
직접 `UnixListener::bind` + `handle_connection_shared`를 쓰는 가장 작은
형태로 단순화하거나, `serve_uds_with_activity`에 임의의 `Arc<Mutex<Instant>>`를
만들어 넘겨 호출한다 — 테스트 하나하나가 이미 "한 줄 요청에 한 줄 응답"만
검증하므로 어느 쪽이든 기존 단언은 그대로 유지된다.

- [ ] **Step 6: 테스트 실행해 통과 확인**

Run: `cargo test --manifest-path crates/decide/Cargo.toml daemon:: -- --test-threads=1 2>&1 | tail -80`
Expected: PASS — Step 2에서 고친 기존 테스트 전부 + 이번 `serve_default_binds_both_uds_and_http_and_both_work`.
`--test-threads=1`은 데몬 테스트들이 실제 소켓/포트를 열기 때문에 이
파일에서는 원래도 암묵적으로 순서에 민감하다(짧은 임시 디렉터리 이름
충돌 방지와 같은 이유).

- [ ] **Step 7: 커밋**

```bash
git add crates/decide/src/daemon.rs
git commit -m "feat(daemon): UDS와 HTTP를 한 프로세스에서 함께 서빙한다(serve_unified)"
```

---

## Task 4: `decide install`이 HTTP로 등록하고 데몬을 띄운다

**Files:**
- Modify: `crates/decide/src/main.rs`

**Interfaces:**
- Consumes: Task 3의 `daemon::DEFAULT_HTTP_ADDR`, `http::http_port_in_use`.
  기존 `claude::settings_path`, `claude::hook_specs`, `claude::install_hooks`.
- Produces: `run_install()`, `run_install_claude()`의 동작 변경(시그니처 동일).

- [ ] **Step 1: 실패하는 테스트 먼저 — install 커맨드 조립**

지금 `run_install`은 `Command::new("claude").args([...])`를 직접 실행해서
테스트하기 어렵다(실제 `claude` CLI가 필요). 커맨드 인자 조립 부분을
먼저 함수로 뽑고 그 함수를 테스트한다. `crates/decide/src/main.rs`
맨 아래, 기존 `#[cfg(test)]`가 없다면 새로 만든다:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn install_mcp_args_register_http_transport_at_the_fixed_port() {
        let args = install_mcp_args();
        assert_eq!(
            args,
            vec![
                "mcp", "add", "-s", "user", "--transport", "http", "decide",
                "http://127.0.0.1:48080/mcp",
            ]
        );
    }
}
```

- [ ] **Step 2: 테스트 실행해 실패 확인**

Run: `cargo test --manifest-path crates/decide/Cargo.toml main::tests::install_mcp_args 2>&1 | tail -20`

(참고: `main.rs`는 바이너리 크레이트 루트라 통합 테스트에서 직접 `use`할
수 없다 — 이 테스트는 `main.rs` 안의 `#[cfg(test)] mod tests`로 두고
`cargo test --bin decide`로 돈다. `Cargo.toml`에 `[[bin]]` 섹션이 이미
있으므로 추가 설정은 필요 없다.)

Expected: FAIL — `install_mcp_args`가 아직 없어 컴파일 오류.

- [ ] **Step 3: 구현 — `install_mcp_args` 추출과 `run_install`/`run_install_claude` 변경**

`crates/decide/src/main.rs`의 기존 `fn run_install() -> io::Result<()>`를
교체:

```rust
/// `claude mcp add`에 넘길 인자. HTTP transport로 고정 포트(daemon::DEFAULT_HTTP_ADDR)에 등록한다.
fn install_mcp_args() -> Vec<String> {
    vec![
        "mcp".to_string(),
        "add".to_string(),
        "-s".to_string(),
        "user".to_string(),
        "--transport".to_string(),
        "http".to_string(),
        "decide".to_string(),
        format!("http://{}/mcp", daemon::DEFAULT_HTTP_ADDR),
    ]
}

fn run_install() -> io::Result<()> {
    let output = Command::new("claude")
        .args(install_mcp_args())
        .output()
        .map_err(|err| {
            if err.kind() == io::ErrorKind::NotFound {
                io::Error::new(
                    io::ErrorKind::NotFound,
                    "claude 명령을 찾을 수 없습니다. Claude Code CLI가 설치돼 있는지 확인하세요",
                )
            } else {
                err
            }
        })?;
    io::stdout().write_all(&output.stdout)?;
    io::stderr().write_all(&output.stderr)?;
    if output.status.success() {
        return Ok(());
    }
    let already_registered = [&output.stdout[..], &output.stderr[..]]
        .iter()
        .any(|bytes| String::from_utf8_lossy(bytes).contains("already exists"));
    if already_registered {
        return Ok(());
    }
    Err(io::Error::other("claude mcp add 실행이 실패했습니다"))
}
```

`DECIDE_BIN` 상수(`main.rs:10`)는 더 이상 `run_install`에서 쓰이지 않지만
`run_install_claude`의 `claude::hook_command(DECIDE_BIN)`/`claude::gate_command(DECIDE_BIN)`
에서 여전히 쓰이므로 그대로 둔다.

- [ ] **Step 4: 테스트 실행해 통과 확인**

Run: `cargo test --manifest-path crates/decide/Cargo.toml main::tests::install_mcp_args 2>&1 | tail -20`
Expected: PASS.

- [ ] **Step 5: 실패하는 테스트 먼저 — `run_install_claude`가 데몬을 띄운다**

`run_install_claude`가 데몬 기동을 시도하는지는 실제 프로세스 fork를
확인하기 어려우므로(통합 테스트에서 실제로 포트를 뜨게 하는 것은
무겁다), **포트 점유 확인 로직만 함수로 뽑아 테스트**한다:

```rust
    #[test]
    fn spawn_daemon_if_needed_skips_when_the_port_is_already_bound() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = format!("127.0.0.1:{}", listener.local_addr().unwrap().port());
        let mut spawned = 0;
        spawn_daemon_if_needed(&addr, &mut || spawned += 1);
        assert_eq!(spawned, 0, "포트가 이미 쓰이고 있으면 다시 띄우지 않는다");
    }

    #[test]
    fn spawn_daemon_if_needed_spawns_when_the_port_is_free() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = format!("127.0.0.1:{}", listener.local_addr().unwrap().port());
        drop(listener); // 포트를 비운다.
        let freed = (0..20).any(|_| {
            let ok = !decide::http::http_port_in_use(&addr);
            if !ok {
                std::thread::sleep(std::time::Duration::from_millis(20));
            }
            ok
        });
        assert!(freed, "포트가 해제되지 않았다");
        let mut spawned = 0;
        spawn_daemon_if_needed(&addr, &mut || spawned += 1);
        assert_eq!(spawned, 1);
    }
```

- [ ] **Step 6: 테스트 실행해 실패 확인**

Run: `cargo test --manifest-path crates/decide/Cargo.toml main::tests::spawn_daemon_if_needed 2>&1 | tail -30`
Expected: FAIL — `spawn_daemon_if_needed`가 아직 없어 컴파일 오류.

- [ ] **Step 7: 구현**

`crates/decide/src/main.rs`에 `run_install_claude` 앞에 추가:

```rust
/// http_addr가 이미 쓰이고 있지 않으면 spawn을 호출한다(실제로는 decide daemon을 띄움).
fn spawn_daemon_if_needed(http_addr: &str, spawn: &mut dyn FnMut()) {
    if !decide::http::http_port_in_use(http_addr) {
        spawn();
    }
}
```

`run_install_claude`를 바꿔 마지막에 데몬 기동을 시도한다:

```rust
fn run_install_claude() -> io::Result<()> {
    run_install()?;
    let path = claude::settings_path().map_err(io::Error::other)?;
    let display = claude::hook_command(DECIDE_BIN);
    let gate = claude::gate_command(DECIDE_BIN);
    match claude::install_hooks(&path, &claude::hook_specs(&display, &gate)).map_err(io::Error::other)? {
        Installed::Added => println!("훅을 등록했습니다(결과 표시, bash-risk 게이트): {}", path.display()),
        Installed::AlreadyPresent => println!("훅이 이미 등록돼 있습니다: {}", path.display()),
    }
    spawn_daemon_if_needed(daemon::DEFAULT_HTTP_ADDR, &mut || {
        if decide::gate::client::spawn_daemon().is_ok() {
            println!("decide daemon을 띄웠습니다 (http://{}/mcp)", daemon::DEFAULT_HTTP_ADDR);
        }
    });
    Ok(())
}
```

- [ ] **Step 8: 테스트 실행해 통과 확인**

Run: `cargo test --manifest-path crates/decide/Cargo.toml main:: 2>&1 | tail -40`
Expected: PASS — Task 4의 3개 테스트 모두 통과.

- [ ] **Step 9: 전체 테스트 스위트 회귀 확인**

Run: `cargo test --manifest-path crates/decide/Cargo.toml 2>&1 | tail -30`
Expected: PASS, 0 failed.

- [ ] **Step 10: 커밋**

```bash
git add crates/decide/src/main.rs
git commit -m "feat(install): decide install이 MCP를 HTTP transport로 등록하고 데몬을 띄운다"
```

---

## Task 5: 실제 바이너리로 수동 검증

**Files:** 없음(검증만).

**Interfaces:** 없음.

- [ ] **Step 1: release 빌드**

Run: `cargo build --release --manifest-path crates/decide/Cargo.toml --bin decide 2>&1 | tail -10`
Expected: 경고 없이 빌드 성공.

- [ ] **Step 2: 떠 있는 기존 decide 프로세스 정리**

Run: `pgrep -f "decide mcp|decide daemon" | xargs -r kill 2>&1; sleep 1; pgrep -f "decide mcp|decide daemon" 2>&1`
Expected: 두 번째 출력이 비어 있다(모두 종료됨).

- [ ] **Step 3: 데몬을 직접 띄우고 두 트랜스포트를 모두 확인**

```bash
./crates/decide/target/release/decide daemon &
sleep 1
echo '{"jsonrpc":"2.0","id":1,"method":"tools/list"}' | curl -s -X POST http://127.0.0.1:48080/mcp -d @- | head -c 300
echo
ls -la ~/.cache/decide/decide.sock
```

Expected: `tools/list`의 JSON 응답에 `"name":"decide"`가 보이고, UDS 소켓
파일도 존재한다.

- [ ] **Step 4: `decide install --claude`로 자동 등록 확인(실제 `.mcp.json`/`~/.claude.json`
      변경 전에 사용자에게 확인)**

이 스텝은 사용자의 실제 Claude Code 설정을 바꾸므로, 실행 전에 사용자
승인을 받는다. 승인 후:

```bash
./crates/decide/target/release/decide install --claude
```

Expected: "already exists"면 성공으로 끝나거나(이미 stdio로 등록돼 있으면
먼저 `claude mcp remove decide`가 필요할 수 있음 — 그 경우 안내 문구를
읽고 사용자에게 알린다), 새로 등록되면 `claude mcp list`로 `decide`가
`http://127.0.0.1:48080/mcp`로 보이는지 확인한다.

- [ ] **Step 5: Claude Code를 재연결해 모델이 중복 로드되지 않는지 확인**

사용자에게 `/mcp` 재연결을 두 번 이상 요청하고, 그 사이 `ps aux | grep
"decide daemon"`으로 데몬 프로세스 수가 1개로 유지되는지, 재연결마다
도구 호출 지연이 콜드스타트 수준(수 초)이 아니라 즉시 응답하는지 보고한다.

Expected: 데몬 프로세스 1개 유지, 재연결 후 즉시 응답(모델이 이미
로드돼 있으므로).

---

## Self-Review 체크 결과 (writing-plans 요구사항)

- **Spec coverage:** 설계서의 "결정" 각 항목(두 트랜스포트 통합, HTTP
  최소 구현, 포트 고정, tiny_http 의존성, 동시성, idle 종료, install
  자동 등록, 자동 기동 안 됨)이 Task 1~4에 모두 반영됐다. "범위 밖"
  (SSE, 포트 설정, launchd, OAuth, stdio 완전 제거)은 어떤 Task도
  건드리지 않음을 Global Constraints에 명시했다.
- **Placeholder scan:** 모든 스텝에 실행 가능한 코드/명령이 있다.
  "TBD"/"나중에" 없음.
- **Type consistency:** `handle_http_body`(Task 1) → `serve_http`/`handle_one`
  (Task 2) → `serve_unified`(Task 3) → `install_mcp_args`/`spawn_daemon_if_needed`
  (Task 4)까지 함수 이름과 시그니처가 각 Task의 "Produces"/"Consumes"
  블록과 일치한다. `DEFAULT_HTTP_ADDR`는 Task 3에서 정의되고 Task 4가
  그대로 참조한다.
- **Review Focus 반영:** 이중 기동 방지(Task 3 `serve_unified`의 두 포트
  확인, Task 4의 `spawn_daemon_if_needed` 테스트), 깨진 JSON 처리(Task 1
  `invalid_json_body_returns_200_with_a_parse_error_payload`), 잘못된
  경로/메서드 404(Task 2 `a_get_or_wrong_path_returns_404`), 동시 접근
  (Task 3 Step 1 설명 — 백본 Mutex와 캐시 Mutex 사이 순환 대기 없음),
  install 재실행 멱등성(Task 4 Step 5~7, 기존 `run_install`의 "already
  exists" 처리를 그대로 유지) — 다섯 항목 모두 소유 태스크에 테스트나
  명시적 설명이 있다.
