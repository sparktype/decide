use crate::backend::{decide, Env};
use crate::protocol::DecideResult;
use crate::typesafe::Transport;
use serde_json::{json, Value};
use std::time::{Duration, Instant};

const TOOL_DESCRIPTION: &str = "\
여러 선택지 중 고르기(choice), 순서형 점수 매기기(score), 또는 참에 가까운 \
확률 추정(noul)이 필요할 때 판단 모델을 단일 순전파로 호출한다. \
개방형 추론/생성 작업에는 사용하지 않는다. noul의 결과는 boolean이 아니라 \
0.0~1.0 확률이다.";

pub fn handle_message<T: Transport>(
    message: &Value,
    env: &Env,
    transport: &mut T,
) -> Option<Value> {
    let obj = match message.as_object() {
        Some(obj) => obj,
        None => {
            return Some(rpc_error(
                Value::Null,
                -32600,
                "요청은 JSON 객체여야 합니다",
            ))
        }
    };
    let Some(method) = obj.get("method").and_then(Value::as_str) else {
        return None;
    };
    let Some(id) = obj.get("id").cloned() else {
        return None;
    };
    let result = match method {
        "initialize" => Ok(initialize_result(obj.get("params"))),
        "ping" => Ok(json!({})),
        "tools/list" => Ok(tools_list()),
        "tools/call" => tool_call(obj.get("params"), env, transport),
        _ => Err((-32601, format!("메서드를 찾을 수 없습니다: {method}"))),
    };
    Some(match result {
        Ok(result) => json!({"jsonrpc": "2.0", "id": id, "result": result}),
        Err((code, message)) => rpc_error(id, code, &message),
    })
}

fn initialize_result(params: Option<&Value>) -> Value {
    let version = params
        .and_then(|params| params.get("protocolVersion"))
        .and_then(Value::as_str)
        .filter(|version| !version.is_empty())
        .unwrap_or("2025-03-26");
    json!({
        "protocolVersion": version,
        "capabilities": {"tools": {}},
        "serverInfo": {"name": "decide", "version": env!("CARGO_PKG_VERSION")},
    })
}

fn tools_list() -> Value {
    json!({
        "tools": [{
            "name": "decide",
            "description": TOOL_DESCRIPTION,
            "inputSchema": {
                "type": "object",
                "properties": {
                    "state": {"type": "string"},
                    "type": {"type": "string", "enum": ["choice", "score", "noul"]},
                    "instructions": {"type": "string"},
                    "options": {"type": "array", "items": {"type": "string"}},
                    "criteria": {"type": "array", "items": {"type": "string"}}
                },
                "required": ["state", "type", "instructions"]
            }
        }]
    })
}

fn tool_call<T: Transport>(
    params: Option<&Value>,
    env: &Env,
    transport: &mut T,
) -> Result<Value, (i32, String)> {
    let params = params
        .and_then(Value::as_object)
        .ok_or_else(|| (-32602, "params가 필요합니다".to_string()))?;
    let name = params.get("name").and_then(Value::as_str).unwrap_or("");
    if name != "decide" {
        return Ok(tool_error("알 수 없는 도구입니다"));
    }
    let arguments = params
        .get("arguments")
        .cloned()
        .unwrap_or_else(|| json!({}));
    let origin = Instant::now();
    match decide(
        &arguments,
        env,
        transport,
        || origin.elapsed().as_secs_f64() * 1000.0,
        || std::thread::sleep(Duration::from_secs(1)),
    ) {
        Ok(result) => Ok(tool_success(&result)),
        Err(message) => Ok(tool_error(&message)),
    }
}

fn tool_success(result: &DecideResult) -> Value {
    let text = serde_json::to_string_pretty(result).unwrap_or_else(|_| "{}".to_string());
    let structured = serde_json::to_value(result).unwrap_or(Value::Null);
    json!({
        "content": [{"type": "text", "text": text}],
        "structuredContent": structured,
        "isError": false,
    })
}

fn tool_error(message: &str) -> Value {
    json!({
        "content": [{"type": "text", "text": message}],
        "isError": true,
    })
}

fn rpc_error(id: Value, code: i32, message: &str) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "error": {"code": code, "message": message},
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::typesafe::{RawResponse, Transport};
    use std::cell::Cell;

    struct Script {
        status: u16,
        body: String,
        calls: Cell<usize>,
    }

    impl Transport for Script {
        fn post_json(&mut self, _body: &Value) -> Result<RawResponse, String> {
            self.calls.set(self.calls.get() + 1);
            Ok(RawResponse {
                status: self.status,
                body: self.body.clone(),
            })
        }
    }

    fn env_local() -> Env {
        Env {
            backend: Some("local".into()),
            api_key: Some("k".into()),
        }
    }

    #[test]
    fn initialize_echoes_the_protocol_and_lists_decide() {
        let mut script = Script {
            status: 500,
            body: String::new(),
            calls: Cell::new(0),
        };
        let init = handle_message(
            &json!({
                "jsonrpc": "2.0",
                "id": 1,
                "method": "initialize",
                "params": {"protocolVersion": "2025-03-26"}
            }),
            &env_local(),
            &mut script,
        )
        .unwrap();
        assert_eq!(init["result"]["protocolVersion"], "2025-03-26");
        assert_eq!(init["result"]["serverInfo"]["name"], "decide");

        assert!(handle_message(
            &json!({"jsonrpc": "2.0", "method": "notifications/initialized"}),
            &env_local(),
            &mut script,
        )
        .is_none());

        let listed = handle_message(
            &json!({"jsonrpc": "2.0", "id": 2, "method": "tools/list"}),
            &env_local(),
            &mut script,
        )
        .unwrap();
        assert_eq!(listed["result"]["tools"][0]["name"], "decide");
        assert!(listed["result"]["tools"][0]["description"]
            .as_str()
            .unwrap()
            .contains("noul"));
    }

    #[test]
    fn tool_call_routes_to_typesafe_or_local() {
        let mut script = Script {
            status: 200,
            body: r#"{"model":"jev-1.13.0","answers":{"q":{"type":"noul","noul":0.25}}}"#.into(),
            calls: Cell::new(0),
        };
        let response = handle_message(
            &json!({
                "jsonrpc": "2.0",
                "id": 3,
                "method": "tools/call",
                "params": {
                    "name": "decide",
                    "arguments": {"state": "다운", "type": "noul", "instructions": "긴급한가?"}
                }
            }),
            &Env {
                backend: None,
                api_key: Some("k".into()),
            },
            &mut script,
        )
        .unwrap();
        assert_eq!(response["result"]["isError"], false);
        assert_eq!(
            response["result"]["structuredContent"]["routing"]["backend"],
            "typesafe"
        );
        assert_eq!(
            response["result"]["structuredContent"]["answer"]["noul"],
            0.25
        );

        let response = handle_message(
            &json!({
                "jsonrpc": "2.0",
                "id": 4,
                "method": "tools/call",
                "params": {
                    "name": "decide",
                    "arguments": {"state": "다운", "type": "noul", "instructions": "긴급한가?"}
                }
            }),
            &env_local(),
            &mut script,
        )
        .unwrap();
        assert_eq!(response["result"]["isError"], false);
        assert_eq!(
            response["result"]["structuredContent"]["routing"]["backend"],
            "local"
        );
        assert_eq!(script.calls.get(), 2);
    }
}
