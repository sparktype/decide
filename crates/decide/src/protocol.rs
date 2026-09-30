use serde::Serialize;
use serde_json::Value;

pub const CHOICE_TOO_FEW: &str = "choice 타입은 서로 다른 옵션이 최소 2개 필요합니다";
pub const SCORE_TOO_FEW: &str = "score 타입은 등급이 최소 2개 필요합니다";
pub const NOUL_EXTRA: &str = "noul 타입은 options/criteria를 받지 않습니다";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Kind {
    Choice,
    Score,
    Noul,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Incoming {
    pub state: String,
    pub kind: Kind,
    pub instructions: String,
    pub options: Vec<String>,
    pub criteria: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Question {
    Choice {
        instructions: String,
        options: Vec<String>,
    },
    Score {
        instructions: String,
        criteria: Vec<String>,
    },
    Noul {
        instructions: String,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct DecideResult {
    pub answer: Value,
    pub routing: Value,
    pub latency_ms: f64,
}

pub const QUESTIONS_EMPTY: &str = "questions는 비어 있을 수 없습니다";
pub const QUESTIONS_NOT_OBJECT: &str = "questions는 객체여야 합니다";
pub const QUESTION_ID_EMPTY: &str = "질문 id는 비어 있을 수 없습니다";

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct IncomingMany {
    pub state: String,
    pub questions: Vec<(String, Incoming)>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct DecideManyResult {
    pub answers: Value,
    pub routing: Value,
    pub latency_ms: f64,
}

pub fn parse_arguments(value: &Value) -> Result<Incoming, String> {
    let obj = value
        .as_object()
        .ok_or_else(|| "요청은 JSON 객체여야 합니다".to_string())?;
    let state = match obj.get("state") {
        Some(Value::String(s)) => s.clone(),
        Some(_) => return Err("state는 문자열이어야 합니다".to_string()),
        None => return Err("state가 필요합니다".to_string()),
    };
    let instructions = match obj.get("instructions") {
        Some(Value::String(s)) => s.clone(),
        Some(_) => return Err("instructions는 문자열이어야 합니다".to_string()),
        None => return Err("instructions가 필요합니다".to_string()),
    };
    let kind = match obj.get("type") {
        Some(Value::String(s)) => parse_kind(s)?,
        Some(_) => return Err("type은 choice, score, noul 중 하나여야 합니다".to_string()),
        None => return Err("type이 필요합니다".to_string()),
    };
    let options = optional_string_list(obj.get("options"), "options")?;
    let criteria = optional_string_list(obj.get("criteria"), "criteria")?;
    Ok(Incoming {
        state,
        kind,
        instructions,
        options,
        criteria,
    })
}

pub fn validate(req: &Incoming) -> Result<Question, String> {
    match req.kind {
        Kind::Choice => {
            if req.options.is_empty() || distinct(&req.options) < 2 {
                return Err(CHOICE_TOO_FEW.to_string());
            }
            Ok(Question::Choice {
                instructions: req.instructions.clone(),
                options: req.options.clone(),
            })
        }
        Kind::Score => {
            if req.criteria.len() < 2 {
                return Err(SCORE_TOO_FEW.to_string());
            }
            Ok(Question::Score {
                instructions: req.instructions.clone(),
                criteria: req.criteria.clone(),
            })
        }
        Kind::Noul => {
            if !req.options.is_empty() || !req.criteria.is_empty() {
                return Err(NOUL_EXTRA.to_string());
            }
            Ok(Question::Noul {
                instructions: req.instructions.clone(),
            })
        }
    }
}

pub fn parse_many(value: &Value) -> Result<IncomingMany, String> {
    let obj = value
        .as_object()
        .ok_or_else(|| "요청은 JSON 객체여야 합니다".to_string())?;
    let state = match obj.get("state") {
        Some(Value::String(s)) => s.clone(),
        Some(_) => return Err("state는 문자열이어야 합니다".to_string()),
        None => return Err("state가 필요합니다".to_string()),
    };
    let map = match obj.get("questions") {
        Some(Value::Object(map)) => map,
        Some(_) => return Err(QUESTIONS_NOT_OBJECT.to_string()),
        None => return Err("questions가 필요합니다".to_string()),
    };
    if map.is_empty() {
        return Err(QUESTIONS_EMPTY.to_string());
    }
    let mut questions = Vec::with_capacity(map.len());
    for (id, question) in map {
        if id.is_empty() {
            return Err(QUESTION_ID_EMPTY.to_string());
        }
        let mut args = match question {
            Value::Object(args) => args.clone(),
            _ => return Err(format!("질문 \"{id}\": 질문은 객체여야 합니다")),
        };
        args.insert("state".to_string(), Value::String(state.clone()));
        let incoming = parse_arguments(&Value::Object(args))
            .map_err(|err| format!("질문 \"{id}\": {err}"))?;
        questions.push((id.clone(), incoming));
    }
    Ok(IncomingMany { state, questions })
}

pub fn validate_many(req: &IncomingMany) -> Result<Vec<(String, Question)>, String> {
    req.questions
        .iter()
        .map(|(id, incoming)| {
            validate(incoming)
                .map(|question| (id.clone(), question))
                .map_err(|err| format!("질문 \"{id}\": {err}"))
        })
        .collect()
}

fn parse_kind(raw: &str) -> Result<Kind, String> {
    match raw {
        "choice" => Ok(Kind::Choice),
        "score" => Ok(Kind::Score),
        "noul" => Ok(Kind::Noul),
        _ => Err("type은 choice, score, noul 중 하나여야 합니다".to_string()),
    }
}

fn optional_string_list(value: Option<&Value>, name: &str) -> Result<Vec<String>, String> {
    match value {
        None | Some(Value::Null) => Ok(Vec::new()),
        Some(Value::Array(items)) => items
            .iter()
            .map(|item| {
                item.as_str()
                    .map(str::to_string)
                    .ok_or_else(|| format!("{name}는 문자열 배열이어야 합니다"))
            })
            .collect(),
        Some(_) => Err(format!("{name}는 문자열 배열이어야 합니다")),
    }
}

fn distinct(items: &[String]) -> usize {
    let mut seen = Vec::new();
    for item in items {
        if !seen.contains(item) {
            seen.push(item.clone());
        }
    }
    seen.len()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn incoming(kind: Kind, options: &[&str], criteria: &[&str]) -> Incoming {
        Incoming {
            state: "상태".into(),
            kind,
            instructions: "질문".into(),
            options: options.iter().map(|s| (*s).to_string()).collect(),
            criteria: criteria.iter().map(|s| (*s).to_string()).collect(),
        }
    }

    #[test]
    fn choice_requires_two_distinct_options() {
        assert_eq!(
            validate(&incoming(Kind::Choice, &[], &[])).unwrap_err(),
            CHOICE_TOO_FEW
        );
        assert_eq!(
            validate(&incoming(Kind::Choice, &["a"], &[])).unwrap_err(),
            CHOICE_TOO_FEW
        );
        assert_eq!(
            validate(&incoming(Kind::Choice, &["a", "a"], &[])).unwrap_err(),
            CHOICE_TOO_FEW
        );
        let ok = validate(&incoming(Kind::Choice, &["a", "a", "b"], &[])).unwrap();
        match ok {
            Question::Choice { options, .. } => assert_eq!(options, vec!["a", "a", "b"]),
            other => panic!("choice가 아님: {other:?}"),
        }
    }

    fn many(value: Value) -> Result<IncomingMany, String> {
        parse_many(&value)
    }

    #[test]
    fn many_parses_questions_in_input_order_with_the_shared_state() {
        let parsed = many(json!({
            "state": "서버 다운",
            "questions": {
                "urgent": {"type": "noul", "instructions": "긴급한가?"},
                "team": {"type": "choice", "instructions": "어느 팀?", "options": ["billing", "infra"]},
                "anger": {"type": "score", "instructions": "불만?", "criteria": ["낮음", "높음"]}
            }
        }))
        .unwrap();
        assert_eq!(parsed.state, "서버 다운");
        let ids: Vec<&str> = parsed.questions.iter().map(|(id, _)| id.as_str()).collect();
        assert_eq!(ids, vec!["urgent", "team", "anger"]);
        assert!(parsed.questions.iter().all(|(_, q)| q.state == "서버 다운"));
        assert_eq!(parsed.questions[1].1.kind, Kind::Choice);
        assert_eq!(parsed.questions[1].1.options, vec!["billing", "infra"]);
    }

    #[test]
    fn many_rejects_bad_shapes() {
        assert_eq!(
            many(json!({"state": "s", "questions": []})).unwrap_err(),
            QUESTIONS_NOT_OBJECT
        );
        assert_eq!(
            many(json!({"state": "s", "questions": null})).unwrap_err(),
            QUESTIONS_NOT_OBJECT
        );
        assert_eq!(
            many(json!({"state": "s", "questions": {}})).unwrap_err(),
            QUESTIONS_EMPTY
        );
        assert_eq!(
            many(json!({"state": "s"})).unwrap_err(),
            "questions가 필요합니다"
        );
        assert_eq!(
            many(json!({"questions": {"a": {"type": "noul", "instructions": "?"}}})).unwrap_err(),
            "state가 필요합니다"
        );
        assert_eq!(
            many(json!({"state": 1, "questions": {"a": {"type": "noul", "instructions": "?"}}}))
                .unwrap_err(),
            "state는 문자열이어야 합니다"
        );
        assert_eq!(
            many(json!({"state": "s", "questions": {"": {"type": "noul", "instructions": "?"}}}))
                .unwrap_err(),
            QUESTION_ID_EMPTY
        );
        assert_eq!(
            many(json!({"state": "s", "questions": {"a": "noul"}})).unwrap_err(),
            "질문 \"a\": 질문은 객체여야 합니다"
        );
        assert_eq!(
            many(json!({"state": "s", "questions": {"a": {"type": "noul"}}})).unwrap_err(),
            "질문 \"a\": instructions가 필요합니다"
        );
    }

    #[test]
    fn many_validation_reuses_the_single_question_messages_with_the_id() {
        let bad_choice = many(json!({
            "state": "s",
            "questions": {
                "ok": {"type": "noul", "instructions": "?"},
                "team": {"type": "choice", "instructions": "?", "options": ["a"]}
            }
        }))
        .unwrap();
        assert_eq!(
            validate_many(&bad_choice).unwrap_err(),
            format!("질문 \"team\": {CHOICE_TOO_FEW}")
        );
        let bad_noul = many(json!({
            "state": "s",
            "questions": {"u": {"type": "noul", "instructions": "?", "options": ["a"]}}
        }))
        .unwrap();
        assert_eq!(
            validate_many(&bad_noul).unwrap_err(),
            format!("질문 \"u\": {NOUL_EXTRA}")
        );
        let good = many(json!({
            "state": "s",
            "questions": {
                "a": {"type": "noul", "instructions": "?"},
                "b": {"type": "score", "instructions": "?", "criteria": ["x", "y"]}
            }
        }))
        .unwrap();
        let questions = validate_many(&good).unwrap();
        assert_eq!(questions.len(), 2);
        assert_eq!(questions[0].0, "a");
        assert!(matches!(questions[1].1, Question::Score { .. }));
    }

    #[test]
    fn score_requires_two_levels() {
        assert_eq!(
            validate(&incoming(Kind::Score, &[], &["낮음"])).unwrap_err(),
            SCORE_TOO_FEW
        );
        assert!(validate(&incoming(Kind::Score, &[], &["낮음", "높음"])).is_ok());
    }

    #[test]
    fn noul_rejects_options_and_criteria() {
        assert_eq!(
            validate(&incoming(Kind::Noul, &["예"], &[])).unwrap_err(),
            NOUL_EXTRA
        );
        assert_eq!(
            validate(&incoming(Kind::Noul, &[], &["예"])).unwrap_err(),
            NOUL_EXTRA
        );
        assert!(validate(&incoming(Kind::Noul, &[], &[])).is_ok());
    }

    #[test]
    fn parse_treats_null_lists_as_empty() {
        let parsed = parse_arguments(&json!({
            "state": "상태",
            "type": "noul",
            "instructions": "참인가?",
            "options": null,
            "criteria": null
        }))
        .unwrap();
        assert_eq!(parsed.kind, Kind::Noul);
        assert!(parsed.options.is_empty());
        assert!(validate(&parsed).is_ok());
    }

    #[test]
    fn parse_rejects_missing_and_bad_types() {
        assert_eq!(
            parse_arguments(&json!({"type": "noul", "instructions": "?"})).unwrap_err(),
            "state가 필요합니다"
        );
        assert_eq!(
            parse_arguments(&json!({"state": 1, "type": "noul", "instructions": "?"})).unwrap_err(),
            "state는 문자열이어야 합니다"
        );
        assert_eq!(
            parse_arguments(&json!({"state": "s", "type": "maybe", "instructions": "?"}))
                .unwrap_err(),
            "type은 choice, score, noul 중 하나여야 합니다"
        );
        assert_eq!(
            parse_arguments(
                &json!({"state": "s", "type": "choice", "instructions": "?", "options": "a"})
            )
            .unwrap_err(),
            "options는 문자열 배열이어야 합니다"
        );
    }
}
