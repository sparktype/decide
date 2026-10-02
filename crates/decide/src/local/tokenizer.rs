// Clef-flash 스키마 텍스트 조립과 질문/옵션 문자 오프셋 계산.
//
// Task 7에서 Cloudflare/clef-flash의 실제 `joint_schema_model.py::encode_record`
// 소스(https://huggingface.co/Cloudflare/clef-flash/raw/main/joint_schema_model.py)를
// 직접 받아 대조한 결과, 이전 구현(Task 2)의 스키마 텍스트 형식이 실제 모델의
// 입력 형식과 달랐다 — 모델 카드 설명을 추측해 만든 자체 포맷이었다. 모델은
// 특정 문자열 포맷 하나에 fine-tune된 것이 아니라, 그 문자열이 토큰 ID로
// 어떻게 쪼개지는지(토큰 경계)까지 학습 시점과 똑같아야 한다. 그래서 여기서는
// `encode_record`가 조립하는 세그먼트들을 그대로 재현하고, 각 세그먼트를
// `encode_record`와 동일하게 **개별적으로** 토큰화한다(세그먼트를 하나의
// 문자열로 합친 뒤 토큰화하면 BPE 병합이 세그먼트 경계를 넘어 발생할 수 있어
// 토큰 ID 시퀀스가 달라진다 — 직접 검증했다. 예: "...:\n" + "\nFIELD..."를
// 합쳐서 토큰화하면 "\n\n"이 한 토큰으로 병합되지만, 두 세그먼트를 각각
// 토큰화하면 "\n"이 두 개의 별도 토큰으로 남는다).
//
// `spans()`가 돌려주는 문자 오프셋은 전체 조립된 텍스트(모든 세그먼트를
// 이어붙인 문자열) 기준이다 — `joint_head.rs`가 HuggingFace `tokenizers`
// crate의 `Encoding::get_offsets()`(바이트 오프셋)와 맞춰 문자 스팬을 토큰
// 스팬으로 변환한다.
use crate::protocol::Question;
use serde_json::json;

pub const MAX_STATE_CHARS: usize = 48_000;

pub const SYSTEM_PROMPT: &str = "Read the complete state and schema. Decide every field jointly. Each answer must be exactly one of that field's allowed options.";

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

/// 실제 소스의 `QUESTION_TYPES = {"noul": 0, "choice": 1, "score": 2}`와
/// 동일한 인코딩 — `joint_head.rs`의 `type_embedding` 조회에 쓴다.
pub fn question_type_id(question: &Question) -> u32 {
    match question {
        Question::Noul { .. } => 0,
        Question::Choice { .. } => 1,
        Question::Score { .. } => 2,
    }
}

fn instructions(question: &Question) -> &str {
    match question {
        Question::Choice { instructions, .. } => instructions,
        Question::Score { instructions, .. } => instructions,
        Question::Noul { instructions } => instructions,
    }
}

/// `joint_schema_model.py::question_options`와 동일한 (option_id, description)
/// 순서. noul은 true/false 고정, choice는 criteria 키를 문자열 정렬, score는
/// criteria(설명) 목록을 원래 순서대로 0부터 인덱싱한다.
fn option_entries(question: &Question) -> Vec<(String, Option<String>)> {
    match question {
        Question::Noul { .. } => vec![
            (
                "true".to_string(),
                Some("The proposition is true or the answer is yes.".to_string()),
            ),
            (
                "false".to_string(),
                Some("The proposition is false or the answer is no.".to_string()),
            ),
        ],
        Question::Choice { options, .. } => {
            let mut ids: Vec<&String> = options.iter().collect();
            ids.sort();
            ids.into_iter()
                .map(|id| (id.clone(), Some(id.clone())))
                .collect()
        }
        Question::Score { criteria, .. } => criteria
            .iter()
            .enumerate()
            .map(|(index, description)| (index.to_string(), Some(description.clone())))
            .collect(),
    }
}

/// `encode_record`가 토크나이저에 넘기는 세그먼트 그대로. 세그먼트 경계가
/// 토큰화 결과에 영향을 주므로(위 모듈 주석 참고), 호출자는 각 세그먼트를
/// 개별적으로 토큰화해야 한다. 반환값의 각 항목은 (세그먼트 텍스트, 이
/// 세그먼트가 QuestionSpan/OptionSpan 경계인지)이다 — `spans()`가 문자
/// 오프셋을 계산할 때 그대로 재사용한다.
pub enum Segment {
    Text(String),
    QuestionStart,
    QuestionEnd,
    OptionStart(usize),
    OptionEnd(usize),
}

pub fn segments(state: &str, question: &Question) -> Vec<Segment> {
    let state = truncate_state(state);
    let mut segs = Vec::new();
    segs.push(Segment::Text(format!(
        "<|im_start|>system\n{SYSTEM_PROMPT}<|im_end|>\n<|im_start|>user\nSTATE:\n"
    )));
    segs.push(Segment::Text(state.to_string()));
    segs.push(Segment::Text("\n\nSCHEMA FIELDS:\n".to_string()));
    segs.push(Segment::Text(format!(
        "\nFIELD 1\nID: q\nTYPE: {}\nINSTRUCTION: ",
        kind_name(question)
    )));
    segs.push(Segment::QuestionStart);
    segs.push(Segment::Text(instructions(question).to_string()));
    segs.push(Segment::QuestionEnd);
    segs.push(Segment::Text("\nALLOWED OPTIONS:\n".to_string()));
    for (index, (option_id, description)) in option_entries(question).into_iter().enumerate() {
        segs.push(Segment::Text(format!("OPTION {}: ", index + 1)));
        segs.push(Segment::OptionStart(index));
        let semantics = match &description {
            Some(description) => json!({"description": description, "option_id": option_id}),
            None => json!({"option_id": option_id}),
        };
        segs.push(Segment::Text(
            serde_json::to_string(&semantics).unwrap_or_else(|_| "{}".to_string()),
        ));
        segs.push(Segment::OptionEnd(index));
        segs.push(Segment::Text("\n".to_string()));
    }
    segs.push(Segment::Text("END FIELD\n".to_string()));
    segs.push(Segment::Text(
        "\n<|im_end|>\n<|im_start|>assistant\n<think>\n\n</think>\n\nJOINT SCHEMA DECISIONS:"
            .to_string(),
    ));
    segs
}

/// 모든 텍스트 세그먼트를 이어붙인 전체 문자열. 토큰화에는 쓰지 않는다
/// (세그먼트별 토큰화가 필요 — 모듈 주석 참고) — 디버깅과 `spans()`의 문자
/// 오프셋 좌표계로만 쓴다.
pub fn schema_text(state: &str, question: &Question) -> String {
    let mut text = String::new();
    for seg in segments(state, question) {
        if let Segment::Text(part) = seg {
            text.push_str(&part);
        }
    }
    text
}

pub fn spans(state: &str, question: &Question) -> (QuestionSpan, Vec<OptionSpan>) {
    let mut cursor = 0usize;
    let mut question_start = None;
    let mut question_end = None;
    let mut option_starts: Vec<Option<usize>> = Vec::new();
    let mut option_ends: Vec<Option<usize>> = Vec::new();
    let option_labels: Vec<String> = option_entries(question)
        .into_iter()
        .map(|(id, _)| id)
        .collect();
    for _ in &option_labels {
        option_starts.push(None);
        option_ends.push(None);
    }

    for seg in segments(state, question) {
        match seg {
            Segment::Text(part) => cursor += part.len(),
            Segment::QuestionStart => question_start = Some(cursor),
            Segment::QuestionEnd => question_end = Some(cursor),
            Segment::OptionStart(index) => option_starts[index] = Some(cursor),
            Segment::OptionEnd(index) => option_ends[index] = Some(cursor),
        }
    }

    let question_span = QuestionSpan {
        label: "q".to_string(),
        start_char: question_start.expect("QuestionStart 세그먼트가 있어야 한다"),
        end_char: question_end.expect("QuestionEnd 세그먼트가 있어야 한다"),
    };
    let option_spans = option_labels
        .into_iter()
        .zip(option_starts)
        .zip(option_ends)
        .map(|((label, start), end)| OptionSpan {
            label,
            start_char: start.expect("OptionStart 세그먼트가 있어야 한다"),
            end_char: end.expect("OptionEnd 세그먼트가 있어야 한다"),
        })
        .collect();
    (question_span, option_spans)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::Question;

    #[test]
    fn noul_schema_has_both_fixed_options() {
        let q = Question::Noul { instructions: "참인가?".into() };
        let text = schema_text("상태 문장", &q);
        assert!(text.contains("TYPE: noul"));
        assert!(text.contains("INSTRUCTION: 참인가?"));
        assert!(text.contains("\"option_id\":\"true\""));
        assert!(text.contains("\"option_id\":\"false\""));
        assert!(text.contains("상태 문장"));
    }

    #[test]
    fn choice_schema_lists_options_sorted_by_id() {
        let q = Question::Choice {
            instructions: "어느 팀?".into(),
            options: vec!["technical".into(), "billing".into()],
        };
        let text = schema_text("s", &q);
        assert_eq!(
            text.find("billing").map(|i| i < text.find("technical").unwrap()),
            Some(true)
        );
        assert!(text.contains("TYPE: choice"));
    }

    #[test]
    fn score_schema_lists_levels_in_original_order() {
        let q = Question::Score {
            instructions: "강도?".into(),
            criteria: vec!["낮음".into(), "높음".into()],
        };
        let text = schema_text("s", &q);
        assert_eq!(
            text.find("낮음").map(|i| i < text.find("높음").unwrap()),
            Some(true)
        );
        assert!(text.contains("TYPE: score"));
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
        assert_eq!(option_spans[0].label, "billing");
        assert_eq!(option_spans[1].label, "technical");
        // 옵션 스팬은 OPTION N: 뒤의 JSON 리터럴 전체(semantics 객체)를 가리킨다.
        assert!(text[option_spans[0].start_char..option_spans[0].end_char].contains("billing"));
        assert!(text[option_spans[1].start_char..option_spans[1].end_char].contains("technical"));
    }

    #[test]
    fn noul_option_spans_cover_true_and_false_descriptions() {
        let q = Question::Noul { instructions: "q".into() };
        let state = "s";
        let (_, option_spans) = spans(state, &q);
        let text = schema_text(state, &q);
        assert_eq!(option_spans.len(), 2);
        assert_eq!(option_spans[0].label, "true");
        assert_eq!(option_spans[1].label, "false");
        assert!(text[option_spans[0].start_char..option_spans[0].end_char].contains("true"));
        assert!(text[option_spans[1].start_char..option_spans[1].end_char].contains("false"));
    }
}
