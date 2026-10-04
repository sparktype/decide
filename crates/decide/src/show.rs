// PostToolUse 훅 입력에서 decide의 질문과 결과를 사용자용 요약으로 만든다
use serde_json::Value;
use std::cmp::Ordering;

pub const TOOL_NAME: &str = "mcp__decide__decide";
const QUESTION_MAX_CHARS: usize = 80;
const REST_MAX: usize = 3;

pub fn render(input: &Value) -> Option<String> {
    if input.get("tool_name")?.as_str()? != TOOL_NAME {
        return None;
    }
    let result = result_json(input.get("tool_response")?)?;
    let answer = result.get("answer")?;
    let question = truncate(input.get("tool_input")?.get("instructions")?.as_str()?);
    let head = match answer.get("type")?.as_str()? {
        "choice" => choice_text(answer, &question)?,
        "noul" => format!(
            "🔎 decide 판단: {question} → {}%",
            pct(answer.get("noul")?.as_f64()?)
        ),
        "score" => score_text(answer, &question)?,
        _ => return None,
    };
    Some(format!("{head}\n   {}", footer(&result)))
}

fn result_json(response: &Value) -> Option<Value> {
    let from_text = |text: &Value| -> Option<Value> { serde_json::from_str(text.as_str()?).ok() };
    let from_content =
        |content: &Value| content.get(0).and_then(|item| item.get("text")).and_then(from_text);
    let found = match response {
        Value::String(_) => from_text(response),
        Value::Array(_) => from_content(response),
        Value::Object(object) => {
            if object.get("isError").and_then(Value::as_bool) == Some(true) {
                return None;
            }
            if object.contains_key("answer") {
                Some(response.clone())
            } else {
                object
                    .get("structuredContent")
                    .filter(|structured| structured.get("answer").is_some())
                    .cloned()
                    .or_else(|| object.get("content").and_then(from_content))
            }
        }
        _ => None,
    };
    found.filter(|value| value.get("answer").is_some())
}

pub(crate) fn truncate(question: &str) -> String {
    if question.chars().count() <= QUESTION_MAX_CHARS {
        return question.to_string();
    }
    let head: String = question.chars().take(QUESTION_MAX_CHARS).collect();
    format!("{head}…")
}

pub(crate) fn pct(probability: f64) -> i64 {
    (probability * 100.0).round() as i64
}

fn choice_text(answer: &Value, question: &str) -> Option<String> {
    let choice = answer.get("choice")?.as_str()?;
    let probabilities = answer.get("probabilities")?.as_object()?;
    let chosen = probabilities.get(choice)?.as_f64()?;
    let mut rest: Vec<(&str, f64)> = probabilities
        .iter()
        .filter(|(label, _)| label.as_str() != choice)
        .filter_map(|(label, value)| Some((label.as_str(), value.as_f64()?)))
        .collect();
    rest.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(Ordering::Equal));
    let others = rest
        .iter()
        .take(REST_MAX)
        .map(|(label, probability)| format!("{label} {}%", pct(*probability)))
        .collect::<Vec<_>>()
        .join(" · ");
    let mut out = format!(
        "🔎 decide가 선택했습니다: \"{choice}\" ({}%)\n   질문: {question}",
        pct(chosen)
    );
    if !others.is_empty() {
        out.push_str(&format!("\n   나머지: {others}"));
    }
    Some(out)
}

fn score_text(answer: &Value, question: &str) -> Option<String> {
    let score = answer.get("score")?.as_f64()?;
    let probabilities = answer.get("probabilities")?.as_object()?;
    let legend = answer.get("legend")?;
    let (top_key, top_probability) = probabilities
        .iter()
        .filter_map(|(key, value)| Some((key.as_str(), value.as_f64()?)))
        .max_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(Ordering::Equal))?;
    // TypeSafe는 인덱스 문자열을 키로 하는 객체, 로컬은 같은 순서의 배열로 legend를 돌려준다.
    let (label, levels) = match legend {
        Value::Object(map) => (map.get(top_key)?.as_str()?, map.len()),
        Value::Array(items) => (items.get(top_key.parse::<usize>().ok()?)?.as_str()?, items.len()),
        _ => return None,
    };
    let max = levels.saturating_sub(1);
    Some(format!(
        "🔎 decide 점수: {question} → 기대값 {score:.2}/{max}, 가장 가능성 높은 등급 \"{label}\" {}%",
        pct(top_probability)
    ))
}

pub(crate) fn footer(result: &Value) -> String {
    let routing = result.get("routing");
    let field = |key: &str| {
        routing
            .and_then(|routing| routing.get(key))
            .and_then(Value::as_str)
            .unwrap_or("?")
    };
    let cached = routing
        .and_then(|routing| routing.get("cached"))
        .and_then(Value::as_bool)
        == Some(true);
    let tail = if cached {
        "(캐시)".to_string()
    } else {
        match result.get("latency_ms").and_then(Value::as_f64) {
            Some(ms) => format!("{}ms", ms.round() as i64),
            None => "?".to_string(),
        }
    };
    format!("{} · {} · {tail}", field("backend"), field("model"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const CHOICE: &str = r#"{"answer":{"type":"choice","choice":"푸시하고 PR 생성","confidence":0.94,"probabilities":{"main에 로컬로 머지":0.01,"푸시하고 PR 생성":0.96,"브랜치를 그대로 유지":0.03,"작업을 폐기":0}},"routing":{"backend":"typesafe","model":"jev-1.13.0"},"latency_ms":203.4905}"#;
    const NOUL: &str = r#"{"answer":{"type":"noul","noul":0.56},"routing":{"backend":"typesafe","model":"jev-1.13.0"},"latency_ms":460.36216700000006}"#;
    const SCORE: &str = r#"{"answer":{"type":"score","score":1.17,"confidence":0.85,"legend":{"0":"거의 없음","1":"낮음","2":"보통","3":"높음","4":"매우 높음"},"probabilities":{"0":0.01,"1":0.83,"2":0.15,"3":0.01,"4":0}},"routing":{"backend":"typesafe","model":"jev-1.13.0"},"latency_ms":181.45366700000002}"#;

    const CHOICE_TEXT: &str = "🔎 decide가 선택했습니다: \"푸시하고 PR 생성\" (96%)\n   질문: 이 브랜치를 마무리하는 가장 적절한 방법은?\n   나머지: 브랜치를 그대로 유지 3% · main에 로컬로 머지 1% · 작업을 폐기 0%\n   typesafe · jev-1.13.0 · 203ms";
    const NOUL_TEXT: &str = "🔎 decide 판단: 이 변경은 머지해도 될 만큼 검증되었는가? → 56%\n   typesafe · jev-1.13.0 · 460ms";
    const SCORE_TEXT: &str = "🔎 decide 점수: 남아 있는 위험의 크기는? → 기대값 1.17/4, 가장 가능성 높은 등급 \"낮음\" 83%\n   typesafe · jev-1.13.0 · 181ms";

    fn hook_input(instructions: &str, response: Value) -> Value {
        json!({
            "hook_event_name": "PostToolUse",
            "tool_name": TOOL_NAME,
            "tool_input": {"state": "s", "type": "x", "instructions": instructions},
            "tool_response": response
        })
    }

    fn text(raw: &str) -> Value {
        Value::String(raw.to_string())
    }

    #[test]
    fn renders_a_local_score_answer_whose_legend_is_an_array() {
        let local = r#"{"answer":{"type":"score","score":0.8,"confidence":0.8,"legend":["낮음","높음"],"probabilities":{"0":0.2,"1":0.8}},"routing":{"backend":"local","model":"clef-flash"},"latency_ms":12.4}"#;
        assert_eq!(
            render(&hook_input("q", text(local))).unwrap(),
            "🔎 decide 점수: q → 기대값 0.80/1, 가장 가능성 높은 등급 \"높음\" 80%\n   local · clef-flash · 12ms"
        );
    }

    #[test]
    fn renders_the_three_types_exactly() {
        let choice = hook_input("이 브랜치를 마무리하는 가장 적절한 방법은?", text(CHOICE));
        assert_eq!(render(&choice).unwrap(), CHOICE_TEXT);
        let noul = hook_input("이 변경은 머지해도 될 만큼 검증되었는가?", text(NOUL));
        assert_eq!(render(&noul).unwrap(), NOUL_TEXT);
        let score = hook_input("남아 있는 위험의 크기는?", text(SCORE));
        assert_eq!(render(&score).unwrap(), SCORE_TEXT);
    }

    #[test]
    fn accepts_structured_content_and_content_text() {
        let question = "이 변경은 머지해도 될 만큼 검증되었는가?";
        let structured = json!({
            "content": [{"type": "text", "text": "무시된다"}],
            "structuredContent": serde_json::from_str::<Value>(NOUL).unwrap(),
            "isError": false
        });
        assert_eq!(render(&hook_input(question, structured)).unwrap(), NOUL_TEXT);
        let content_only = json!({
            "content": [{"type": "text", "text": NOUL}],
            "isError": false
        });
        assert_eq!(render(&hook_input(question, content_only)).unwrap(), NOUL_TEXT);
    }

    #[test]
    fn accepts_a_bare_content_array() {
        let response = json!([{"type": "text", "text": NOUL}]);
        assert_eq!(
            render(&hook_input("이 변경은 머지해도 될 만큼 검증되었는가?", response)).unwrap(),
            NOUL_TEXT
        );
    }

    #[test]
    fn accepts_an_object_that_is_the_result_itself() {
        let response = serde_json::from_str::<Value>(NOUL).unwrap();
        assert_eq!(
            render(&hook_input("이 변경은 머지해도 될 만큼 검증되었는가?", response)).unwrap(),
            NOUL_TEXT
        );
    }

    #[test]
    fn falls_back_to_content_text_when_structured_content_has_no_answer() {
        let response = json!({
            "structuredContent": {},
            "content": [{"type": "text", "text": NOUL}],
            "isError": false
        });
        assert_eq!(
            render(&hook_input("이 변경은 머지해도 될 만큼 검증되었는가?", response)).unwrap(),
            NOUL_TEXT
        );
    }

    #[test]
    fn stays_silent_for_anything_it_cannot_read() {
        let ok = hook_input("q", text(NOUL));
        let mut other_tool = ok.clone();
        other_tool["tool_name"] = json!("mcp__other__tool");
        assert_eq!(render(&other_tool), None);
        assert_eq!(render(&hook_input("q", text("not json"))), None);
        assert_eq!(render(&hook_input("q", text(r#"{"routing":{}}"#))), None);
        assert_eq!(
            render(&hook_input("q", text(r#"{"answer":{"type":"mystery"}}"#))),
            None
        );
        assert_eq!(
            render(&hook_input(
                "q",
                json!({"content": [{"type": "text", "text": "오류"}], "isError": true})
            )),
            None
        );
        assert_eq!(render(&hook_input("q", json!({}))), None);
        assert_eq!(render(&hook_input("q", json!(42))), None);
        let label_missing = r#"{"answer":{"type":"choice","choice":"x","probabilities":{"a":1.0}}}"#;
        assert_eq!(render(&hook_input("q", text(label_missing))), None);
        let mut no_tool_input = ok.clone();
        no_tool_input.as_object_mut().unwrap().remove("tool_input");
        assert_eq!(render(&no_tool_input), None);
        assert_eq!(render(&json!({})), None);
        assert_eq!(render(&json!("text")), None);
    }

    #[test]
    fn truncates_a_long_question_by_characters_and_limits_the_rest_to_three() {
        let long = "가".repeat(100);
        let out = render(&hook_input(&long, text(NOUL))).unwrap();
        assert!(out.contains(&format!("{}…", "가".repeat(80))));
        assert!(!out.contains(&"가".repeat(81)));

        let five = r#"{"answer":{"type":"choice","choice":"a","probabilities":{"a":0.5,"b":0.2,"c":0.15,"d":0.1,"e":0.05}},"routing":{"backend":"local","model":"m"},"latency_ms":10.0}"#;
        let out = render(&hook_input("q", text(five))).unwrap();
        assert!(out.contains("나머지: b 20% · c 15% · d 10%"));
        assert!(!out.contains("e 5%"));
    }

    #[test]
    fn equal_probabilities_do_not_panic_and_keep_input_order() {
        let tie = r#"{"answer":{"type":"choice","choice":"a","probabilities":{"a":0.4,"b":0.3,"c":0.3}},"routing":{"backend":"local","model":"m"},"latency_ms":1.0}"#;
        let out = render(&hook_input("q", text(tie))).unwrap();
        assert!(out.contains("나머지: b 30% · c 30%"));
        let score_tie = r#"{"answer":{"type":"score","score":0.5,"legend":{"0":"낮음","1":"높음"},"probabilities":{"0":0.5,"1":0.5}},"routing":{"backend":"local","model":"m"},"latency_ms":1.0}"#;
        assert!(render(&hook_input("q", text(score_tie))).is_some());
    }

    #[test]
    fn cached_results_show_a_marker_instead_of_latency() {
        let cached = r#"{"answer":{"type":"noul","noul":0.56},"routing":{"backend":"typesafe","model":"jev-1.13.0","cached":true},"latency_ms":0.0}"#;
        assert_eq!(
            render(&hook_input("q", text(cached))).unwrap(),
            "🔎 decide 판단: q → 56%\n   typesafe · jev-1.13.0 · (캐시)"
        );
    }
}
