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
