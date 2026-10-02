// 로컬 백엔드 로짓을 TypeSafe 응답과 같은 JSON 틀로 변환한다
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
            let p = 1.0 / (1.0 + (-logits[0] as f64).exp());
            json!({"type": "noul", "noul": round4(p)})
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
    fn noul_logit_becomes_sigmoid_probability() {
        let q = Question::Noul { instructions: "참인가?".into() };
        let answer = to_answer(&q, &[0.0]);
        assert_eq!(answer["type"], "noul");
        let noul = answer["noul"].as_f64().unwrap();
        assert!((noul - 0.5).abs() < 1e-6);
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
