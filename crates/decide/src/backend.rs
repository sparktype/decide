use crate::local;
use crate::protocol::{
    parse_arguments, parse_many, validate, validate_many, DecideManyResult, DecideResult,
};
use crate::typesafe::{self, LiveTransport, Transport};
use serde_json::{json, Value};

pub const MISSING_KEY: &str = "TYPESAFE_API_KEY가 없습니다";
pub const UNKNOWN_BACKEND: &str = "DECIDE_BACKEND는 typesafe 또는 local이어야 합니다";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Backend {
    Typesafe,
    Local,
}

#[derive(Debug, Clone)]
pub struct Env {
    pub backend: Option<String>,
    pub api_key: Option<String>,
}

impl Env {
    pub fn from_process() -> Self {
        Self {
            backend: std::env::var("DECIDE_BACKEND").ok(),
            api_key: std::env::var("TYPESAFE_API_KEY").ok(),
        }
    }
}

pub fn select_backend(explicit: Option<&str>, api_key: Option<&str>) -> Result<Backend, String> {
    let explicit = nonempty(explicit);
    let api_key = nonempty(api_key);
    match explicit {
        Some("typesafe") => {
            if api_key.is_none() {
                Err(MISSING_KEY.to_string())
            } else {
                Ok(Backend::Typesafe)
            }
        }
        Some("local") => Ok(Backend::Local),
        Some(_) => Err(UNKNOWN_BACKEND.to_string()),
        None => {
            if api_key.is_some() {
                Ok(Backend::Typesafe)
            } else {
                Ok(Backend::Local)
            }
        }
    }
}

pub fn decide<T: Transport>(
    raw: &Value,
    env: &Env,
    transport: &mut T,
    mut millis: impl FnMut() -> f64,
    sleep: impl FnMut(),
) -> Result<DecideResult, String> {
    let incoming = parse_arguments(raw)?;
    let question = validate(&incoming)?;
    let backend = select_backend(env.backend.as_deref(), env.api_key.as_deref())?;
    let (name, label) = labels(backend);
    if backend == Backend::Typesafe {
        if let Some(message) = typesafe::limit_error(&question) {
            return Err(message.to_string());
        }
    }
    let body = typesafe::request_body(&incoming.state, &question);
    let start = millis();
    let response = typesafe::execute(transport, &body, label, sleep)?;
    let latency_ms = millis() - start;
    let (answer, model) = typesafe::map_response(&response, label)?;
    Ok(DecideResult {
        answer,
        routing: json!({
            "backend": name,
            "model": model,
        }),
        latency_ms,
    })
}

fn labels(backend: Backend) -> (&'static str, &'static str) {
    match backend {
        Backend::Typesafe => ("typesafe", "TypeSafe"),
        Backend::Local => ("local", "로컬"),
    }
}

pub fn decide_many<T: Transport>(
    raw: &Value,
    env: &Env,
    transport: &mut T,
    mut millis: impl FnMut() -> f64,
    sleep: impl FnMut(),
) -> Result<DecideManyResult, String> {
    let incoming = parse_many(raw)?;
    let questions = validate_many(&incoming)?;
    let backend = select_backend(env.backend.as_deref(), env.api_key.as_deref())?;
    let (name, label) = labels(backend);
    if backend == Backend::Typesafe {
        for (id, question) in &questions {
            if let Some(message) = typesafe::limit_error(question) {
                return Err(format!("질문 \"{id}\": {message}"));
            }
        }
    }
    let body = typesafe::request_body_many(&incoming.state, &questions);
    let start = millis();
    let response = typesafe::execute(transport, &body, label, sleep)?;
    let latency_ms = millis() - start;
    let (answers, model) = typesafe::map_answers(&response, &questions, label)?;
    Ok(DecideManyResult {
        answers,
        routing: json!({
            "backend": name,
            "model": model,
        }),
        latency_ms,
    })
}

pub fn live_transport(env: &Env) -> LiveTransport {
    match select_backend(env.backend.as_deref(), env.api_key.as_deref()) {
        Ok(Backend::Local) => LiveTransport::local(&local::url()),
        _ => LiveTransport::typesafe(nonempty(env.api_key.as_deref()).unwrap_or("")),
    }
}

pub fn nonempty(value: Option<&str>) -> Option<&str> {
    value.map(str::trim).filter(|text| !text.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::typesafe::{RawResponse, Transport};
    use serde_json::json;
    use std::cell::Cell;

    struct Script {
        responses: Vec<Result<RawResponse, String>>,
        calls: Cell<usize>,
    }

    impl Transport for Script {
        fn post_json(&mut self, _body: &Value) -> Result<RawResponse, String> {
            let index = self.calls.get();
            let next = self.responses.get(index).expect("호출이 남았어야 한다");
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

    fn env(backend: Option<&str>, key: Option<&str>) -> Env {
        Env {
            backend: backend.map(str::to_string),
            api_key: key.map(str::to_string),
        }
    }

    fn noul() -> Value {
        json!({"state": "서버가 다운됐습니다", "type": "noul", "instructions": "긴급한가?"})
    }

    #[test]
    fn selection_follows_the_key_and_the_override() {
        assert_eq!(select_backend(None, Some("k")).unwrap(), Backend::Typesafe);
        assert_eq!(select_backend(None, None).unwrap(), Backend::Local);
        assert_eq!(select_backend(None, Some("  ")).unwrap(), Backend::Local);
        assert_eq!(
            select_backend(Some("typesafe"), Some("k")).unwrap(),
            Backend::Typesafe
        );
        assert_eq!(
            select_backend(Some("typesafe"), None).unwrap_err(),
            MISSING_KEY
        );
        assert_eq!(
            select_backend(Some("local"), Some("k")).unwrap(),
            Backend::Local
        );
        assert_eq!(
            select_backend(Some("cloud"), Some("k")).unwrap_err(),
            UNKNOWN_BACKEND
        );
    }

    #[test]
    fn typesafe_error_does_not_call_local() {
        let mut script = Script {
            responses: vec![Ok(RawResponse {
                status: 401,
                body: r#"{"message":"unauthorized"}"#.into(),
            })],
            calls: Cell::new(0),
        };
        let err = decide(&noul(), &env(None, Some("k")), &mut script, || 0.0, || {}).unwrap_err();
        assert!(err.contains("401"), "{err}");
        assert!(!err.contains("로컬"), "{err}");
        assert_eq!(script.calls.get(), 1);
    }

    const NOUL_OK: &str =
        r#"{"model":"jev-style-2b-decision-v3","answers":{"q":{"type":"noul","noul":0.9}}}"#;

    fn ok_script(body: &str) -> Script {
        Script {
            responses: vec![Ok(RawResponse {
                status: 200,
                body: body.into(),
            })],
            calls: Cell::new(0),
        }
    }

    #[test]
    fn local_routes_the_server_answer_as_local() {
        let mut script = ok_script(NOUL_OK);
        let result = decide(
            &noul(),
            &env(Some("local"), Some("k")),
            &mut script,
            || 0.0,
            || panic!("성공 응답은 재시도하지 않는다"),
        )
        .unwrap();
        assert_eq!(result.routing["backend"], "local");
        assert_eq!(result.routing["model"], "jev-style-2b-decision-v3");
        assert_eq!(result.answer["noul"], 0.9);
        assert_eq!(script.calls.get(), 1);
    }

    #[test]
    fn local_leaves_option_limits_to_the_server_but_typesafe_checks_first() {
        let mut options = Vec::new();
        for i in 0..256 {
            options.push(i.to_string());
        }
        let raw = json!({
            "state": "s",
            "type": "choice",
            "instructions": "어느 쪽?",
            "options": options,
        });
        let mut script = ok_script(
            r#"{"model":"m","answers":{"q":{"type":"choice","choice":"0","probabilities":{"0":1.0}}}}"#,
        );
        decide(&raw, &env(Some("local"), None), &mut script, || 0.0, || {}).unwrap();
        assert_eq!(script.calls.get(), 1);

        let mut untouched = Script {
            responses: vec![],
            calls: Cell::new(0),
        };
        let err = decide(
            &raw,
            &env(Some("typesafe"), Some("k")),
            &mut untouched,
            || 0.0,
            || {},
        )
        .unwrap_err();
        assert_eq!(err, typesafe::CHOICE_LIMIT);
        assert_eq!(untouched.calls.get(), 0);
    }

    #[test]
    fn local_failure_names_local_and_does_not_fall_back() {
        let mut script = Script {
            responses: vec![Err("connection refused".into())],
            calls: Cell::new(0),
        };
        let err = decide(
            &noul(),
            &env(Some("local"), Some("k")),
            &mut script,
            || 0.0,
            || panic!("재시도하면 안 된다"),
        )
        .unwrap_err();
        assert!(err.contains("로컬"), "{err}");
        assert!(!err.contains("TypeSafe"), "{err}");
        assert_eq!(script.calls.get(), 1);
    }

    #[test]
    fn success_maps_the_recorded_response_and_measures_only_the_call() {
        let mut script = Script {
            responses: vec![Ok(RawResponse {
                status: 200,
                body: r#"{"model":"jev-1.13.0","answers":{"q":{"type":"choice","choice":"billing","probabilities":{"billing":0.88,"technical":0.12},"confidence":0.81}}}"#.into(),
            })],
            calls: Cell::new(0),
        };
        let ticks = Cell::new(0.0);
        let result = decide(
            &json!({
                "state": "중복 결제",
                "type": "choice",
                "instructions": "어느 팀?",
                "options": ["billing", "technical"]
            }),
            &env(None, Some("k")),
            &mut script,
            || {
                let now = ticks.get();
                ticks.set(now + 12.5);
                now
            },
            || panic!("성공 응답은 재시도하지 않는다"),
        )
        .unwrap();
        assert_eq!(result.latency_ms, 12.5);
        assert_eq!(result.routing["backend"], "typesafe");
        assert_eq!(result.routing["model"], "jev-1.13.0");
        assert_eq!(result.answer["choice"], "billing");
        assert!(result.answer.get("action").is_none());
    }

    fn many_raw() -> Value {
        json!({
            "state": "서버 다운",
            "questions": {
                "urgent": {"type": "noul", "instructions": "긴급한가?"},
                "team": {"type": "choice", "instructions": "어느 팀?", "options": ["billing", "infra"]}
            }
        })
    }

    const MANY_OK: &str = r#"{"model":"jev-1.13.0","answers":{"team":{"type":"choice","choice":"infra","probabilities":{"billing":0.1,"infra":0.9},"confidence":0.7},"urgent":{"type":"noul","noul":0.8}}}"#;

    #[test]
    fn many_maps_answers_in_request_order_with_one_call() {
        let mut script = ok_script(MANY_OK);
        let ticks = Cell::new(0.0);
        let result = decide_many(
            &many_raw(),
            &env(None, Some("k")),
            &mut script,
            || {
                let now = ticks.get();
                ticks.set(now + 7.5);
                now
            },
            || panic!("성공 응답은 재시도하지 않는다"),
        )
        .unwrap();
        assert_eq!(script.calls.get(), 1);
        assert_eq!(result.latency_ms, 7.5);
        assert_eq!(result.routing["backend"], "typesafe");
        assert_eq!(result.routing["model"], "jev-1.13.0");
        let ids: Vec<&String> = result.answers.as_object().unwrap().keys().collect();
        assert_eq!(ids, vec!["urgent", "team"]);
        assert_eq!(result.answers["team"]["choice"], "infra");
        assert_eq!(result.answers["urgent"]["noul"], 0.8);
    }

    #[test]
    fn many_validation_error_makes_no_call() {
        let mut script = Script {
            responses: vec![],
            calls: Cell::new(0),
        };
        let raw = json!({
            "state": "s",
            "questions": {
                "ok": {"type": "noul", "instructions": "?"},
                "team": {"type": "choice", "instructions": "?", "options": ["a"]}
            }
        });
        let err = decide_many(&raw, &env(None, Some("k")), &mut script, || 0.0, || {}).unwrap_err();
        assert!(err.starts_with("질문 \"team\": "), "{err}");
        assert_eq!(script.calls.get(), 0);
    }

    #[test]
    fn many_typesafe_limit_names_the_question_but_local_passes_through() {
        let options: Vec<String> = (0..256).map(|i| i.to_string()).collect();
        let raw = json!({
            "state": "s",
            "questions": {
                "small": {"type": "noul", "instructions": "?"},
                "big": {"type": "choice", "instructions": "?", "options": options}
            }
        });
        let mut untouched = Script {
            responses: vec![],
            calls: Cell::new(0),
        };
        let err = decide_many(
            &raw,
            &env(Some("typesafe"), Some("k")),
            &mut untouched,
            || 0.0,
            || {},
        )
        .unwrap_err();
        assert_eq!(err, format!("질문 \"big\": {}", typesafe::CHOICE_LIMIT));
        assert_eq!(untouched.calls.get(), 0);

        let mut script = ok_script(
            r#"{"model":"m","answers":{"small":{"type":"noul","noul":0.5},"big":{"type":"choice","choice":"0"}}}"#,
        );
        let result = decide_many(&raw, &env(Some("local"), None), &mut script, || 0.0, || {}).unwrap();
        assert_eq!(script.calls.get(), 1);
        assert_eq!(result.routing["backend"], "local");
    }

    #[test]
    fn many_fails_entirely_on_backend_error_or_missing_answer() {
        let mut down = Script {
            responses: vec![Err("connection refused".into())],
            calls: Cell::new(0),
        };
        let err = decide_many(
            &many_raw(),
            &env(Some("local"), None),
            &mut down,
            || 0.0,
            || panic!("재시도하면 안 된다"),
        )
        .unwrap_err();
        assert!(err.contains("로컬"), "{err}");
        assert_eq!(down.calls.get(), 1);

        let mut partial = ok_script(
            r#"{"model":"m","answers":{"urgent":{"type":"noul","noul":0.8}}}"#,
        );
        let err = decide_many(&many_raw(), &env(None, Some("k")), &mut partial, || 0.0, || {})
            .unwrap_err();
        assert_eq!(err, "TypeSafe 응답에 answers.team가 없습니다");
    }

    #[test]
    fn many_handles_a_hundred_questions_without_a_cap() {
        let mut questions = serde_json::Map::new();
        let mut answers = serde_json::Map::new();
        for i in 0..100 {
            questions.insert(
                format!("q{i}"),
                json!({"type": "noul", "instructions": "참인가?"}),
            );
            answers.insert(format!("q{i}"), json!({"type": "noul", "noul": 0.5}));
        }
        let raw = json!({"state": "s", "questions": questions});
        let body = json!({"model": "m", "answers": answers}).to_string();
        let mut script = ok_script(&body);
        let result = decide_many(&raw, &env(None, Some("k")), &mut script, || 0.0, || {}).unwrap();
        let ids: Vec<String> = result.answers.as_object().unwrap().keys().cloned().collect();
        let expected: Vec<String> = (0..100).map(|i| format!("q{i}")).collect();
        assert_eq!(ids, expected);
        assert_eq!(script.calls.get(), 1);
    }
}
