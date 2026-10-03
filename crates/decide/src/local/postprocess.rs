// 로컬 백엔드 로짓을 TypeSafe 응답과 같은 JSON 틀로 변환한다.
//
// 실제 Cloudflare/clef-flash 소스(joint_schema_model.py::systemone_answer)를
// Task 7에서 대조한 결과, noul도 choice/score처럼 JointSchemaHead가 옵션당
// 로짓 하나씩을 내고(옵션은 항상 true/false 둘), 그 둘을 softmax한 뒤
// probabilities["true"]를 noul 확률로 쓴다 — 로짓 하나를 시그모이드에 넣는
// 방식(Task 5가 가정했던 것)이 아니다.
//
// 리뷰에서 발견된 버그(fix round 1/5): 이전에는 `logits: &[f32]`를 받아
// `Question::Choice.options`(호출자가 제공한, 정렬되지 않았을 수 있는
// 원래 순서)와 **위치로** zip했다. 하지만 `tokenizer::option_entries()`는
// choice를 알파벳 정렬해 스키마 텍스트를 만들고, `JointHead::score`가
// 돌려주는 로짓은 그 정렬된 순서다. 호출자가 `options: ["technical",
// "billing"]`처럼 이미 정렬되지 않은 배열을 주면, 모델은 `["billing",
// "technical"]` 순서로 로짓을 계산했는데 이 함수는 `logits[0]`을
// "technical"에 붙이는 식으로 라벨이 뒤바뀐다 — 겉보기엔 정상(유효한
// JSON, 확률 합이 1)인데 라벨만 틀린 결과가 나온다. 이제 `logits`를
// `&[(String, f32)]`(옵션 라벨, 로짓) 쌍으로 받아 위치가 아니라 라벨로
// 직접 찾는다 — 호출부(`JointHead::score`/`local::score`)가 이미 같은
// 라벨을 들고 있으므로 "두 리스트가 같은 순서"라는 암묵적 가정이
// 코드 어디에도 남지 않는다.
use crate::protocol::Question;
use serde_json::{json, Map, Value};

fn softmax(logits: &[f32]) -> Vec<f64> {
    let max = logits.iter().cloned().fold(f32::NEG_INFINITY, f32::max) as f64;
    let exps: Vec<f64> = logits.iter().map(|&x| ((x as f64) - max).exp()).collect();
    let sum: f64 = exps.iter().sum();
    exps.into_iter().map(|v| v / sum).collect()
}

fn round4(x: f64) -> f64 {
    (x * 10_000.0).round() / 10_000.0
}

/// `labeled_logits`의 각 라벨에 해당하는 확률을 순서대로 찾는다 — 라벨이
/// 없으면 panic(호출부 버그를 바로 드러내기 위해 조용히 0을 채우지 않는다).
fn probability_for<'a>(labels: &[&'a str], probs: &[f64], label: &str) -> f64 {
    let idx = labels
        .iter()
        .position(|candidate| *candidate == label)
        .unwrap_or_else(|| panic!("joint_head가 \"{label}\" 라벨의 로짓을 돌려주지 않았다"));
    probs[idx]
}

pub fn to_answer(question: &Question, labeled_logits: &[(String, f32)]) -> Value {
    let labels: Vec<&str> = labeled_logits.iter().map(|(label, _)| label.as_str()).collect();
    let logits: Vec<f32> = labeled_logits.iter().map(|(_, logit)| *logit).collect();
    let probs = softmax(&logits);

    match question {
        Question::Noul { .. } => {
            json!({"type": "noul", "noul": round4(probability_for(&labels, &probs, "true"))})
        }
        Question::Choice { options, .. } => {
            let mut probabilities = Map::new();
            for option in options {
                probabilities.insert(option.clone(), json!(round4(probability_for(&labels, &probs, option))));
            }
            let (best_option, best_prob) = options
                .iter()
                .map(|option| (option, probability_for(&labels, &probs, option)))
                .max_by(|a, b| a.1.partial_cmp(&b.1).unwrap())
                .unwrap();
            json!({
                "type": "choice",
                "choice": best_option,
                "confidence": round4(best_prob),
                "probabilities": Value::Object(probabilities),
            })
        }
        Question::Score { criteria, .. } => {
            // score의 라벨은 tokenizer::option_entries()가 매긴 인덱스
            // 문자열("0", "1", ...)이다 — criteria의 위치와 그 인덱스가
            // 항상 일치한다(score는 정렬하지 않고 원래 순서를 쓰므로).
            let mut probabilities = Map::new();
            for (index, level) in criteria.iter().enumerate() {
                probabilities.insert(level.clone(), json!(round4(probability_for(&labels, &probs, &index.to_string()))));
            }
            let expected: f64 = criteria
                .iter()
                .enumerate()
                .map(|(index, _)| index as f64 * probability_for(&labels, &probs, &index.to_string()))
                .sum();
            let best_prob = criteria
                .iter()
                .enumerate()
                .map(|(index, _)| probability_for(&labels, &probs, &index.to_string()))
                .fold(f64::NEG_INFINITY, f64::max);
            json!({
                "type": "score",
                "score": round4(expected),
                "confidence": round4(best_prob),
                "legend": criteria,
                "probabilities": Value::Object(probabilities),
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::Question;
    use serde_json::json;

    fn labeled(pairs: &[(&str, f32)]) -> Vec<(String, f32)> {
        pairs.iter().map(|(label, logit)| (label.to_string(), *logit)).collect()
    }

    #[test]
    fn noul_logits_become_softmax_true_probability() {
        // 실제 모델은 noul도 [true, false] 두 옵션 로짓을 낸다 — 모듈 상단
        // 주석 참고.
        let q = Question::Noul { instructions: "참인가?".into() };
        let answer = to_answer(&q, &labeled(&[("true", 0.0), ("false", 0.0)]));
        assert_eq!(answer["type"], "noul");
        let noul = answer["noul"].as_f64().unwrap();
        assert!((noul - 0.5).abs() < 1e-6);

        let confident = to_answer(&q, &labeled(&[("true", 2.0), ("false", 0.0)]));
        let noul = confident["noul"].as_f64().unwrap();
        assert!(noul > 0.5);
    }

    #[test]
    fn choice_logits_become_softmax_with_argmax_choice() {
        let q = Question::Choice {
            instructions: "어느 팀?".into(),
            options: vec!["billing".into(), "technical".into()],
        };
        let answer = to_answer(&q, &labeled(&[("billing", 2.0), ("technical", 0.0)]));
        assert_eq!(answer["type"], "choice");
        assert_eq!(answer["choice"], "billing");
        let probs = &answer["probabilities"];
        assert!(probs["billing"].as_f64().unwrap() > probs["technical"].as_f64().unwrap());
        let sum = probs["billing"].as_f64().unwrap() + probs["technical"].as_f64().unwrap();
        assert!((sum - 1.0).abs() < 1e-6);
    }

    #[test]
    fn choice_labels_do_not_swap_when_option_spans_arrive_in_a_different_order() {
        // 리뷰에서 발견된 버그 재발 방지: 호출자가 준 options 순서가
        // joint_head가 돌려주는(알파벳 정렬) 라벨 순서와 다를 때도
        // 라벨이 뒤바뀌면 안 된다. 여기서는 options를 정렬되지 않은
        // 순서(["technical","billing"])로 주고, 로짓은 그와 다른
        // 순서(["billing","technical"], 실제 joint_head가 돌려주는
        // 정렬된 순서)로 라벨링해서 넘긴다 — 이전 버그라면 logits[0](실은
        // billing의 로짓)을 "technical"에 붙였을 것이다.
        let q = Question::Choice {
            instructions: "어느 팀?".into(),
            options: vec!["technical".into(), "billing".into()],
        };
        let answer = to_answer(&q, &labeled(&[("billing", 5.0), ("technical", -5.0)]));
        assert_eq!(answer["choice"], "billing");
        let probs = &answer["probabilities"];
        assert!(probs["billing"].as_f64().unwrap() > 0.99);
        assert!(probs["technical"].as_f64().unwrap() < 0.01);
    }

    #[test]
    fn score_logits_become_expected_value_over_levels() {
        let q = Question::Score {
            instructions: "강도?".into(),
            criteria: vec!["낮음".into(), "중간".into(), "높음".into()],
        };
        let answer = to_answer(&q, &labeled(&[("0", 0.0), ("1", 0.0), ("2", 10.0)]));
        assert_eq!(answer["type"], "score");
        let score = answer["score"].as_f64().unwrap();
        assert!(score > 1.5, "거의 확실히 '높음'(인덱스 2)에 쏠려야 한다: {score}");
        assert_eq!(answer["legend"], json!(["낮음", "중간", "높음"]));
    }
}
