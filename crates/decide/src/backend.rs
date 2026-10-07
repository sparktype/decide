use crate::protocol::{
    parse_arguments, parse_many, validate, validate_many, DecideManyResult, DecideResult,
};
use crate::typesafe::{self, LiveTransport, Transport};
use serde_json::{json, Value};

pub const MISSING_KEY: &str = "TYPESAFE_API_KEY가 없습니다";
/// 로컬 서버(`scripts/serve-local.sh`가 띄우는 kev.serve)의 기본 주소.
pub const DEFAULT_LOCAL_URL: &str = "http://127.0.0.1:8009/v1/systemone";
const LOCAL_HINT: &str = "scripts/serve-local.sh로 로컬 서버를 띄웠는지 확인하세요";
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
        let (file, warnings) = crate::config::load_from_disk(std::env::var("HOME").ok().as_deref());
        for warning in &warnings {
            eprintln!("{warning}");
        }
        let env_var = |name: &str| std::env::var(name).ok().filter(|v| !v.trim().is_empty());
        Self {
            backend: env_var("DECIDE_BACKEND").or(file.backend),
            api_key: env_var("TYPESAFE_API_KEY").or(file.typesafe_api_key),
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
    let response = typesafe::execute(transport, &body, label, sleep)
        .map_err(|err| with_local_hint(backend, err))?;
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
    let response = typesafe::execute(transport, &body, label, sleep)
        .map_err(|err| with_local_hint(backend, err))?;
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

/// `DECIDE_TYPESAFE_URL` 환경변수, 없으면 config.toml의 `[typesafe].url`. 둘 다 없으면 None(TypeSafe 기본 주소).
fn resolved_typesafe_url() -> Option<String> {
    let env = std::env::var("DECIDE_TYPESAFE_URL").ok().filter(|v| !v.trim().is_empty());
    env.or_else(|| crate::config::load_from_disk(std::env::var("HOME").ok().as_deref()).0.typesafe_url)
}

/// `DECIDE_LOCAL_URL` 환경변수, 없으면 config.toml의 `[local].url`, 둘 다 없으면 기본 주소.
pub fn pick_local_url(env: Option<&str>, file: Option<&str>) -> String {
    nonempty(env).or_else(|| nonempty(file)).unwrap_or(DEFAULT_LOCAL_URL).to_string()
}

fn resolved_local_url() -> String {
    let env = std::env::var("DECIDE_LOCAL_URL").ok();
    let file = crate::config::load_from_disk(std::env::var("HOME").ok().as_deref()).0.local_url;
    pick_local_url(env.as_deref(), file.as_deref())
}

/// 로컬 서버에 연결하지 못했을 때만 서버를 띄우는 방법을 덧붙인다(HTTP 오류는 서버 메시지가 더 정확하다).
fn with_local_hint(backend: Backend, err: String) -> String {
    if backend == Backend::Local && err.contains("연결에 실패했습니다") {
        format!("{err} — {LOCAL_HINT}")
    } else {
        err
    }
}

pub fn live_transport(env: &Env) -> LiveTransport {
    if select_backend(env.backend.as_deref(), env.api_key.as_deref()) == Ok(Backend::Local) {
        return LiveTransport::local(&resolved_local_url());
    }
    let key = nonempty(env.api_key.as_deref()).unwrap_or("");
    match resolved_typesafe_url() {
        Some(url) => LiveTransport::with_url(url.trim(), key),
        None => LiveTransport::typesafe(key),
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

    fn ok_script(body: &str) -> Script {
        Script {
            responses: vec![Ok(RawResponse {
                status: 200,
                body: body.into(),
            })],
            calls: Cell::new(0),
        }
    }

    fn local_body() -> &'static str {
        r#"{"model":"kev-4b","answers":{"q":{"type":"noul","noul":0.7}}}"#
    }

    #[test]
    fn local_backend_sends_one_request_and_reports_local() {
        let mut script = ok_script(local_body());
        let result = decide(&noul(), &env(Some("local"), Some("k")), &mut script, || 0.0, || {})
            .unwrap();
        assert_eq!(script.calls.get(), 1);
        assert_eq!(result.routing["backend"], "local");
        assert_eq!(result.routing["model"], "kev-4b");
        assert_eq!(result.answer["noul"], 0.7);
    }

    #[test]
    fn local_connection_failure_points_at_the_serve_script_and_does_not_retry() {
        let mut script = Script {
            responses: vec![Err("Connection refused".into())],
            calls: Cell::new(0),
        };
        let err = decide(
            &noul(),
            &env(Some("local"), None),
            &mut script,
            || 0.0,
            || panic!("재시도하면 안 된다"),
        )
        .unwrap_err();
        assert!(err.starts_with("로컬 연결에 실패했습니다"), "{err}");
        assert!(err.contains("scripts/serve-local.sh"), "{err}");
        assert!(!err.contains("TypeSafe"), "{err}");
        assert_eq!(script.calls.get(), 1);
    }

    #[test]
    fn local_http_error_keeps_the_server_message_without_the_start_hint() {
        let mut script = Script {
            responses: vec![Ok(RawResponse {
                status: 422,
                body: r#"{"message":"state too long"}"#.into(),
            })],
            calls: Cell::new(0),
        };
        let err = decide(&noul(), &env(Some("local"), None), &mut script, || 0.0, || {}).unwrap_err();
        assert_eq!(err, "로컬 요청이 실패했습니다: HTTP 422: state too long");
    }

    #[test]
    fn local_sends_the_option_limit_to_the_server_but_typesafe_checks_first() {
        let options: Vec<String> = (0..256).map(|i| i.to_string()).collect();
        let raw = json!({
            "state": "s",
            "type": "choice",
            "instructions": "어느 쪽?",
            "options": options,
        });
        let mut script = ok_script(
            r#"{"model":"kev-4b","answers":{"q":{"type":"choice","choice":"0","probabilities":{"0":1.0},"confidence":1.0}}}"#,
        );
        let result = decide(&raw, &env(Some("local"), None), &mut script, || 0.0, || {}).unwrap();
        assert_eq!(result.answer["choice"], "0");
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
    fn local_url_prefers_env_then_file_then_the_default() {
        assert_eq!(pick_local_url(Some("http://e/x"), Some("http://f/x")), "http://e/x");
        assert_eq!(pick_local_url(None, Some(" http://f/x ")), "http://f/x");
        assert_eq!(pick_local_url(Some("  "), None), DEFAULT_LOCAL_URL);
        assert_eq!(pick_local_url(None, None), DEFAULT_LOCAL_URL);
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
    fn many_typesafe_limit_names_the_question_but_local_sends_it() {
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

        // 로컬은 TypeSafe의 옵션 한도를 거치지 않고 서버로 보낸다.
        let mut script = ok_script(
            r#"{"model":"kev-4b","answers":{"small":{"type":"noul","noul":0.5},"big":{"type":"choice","choice":"0","probabilities":{"0":1.0},"confidence":1.0}}}"#,
        );
        let result = decide_many(&raw, &env(Some("local"), None), &mut script, || 0.0, || {}).unwrap();
        assert_eq!(result.answers["big"]["choice"], "0");
        assert_eq!(script.calls.get(), 1);
    }

    #[test]
    fn many_local_sends_all_questions_in_one_request_and_typesafe_fails_entirely_on_missing_answer() {
        let mut local_script = ok_script(
            r#"{"model":"kev-4b","answers":{"urgent":{"type":"noul","noul":0.8},"team":{"type":"choice","choice":"infra","probabilities":{"infra":0.9,"app":0.1},"confidence":0.8}}}"#,
        );
        let result = decide_many(
            &many_raw(),
            &env(Some("local"), None),
            &mut local_script,
            || 0.0,
            || panic!("재시도하면 안 된다"),
        )
        .unwrap();
        assert_eq!(local_script.calls.get(), 1);
        assert_eq!(result.routing["backend"], "local");
        assert_eq!(result.answers["team"]["choice"], "infra");

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

    #[test]
    fn from_process_falls_back_to_config_file_when_env_is_unset() {
        let home = std::env::temp_dir().join(format!(
            "decide-backend-env-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(home.join(".config/decide")).unwrap();
        std::fs::write(
            home.join(".config/decide/config.toml"),
            "backend = \"local\"\n\n[typesafe]\napi_key = \"from-file\"\n",
        )
        .unwrap();

        std::env::remove_var("DECIDE_BACKEND");
        std::env::remove_var("TYPESAFE_API_KEY");
        std::env::set_var("HOME", &home);
        let env = Env::from_process();
        assert_eq!(env.backend, Some("local".to_string()));
        assert_eq!(env.api_key, Some("from-file".to_string()));

        // 환경변수가 있으면 같은 키의 TOML 값을 완전히 가린다.
        std::env::set_var("DECIDE_BACKEND", "typesafe");
        std::env::set_var("TYPESAFE_API_KEY", "from-env");
        let env = Env::from_process();
        assert_eq!(env.backend, Some("typesafe".to_string()));
        assert_eq!(env.api_key, Some("from-env".to_string()));

        // 빈 문자열 환경변수는 미설정과 같다 — TOML 값이 다시 보여야 한다.
        std::env::set_var("DECIDE_BACKEND", "");
        std::env::set_var("TYPESAFE_API_KEY", "");
        let env = Env::from_process();
        assert_eq!(env.backend, Some("local".to_string()));
        assert_eq!(env.api_key, Some("from-file".to_string()));

        std::env::remove_var("DECIDE_BACKEND");
        std::env::remove_var("TYPESAFE_API_KEY");
        std::env::remove_var("HOME");
        let _ = std::fs::remove_dir_all(&home);
    }
}
