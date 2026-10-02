// 로컬 백엔드 로짓을 TypeSafe 응답과 같은 JSON 틀로 변환한다.
//
// 실제 Cloudflare/clef-flash 소스(joint_schema_model.py::systemone_answer)를
// Task 7에서 대조한 결과, noul도 choice/score처럼 JointSchemaHead가 옵션당
// 로짓 하나씩을 내고(옵션은 항상 true/false 둘), 그 둘을 softmax한 뒤
// probabilities["true"]를 noul 확률로 쓴다 — 로짓 하나를 시그모이드에 넣는
// 방식(Task 5가 가정했던 것)이 아니다. `tokenizer::option_entries`가 noul의
// 옵션을 항상 [true, false] 순서로 내놓으므로, `JointHead::score`가 돌려주는
// `logits[0]`이 true, `logits[1]`이 false에 대응한다.
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

pub fn to_answer(question: &Question, logits: &[f32]) -> Value {
    match question {
        Question::Noul { .. } => {
            let probs = softmax(logits);
            json!({"type": "noul", "noul": round4(probs[0])})
        }
        Question::Choice { options, .. } => {
            let probs = softmax(logits);
            let (best_idx, _) = probs
                .iter()
                .enumerate()
                .max_by(|a, b| a.1.partial_cmp(b.1).unwrap())
                .unwrap();
            let mut probabilities = Map::new();
            for (option, p) in options.iter().zip(probs.iter()) {
                probabilities.insert(option.clone(), json!(round4(*p)));
            }
            json!({
                "type": "choice",
                "choice": options[best_idx],
                "confidence": round4(probs[best_idx]),
                "probabilities": Value::Object(probabilities),
            })
        }
        Question::Score { criteria, .. } => {
            let probs = softmax(logits);
            let expected: f64 = probs.iter().enumerate().map(|(i, p)| i as f64 * p).sum();
            let (best_idx, _) = probs
                .iter()
                .enumerate()
                .max_by(|a, b| a.1.partial_cmp(b.1).unwrap())
                .unwrap();
            let mut probabilities = Map::new();
            for (level, p) in criteria.iter().zip(probs.iter()) {
                probabilities.insert(level.clone(), json!(round4(*p)));
            }
            json!({
                "type": "score",
                "score": round4(expected),
                "confidence": round4(probs[best_idx]),
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

    #[test]
    fn noul_logits_become_softmax_true_probability() {
        // 실제 모델은 noul도 [true, false] 두 옵션 로짓을 낸다 — 모듈 상단
        // 주석 참고.
        let q = Question::Noul { instructions: "참인가?".into() };
        let answer = to_answer(&q, &[0.0, 0.0]);
        assert_eq!(answer["type"], "noul");
        let noul = answer["noul"].as_f64().unwrap();
        assert!((noul - 0.5).abs() < 1e-6);

        let confident = to_answer(&q, &[2.0, 0.0]);
        let noul = confident["noul"].as_f64().unwrap();
        assert!(noul > 0.5);
    }

    #[test]
    fn choice_logits_become_softmax_with_argmax_choice() {
        let q = Question::Choice {
            instructions: "어느 팀?".into(),
            options: vec!["billing".into(), "technical".into()],
        };
        let answer = to_answer(&q, &[2.0, 0.0]);
        assert_eq!(answer["type"], "choice");
        assert_eq!(answer["choice"], "billing");
        let probs = &answer["probabilities"];
        assert!(probs["billing"].as_f64().unwrap() > probs["technical"].as_f64().unwrap());
        let sum = probs["billing"].as_f64().unwrap() + probs["technical"].as_f64().unwrap();
        assert!((sum - 1.0).abs() < 1e-6);
    }

    #[test]
    fn score_logits_become_expected_value_over_levels() {
        let q = Question::Score {
            instructions: "강도?".into(),
            criteria: vec!["낮음".into(), "중간".into(), "높음".into()],
        };
        let answer = to_answer(&q, &[0.0, 0.0, 10.0]);
        assert_eq!(answer["type"], "score");
        let score = answer["score"].as_f64().unwrap();
        assert!(score > 1.5, "거의 확실히 '높음'(인덱스 2)에 쏠려야 한다: {score}");
        assert_eq!(answer["legend"], json!(["낮음", "중간", "높음"]));
    }
}
