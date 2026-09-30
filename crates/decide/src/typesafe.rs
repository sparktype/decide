use crate::protocol::Question;
use serde_json::{json, Map, Value};

pub const ENDPOINT: &str = "https://api.typesafe.ai/v1/systemone";
pub const MODEL: &str = "jev-latest";
pub const CHOICE_LIMIT: &str = "choice 타입은 TypeSafe에서 옵션이 255개를 넘을 수 없습니다";
pub const SCORE_LIMIT: &str = "score 타입은 TypeSafe에서 등급이 10개를 넘을 수 없습니다";

pub struct RawResponse {
    pub status: u16,
    pub body: String,
}

pub trait Transport {
    fn post_json(&mut self, body: &Value) -> Result<RawResponse, String>;
}

pub fn limit_error(question: &Question) -> Option<&'static str> {
    match question {
        Question::Choice { options, .. } if options.len() > 255 => Some(CHOICE_LIMIT),
        Question::Score { criteria, .. } if criteria.len() > 10 => Some(SCORE_LIMIT),
        _ => None,
    }
}

pub fn request_body(state: &str, question: &Question) -> Value {
    let q = match question {
        Question::Choice {
            instructions,
            options,
        } => {
            let mut criteria = Map::new();
            for option in options {
                criteria.insert(option.clone(), Value::String(option.clone()));
            }
            json!({
                "type": "choice",
                "instructions": instructions,
                "criteria": Value::Object(criteria),
            })
        }
        Question::Score {
            instructions,
            criteria,
        } => json!({
            "type": "score",
            "instructions": instructions,
            "criteria": criteria,
        }),
        Question::Noul { instructions } => json!({
            "type": "noul",
            "instructions": instructions,
        }),
    };
    json!({
        "model": MODEL,
        "state": state,
        "questions": { "q": q },
    })
}

pub fn authorization(key: &str) -> String {
    format!("Bearer {key}")
}

pub fn execute<T: Transport>(
    transport: &mut T,
    body: &Value,
    label: &str,
    mut sleep: impl FnMut(),
) -> Result<Value, String> {
    let mut attempt = 0;
    loop {
        attempt += 1;
        let raw = match transport.post_json(body) {
            Ok(raw) => raw,
            Err(err) => return Err(format!("{label} 연결에 실패했습니다: {err}")),
        };
        if (raw.status == 429 || raw.status == 529) && attempt == 1 {
            sleep();
            continue;
        }
        if !(200..300).contains(&raw.status) {
            return Err(http_error(label, raw.status, &raw.body));
        }
        return serde_json::from_str(&raw.body)
            .map_err(|_| format!("{label} 응답이 JSON이 아닙니다"));
    }
}

pub fn map_response(body: &Value, label: &str) -> Result<(Value, String), String> {
    let model = body
        .get("model")
        .and_then(Value::as_str)
        .ok_or_else(|| format!("{label} 응답에 model이 없습니다"))?
        .to_string();
    let answer = body
        .get("answers")
        .and_then(|answers| answers.get("q"))
        .cloned()
        .ok_or_else(|| format!("{label} 응답에 answers.q가 없습니다"))?;
    Ok((answer, model))
}

fn http_error(label: &str, status: u16, body: &str) -> String {
    let detail = serde_json::from_str::<Value>(body)
        .ok()
        .and_then(|value| {
            value
                .get("message")
                .or_else(|| value.get("error"))
                .and_then(Value::as_str)
                .map(str::to_string)
        })
        .unwrap_or_else(|| body.trim().to_string());
    if detail.is_empty() {
        format!("{label} 요청이 실패했습니다: HTTP {status}")
    } else {
        format!("{label} 요청이 실패했습니다: HTTP {status}: {detail}")
    }
}

pub struct LiveTransport {
    agent: ureq::Agent,
    url: String,
    key: Option<String>,
}

impl LiveTransport {
    pub fn typesafe(key: &str) -> Self {
        Self::build(ENDPOINT, Some(key.to_string()))
    }

    pub fn local(url: &str) -> Self {
        Self::build(url, None)
    }

    fn build(url: &str, key: Option<String>) -> Self {
        Self {
            agent: ureq::AgentBuilder::new()
                .timeout(std::time::Duration::from_secs(30))
                .build(),
            url: url.to_string(),
            key,
        }
    }
}

impl Transport for LiveTransport {
    fn post_json(&mut self, body: &Value) -> Result<RawResponse, String> {
        let mut request = self
            .agent
            .post(&self.url)
            .set("Content-Type", "application/json");
        if let Some(key) = &self.key {
            request = request.set("Authorization", &authorization(key));
        }
        match request.send_json(body) {
            Ok(response) => Ok(RawResponse {
                status: response.status(),
                body: response
                    .into_string()
                    .map_err(|err| format!("응답을 읽지 못했습니다: {err}"))?,
            }),
            Err(ureq::Error::Status(status, response)) => Ok(RawResponse {
                status,
                body: response.into_string().unwrap_or_default(),
            }),
            Err(err) if self.key.is_none() => Err(format!("{err}. {}", crate::local::CONNECT_HINT)),
            Err(err) => Err(err.to_string()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::{validate, Incoming, Kind, Question};
    use std::cell::Cell;

    struct Script {
        responses: Vec<Result<RawResponse, String>>,
        calls: Cell<usize>,
    }

    impl Transport for Script {
        fn post_json(&mut self, _body: &Value) -> Result<RawResponse, String> {
            let index = self.calls.get();
            let next = self.responses.get(index).expect("예상보다 많은 호출");
            self.calls.set(index + 1);
            match next {
                Ok(raw) => Ok(RawResponse {
                    status: raw.status,
                    body: raw.body.clone(),
                }),
                Err(err) => Err(err.clone()),
            }
        }
    }

    fn noul() -> Question {
        validate(&Incoming {
            state: "상태".into(),
            kind: Kind::Noul,
            instructions: "참인가?".into(),
            options: vec![],
            criteria: vec![],
        })
        .unwrap()
    }

    fn ok_body() -> String {
        r#"{"model":"jev-1.13.0","answers":{"q":{"type":"noul","noul":0.5}}}"#.into()
    }

    #[test]
    fn body_shapes_match_the_three_types() {
        let choice = validate(&Incoming {
            state: "s".into(),
            kind: Kind::Choice,
            instructions: "어느 팀?".into(),
            options: vec!["billing".into(), "technical".into()],
            criteria: vec![],
        })
        .unwrap();
        let body = request_body("청구서", &choice);
        assert_eq!(body["model"], "jev-latest");
        assert_eq!(body["state"], "청구서");
        assert_eq!(body["questions"]["q"]["criteria"]["billing"], "billing");
        let text = serde_json::to_string(&body).unwrap();
        let billing = text.find("billing").unwrap();
        let technical = text.find("technical").unwrap();
        assert!(billing < technical);

        let score = validate(&Incoming {
            state: "s".into(),
            kind: Kind::Score,
            instructions: "강도?".into(),
            options: vec![],
            criteria: vec!["낮음".into(), "높음".into()],
        })
        .unwrap();
        let body = request_body("s", &score);
        assert_eq!(body["questions"]["q"]["criteria"], json!(["낮음", "높음"]));

        let body = request_body("s", &noul());
        assert!(body["questions"]["q"].get("criteria").is_none());
        assert_eq!(authorization("secret"), "Bearer secret");
        assert!(!serde_json::to_string(&body).unwrap().contains("secret"));
    }

    #[test]
    fn limits_are_exclusive_of_the_maximum() {
        let mut options: Vec<_> = (0..255).map(|i| i.to_string()).collect();
        let at_max = validate(&Incoming {
            state: "s".into(),
            kind: Kind::Choice,
            instructions: "q".into(),
            options: options.clone(),
            criteria: vec![],
        })
        .unwrap();
        assert!(limit_error(&at_max).is_none());
        options.push("255".into());
        let over = validate(&Incoming {
            state: "s".into(),
            kind: Kind::Choice,
            instructions: "q".into(),
            options,
            criteria: vec![],
        })
        .unwrap();
        assert_eq!(limit_error(&over), Some(CHOICE_LIMIT));

        let mut levels: Vec<_> = (0..10).map(|i| format!("L{i}")).collect();
        let at_max = validate(&Incoming {
            state: "s".into(),
            kind: Kind::Score,
            instructions: "q".into(),
            options: vec![],
            criteria: levels.clone(),
        })
        .unwrap();
        assert!(limit_error(&at_max).is_none());
        levels.push("L10".into());
        let over = validate(&Incoming {
            state: "s".into(),
            kind: Kind::Score,
            instructions: "q".into(),
            options: vec![],
            criteria: levels,
        })
        .unwrap();
        assert_eq!(limit_error(&over), Some(SCORE_LIMIT));
    }

    #[test]
    fn retries_429_once_then_accepts_success() {
        let mut script = Script {
            responses: vec![
                Ok(RawResponse {
                    status: 429,
                    body: "{}".into(),
                }),
                Ok(RawResponse {
                    status: 200,
                    body: ok_body(),
                }),
            ],
            calls: Cell::new(0),
        };
        let slept = Cell::new(0);
        let parsed = execute(&mut script, &request_body("s", &noul()), "TypeSafe", || {
            slept.set(slept.get() + 1);
        })
        .unwrap();
        assert_eq!(script.calls.get(), 2);
        assert_eq!(slept.get(), 1);
        assert_eq!(parsed["model"], "jev-1.13.0");
    }

    #[test]
    fn second_529_is_an_error_and_connection_failures_do_not_retry() {
        let mut overloaded = Script {
            responses: vec![
                Ok(RawResponse {
                    status: 529,
                    body: r#"{"message":"busy"}"#.into(),
                }),
                Ok(RawResponse {
                    status: 529,
                    body: r#"{"message":"busy"}"#.into(),
                }),
            ],
            calls: Cell::new(0),
        };
        let err = execute(
            &mut overloaded,
            &request_body("s", &noul()),
            "TypeSafe",
            || {},
        )
        .unwrap_err();
        assert!(err.contains("529"), "{err}");
        assert!(err.contains("busy"), "{err}");
        assert_eq!(overloaded.calls.get(), 2);

        let mut down = Script {
            responses: vec![Err("connection reset".into())],
            calls: Cell::new(0),
        };
        let err = execute(&mut down, &request_body("s", &noul()), "TypeSafe", || {
            panic!("재시도하면 안 된다")
        })
        .unwrap_err();
        assert!(err.contains("연결"), "{err}");
        assert_eq!(down.calls.get(), 1);

        let mut invalid = Script {
            responses: vec![Ok(RawResponse {
                status: 422,
                body: r#"{"message":"bad"}"#.into(),
            })],
            calls: Cell::new(0),
        };
        let err = execute(
            &mut invalid,
            &request_body("s", &noul()),
            "TypeSafe",
            || panic!("재시도하면 안 된다"),
        )
        .unwrap_err();
        assert!(err.contains("422"), "{err}");
        assert_eq!(invalid.calls.get(), 1);
    }

    fn serve_once(reply: &'static str) -> (String, std::thread::JoinHandle<String>) {
        use std::io::{Read, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/v1/systemone", listener.local_addr().unwrap());
        let handle = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = Vec::new();
            let mut chunk = [0u8; 4096];
            loop {
                let n = stream.read(&mut chunk).unwrap();
                request.extend_from_slice(&chunk[..n]);
                let text = String::from_utf8_lossy(&request).to_string();
                if let Some(end) = text.find("\r\n\r\n") {
                    let length = text
                        .lines()
                        .find_map(|l| {
                            l.to_lowercase()
                                .strip_prefix("content-length:")
                                .map(|v| v.trim().parse::<usize>().unwrap())
                        })
                        .unwrap_or(0);
                    if request.len() >= end + 4 + length {
                        break;
                    }
                }
                assert!(n > 0, "요청이 끝나기 전에 연결이 닫혔다");
            }
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                reply.len(),
                reply
            );
            stream.write_all(response.as_bytes()).unwrap();
            String::from_utf8_lossy(&request).to_string()
        });
        (url, handle)
    }

    #[test]
    fn local_transport_posts_to_its_url_without_authorization() {
        let (url, server) =
            serve_once(r#"{"model":"m","answers":{"q":{"type":"noul","noul":0.5}}}"#);
        let mut transport = LiveTransport::local(&url);
        let raw = transport.post_json(&request_body("s", &noul())).unwrap();
        assert_eq!(raw.status, 200);
        let request = server.join().unwrap();
        assert!(request.starts_with("POST /v1/systemone "), "{request}");
        assert!(
            !request.to_lowercase().contains("authorization"),
            "{request}"
        );
    }

    #[test]
    fn local_connection_failure_points_at_jev_style_serve() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/v1/systemone", listener.local_addr().unwrap());
        drop(listener);
        let err = LiveTransport::local(&url)
            .post_json(&request_body("s", &noul()))
            .err()
            .unwrap();
        assert!(err.contains("jev-style serve"), "{err}");
    }

    #[test]
    fn map_passes_the_answer_through() {
        let body = json!({
            "model": "jev-1.13.0",
            "answers": {"q": {"type": "noul", "noul": 0.95}},
            "usage": {"input_tokens": 1, "output_tokens": 1}
        });
        let (answer, model) = map_response(&body, "TypeSafe").unwrap();
        assert_eq!(model, "jev-1.13.0");
        assert_eq!(answer, json!({"type": "noul", "noul": 0.95}));
        assert!(answer.get("confidence").is_none());
        assert!(answer.get("action").is_none());
        assert_eq!(
            map_response(&json!({"model": "jev-1.13.0"}), "TypeSafe").unwrap_err(),
            "TypeSafe 응답에 answers.q가 없습니다"
        );
    }
}
