use serde::Serialize;
use serde_json::Value;

pub const CHOICE_TOO_FEW: &str = "choice 타입은 서로 다른 옵션이 최소 2개 필요합니다";
pub const SCORE_TOO_FEW: &str = "score 타입은 등급이 최소 2개 필요합니다";
pub const NOUL_EXTRA: &str = "noul 타입은 options/criteria를 받지 않습니다";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Choice,
    Score,
    Noul,
}

#[derive(Debug, Clone, PartialEq, Eq)]
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
