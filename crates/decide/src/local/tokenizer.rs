// Clef-flash 스키마 텍스트 조립과 질문/옵션 문자 오프셋 계산
use crate::protocol::Question;

pub const MAX_STATE_CHARS: usize = 48_000;

pub fn truncate_state(state: &str) -> &str {
    match state.char_indices().nth(MAX_STATE_CHARS) {
        Some((byte_idx, _)) => &state[..byte_idx],
        None => state,
    }
}

pub struct QuestionSpan {
    pub label: String,
    pub start_char: usize,
    pub end_char: usize,
}

pub struct OptionSpan {
    pub label: String,
    pub start_char: usize,
    pub end_char: usize,
}

fn kind_name(question: &Question) -> &'static str {
    match question {
        Question::Choice { .. } => "choice",
        Question::Score { .. } => "score",
        Question::Noul { .. } => "noul",
    }
}

fn instructions(question: &Question) -> &str {
    match question {
        Question::Choice { instructions, .. } => instructions,
        Question::Score { instructions, .. } => instructions,
        Question::Noul { instructions } => instructions,
    }
}

fn option_labels(question: &Question) -> Vec<&str> {
    match question {
        Question::Choice { options, .. } => options.iter().map(String::as_str).collect(),
        Question::Score { criteria, .. } => criteria.iter().map(String::as_str).collect(),
        Question::Noul { .. } => Vec::new(),
    }
}

pub fn schema_text(state: &str, question: &Question) -> String {
    let state = truncate_state(state);
    let mut text = String::new();
    text.push_str("FIELD q\n");
    text.push_str("ID:q\n");
    text.push_str(&format!("TYPE:{}\n", kind_name(question)));
    text.push_str(&format!("INSTRUCTION:{}\n", instructions(question)));
    for label in option_labels(question) {
        text.push_str(&format!("OPTION:{label}\n"));
    }
    let state_json = serde_json::to_string(state).unwrap_or_else(|_| "\"\"".to_string());
    text.push_str("STATE:");
    text.push_str(&state_json);
    text
}

pub fn spans(state: &str, question: &Question) -> (QuestionSpan, Vec<OptionSpan>) {
    let text = schema_text(state, question);
    let instr = instructions(question);
    let instr_start = text.find(instr).expect("instructions 텍스트가 schema_text 안에 있어야 한다");
    let question_span = QuestionSpan {
        label: "q".to_string(),
        start_char: instr_start,
        end_char: instr_start + instr.len(),
    };
    let mut option_spans = Vec::new();
    let mut search_from = instr_start + instr.len();
    for label in option_labels(question) {
        let marker = format!("OPTION:{label}");
        let marker_pos = text[search_from..].find(&marker).expect("옵션 마커를 찾을 수 없다") + search_from;
        let label_start = marker_pos + "OPTION:".len();
        let label_end = label_start + label.len();
        option_spans.push(OptionSpan {
            label: label.to_string(),
            start_char: label_start,
            end_char: label_end,
        });
        search_from = label_end;
    }
    (question_span, option_spans)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::Question;

    #[test]
    fn noul_schema_has_no_options_block() {
        let q = Question::Noul { instructions: "참인가?".into() };
        let text = schema_text("상태 문장", &q);
        assert!(text.contains("FIELD q"));
        assert!(text.contains("ID:q"));
        assert!(text.contains("TYPE:noul"));
        assert!(text.contains("INSTRUCTION:참인가?"));
        assert!(!text.contains("OPTION"));
        assert!(text.contains("\"상태 문장\""));
    }

    #[test]
    fn choice_schema_lists_options_in_order() {
        let q = Question::Choice {
            instructions: "어느 팀?".into(),
            options: vec!["billing".into(), "technical".into()],
        };
        let text = schema_text("s", &q);
        assert_eq!(text.find("billing").map(|i| i < text.find("technical").unwrap()), Some(true));
        assert!(text.contains("TYPE:choice"));
    }

    #[test]
    fn score_schema_lists_levels_in_order() {
        let q = Question::Score {
            instructions: "강도?".into(),
            criteria: vec!["낮음".into(), "높음".into()],
        };
        let text = schema_text("s", &q);
        assert_eq!(text.find("낮음").map(|i| i < text.find("높음").unwrap()), Some(true));
        assert!(text.contains("TYPE:score"));
    }

    #[test]
    fn state_is_json_serialized_so_embedded_headers_cannot_collide() {
        let q = Question::Noul { instructions: "참인가?".into() };
        let tricky_state = "FIELD q\nID:q\nTYPE:choice";
        let text = schema_text(tricky_state, &q);
        // state는 JSON 문자열로 들어가므로 원문 그대로의 "FIELD q" 줄바꿈은
        // 스키마 헤더가 아니라 JSON 문자열 리터럴 안에 있어야 한다.
        assert!(text.contains("\"FIELD q\\nID:q\\nTYPE:choice\""));
    }

    #[test]
    fn long_state_is_truncated_before_assembly() {
        let long_state = "가".repeat(MAX_STATE_CHARS + 1000);
        let truncated = truncate_state(&long_state);
        assert!(truncated.chars().count() <= MAX_STATE_CHARS);
    }

    #[test]
    fn spans_locate_question_and_options_by_char_offset() {
        let q = Question::Choice {
            instructions: "어느 팀?".into(),
            options: vec!["billing".into(), "technical".into()],
        };
        let state = "s";
        let (question_span, option_spans) = spans(state, &q);
        let text = schema_text(state, &q);
        assert_eq!(&text[question_span.start_char..question_span.end_char], "어느 팀?");
        assert_eq!(option_spans.len(), 2);
        assert_eq!(&text[option_spans[0].start_char..option_spans[0].end_char], "billing");
        assert_eq!(&text[option_spans[1].start_char..option_spans[1].end_char], "technical");
    }
}
