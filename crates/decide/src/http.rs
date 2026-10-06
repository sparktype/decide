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
}
