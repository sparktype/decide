#![cfg(feature = "parity")]
// Rust local 백엔드(백본 MLX 8비트 + joint_head F32)가 Cloudflare/clef-flash
// 원본 PyTorch 구현(F32로 돌린 오라클, `scripts/clef_flash_oracle.py`가 생성한
// `tests/parity_fixtures/golden.json`)과 같은 입력에 대해 "같은 질적 판단"을
// 내리는지 확인한다.
//
// 가중치가 로컬에 없으면(CLEF_WEIGHTS 미설정 + HF 캐시에도 없음) 이 테스트는
// 실행 중 자동으로 다운로드를 시도한다 — `local::ensure_weights()`가 MLX 체크포인트
// (`mlx-community/clef-flash-8bit`)를 받는다.
//
// 리뷰에서 발견된 버그(fix round 1/5): 원시 로짓 공간에서 `TOLERANCE`를
// 두고 "거리가 가까우면 통과"로 판정했었는데, 이 설계는 noul처럼 옵션이
// 2개뿐인 질문에서 치명적이다 — 로짓 2.4 차이 정도는 "거리상 가까움"
// 안에 들어오지만, softmax/sigmoid를 거치면 0.5 경계를 넘어 질적 판단이
// 뒤집힐 수 있다. 실측: "서버가 다운됐습니다" 케이스에서 Rust는
// p_true=0.546(→ true, 긴급함)인데 Python 오라클은 p_true=0.254(→ false,
// 긴급하지 않음)로 **판단 자체가 반대**였다 — 로짓 차이 2.4는
// TOLERANCE=4.2 안에 들어왔지만 실제로는 완전히 다른 결정이었다.
//
// 그래서 이 게이트는 이제 `postprocess::to_answer`가 실제로 만드는 답과
// 똑같은 후처리(softmax/sigmoid)를 Rust 로짓과 Python 오라클 로짓 양쪽에
// 적용한 뒤, **질적으로 같은 답**인지를 확인한다:
// - noul: 둘 다 0.5의 같은 쪽에 있어야 한다(둘 다 true 또는 둘 다 false).
// - choice: argmax 옵션이 같아야 한다.
// - score: argmax 등급(레벨)이 같아야 한다 — score는 레벨이 늘어날수록
//   인접 레벨 사이의 확률 차이가 작아지기 쉬워서(가장 거친 "같은 결정"
//   기준인 argmax조차 양자화 노이즈로 흔들릴 수 있다는 뜻이다), argmax가
//   다르더라도 기대값(expected value)이 1레벨 이내로 가깝다면 허용한다 —
//   "강도 1단계 차이" 정도는 사람이 봐도 받아들일 수 있는 수준의 질적
//   불일치이고, 그 이상(레벨 2개 이상 차이)은 실제로 다른 결정이라고
//   판단했다.
//
// 원시 로짓 거리는 더 이상 통과/실패를 가르지 않지만, 조사 때 쓸모가
// 있어 참고용으로 계속 출력한다(Task 7 조사에서 확인한 양자화 노이즈
// 패턴 — 자세한 내용은 task-7-report.md 참고).
use decide::local;
use decide::local::tokenizer::option_labels;
use decide::protocol::{validate, Incoming, Kind, Question};
use serde::Deserialize;
use serde_json::Value;
use std::collections::HashMap;

#[derive(Deserialize)]
struct Fixture {
    input: Value,
    logits: Vec<f64>,
}

fn softmax(logits: &[f64]) -> Vec<f64> {
    let max = logits.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
    let exps: Vec<f64> = logits.iter().map(|&x| (x - max).exp()).collect();
    let sum: f64 = exps.iter().sum();
    exps.into_iter().map(|v| v / sum).collect()
}

/// Python 오라클의 로짓(위치 기반 배열, `tokenizer::option_labels`가 정한
/// 순서)에 라벨을 붙인다 — Rust `local::raw_logits`가 이미 라벨을 들고
/// 돌려주는 것과 같은 좌표계로 맞춘다.
fn label_python_logits(question: &Question, logits: &[f64]) -> HashMap<String, f64> {
    let labels = option_labels(question);
    assert_eq!(
        labels.len(),
        logits.len(),
        "Python 오라클 로짓 개수가 tokenizer::option_labels와 다르다"
    );
    labels.into_iter().zip(logits.iter().copied()).collect()
}

fn label_rust_logits(labeled: &[(String, f32)]) -> HashMap<String, f64> {
    labeled.iter().map(|(label, logit)| (label.clone(), *logit as f64)).collect()
}

/// 질문 타입별로 "같은 질적 판단"인지 비교한다. 불일치면 자세한 사유를
/// 담은 `Err`를 돌려준다.
fn qualitative_agreement(
    question: &Question,
    rust_probs: &HashMap<String, f64>,
    python_probs: &HashMap<String, f64>,
) -> Result<(), String> {
    match question {
        Question::Noul { .. } => {
            let rust_true = rust_probs["true"];
            let python_true = python_probs["true"];
            let rust_side = rust_true >= 0.5;
            let python_side = python_true >= 0.5;
            if rust_side != python_side {
                return Err(format!(
                    "noul 판단이 반대다: rust p_true={rust_true:.4}({}), python p_true={python_true:.4}({})",
                    if rust_side { "true" } else { "false" },
                    if python_side { "true" } else { "false" }
                ));
            }
            Ok(())
        }
        Question::Choice { options, .. } => {
            let rust_best = options
                .iter()
                .max_by(|a, b| rust_probs[*a].partial_cmp(&rust_probs[*b]).unwrap())
                .unwrap();
            let python_best = options
                .iter()
                .max_by(|a, b| python_probs[*a].partial_cmp(&python_probs[*b]).unwrap())
                .unwrap();
            if rust_best != python_best {
                return Err(format!(
                    "choice argmax가 다르다: rust=\"{rust_best}\"(p={:.4}), python=\"{python_best}\"(p={:.4})",
                    rust_probs[rust_best], python_probs[python_best]
                ));
            }
            Ok(())
        }
        Question::Score { criteria, .. } => {
            let labels: Vec<String> = (0..criteria.len()).map(|i| i.to_string()).collect();
            let rust_argmax = labels
                .iter()
                .enumerate()
                .max_by(|(_, a), (_, b)| rust_probs[*a].partial_cmp(&rust_probs[*b]).unwrap())
                .map(|(i, _)| i)
                .unwrap();
            let python_argmax = labels
                .iter()
                .enumerate()
                .max_by(|(_, a), (_, b)| python_probs[*a].partial_cmp(&python_probs[*b]).unwrap())
                .map(|(i, _)| i)
                .unwrap();
            if rust_argmax == python_argmax {
                return Ok(());
            }
            let rust_expected: f64 =
                labels.iter().enumerate().map(|(i, label)| i as f64 * rust_probs[label]).sum();
            let python_expected: f64 =
                labels.iter().enumerate().map(|(i, label)| i as f64 * python_probs[label]).sum();
            let diff = (rust_expected - python_expected).abs();
            if diff <= 1.0 {
                return Ok(());
            }
            Err(format!(
                "score argmax도 다르고(rust={rust_argmax}, python={python_argmax}) 기대값도 1레벨 넘게 \
                 차이난다: rust_expected={rust_expected:.4}, python_expected={python_expected:.4}, diff={diff:.4}"
            ))
        }
    }
}

#[test]
fn rust_matches_python_oracle_qualitative_decision_on_all_golden_cases() {
    let raw = std::fs::read_to_string("tests/parity_fixtures/golden.json")
        .expect("골든 fixture가 있어야 한다 — scripts/clef_flash_oracle.py로 생성");
    let fixtures: Vec<Fixture> = serde_json::from_str(&raw).unwrap();
    assert!(!fixtures.is_empty(), "골든 fixture가 비어 있다");

    let mut max_abs_logit_diff = 0.0f64;
    let mut failures = Vec::new();

    for fixture in fixtures {
        let state = fixture.input["state"].as_str().unwrap().to_string();
        let q = &fixture.input["questions"]["q"];
        let kind = match q["type"].as_str().unwrap() {
            "noul" => Kind::Noul,
            "choice" => Kind::Choice,
            "score" => Kind::Score,
            other => panic!("알 수 없는 타입: {other}"),
        };
        let instructions = q["instructions"].as_str().unwrap().to_string();
        let options = q["criteria"]
            .as_object()
            .map(|m| m.keys().cloned().collect())
            .unwrap_or_default();
        let criteria = q["criteria"]
            .as_array()
            .map(|a| a.iter().map(|v| v.as_str().unwrap().to_string()).collect())
            .unwrap_or_default();

        let incoming = Incoming {
            state: state.clone(),
            kind,
            instructions,
            options: if kind == Kind::Choice { options } else { Vec::new() },
            criteria: if kind == Kind::Score { criteria } else { Vec::new() },
        };
        let question: Question = validate(&incoming).unwrap();

        let rust_labeled: Vec<(String, f32)> = local::raw_logits(&state, &question)
            .unwrap_or_else(|err| panic!("local::raw_logits 실패 (state={state:?}): {err}"));

        assert_eq!(
            rust_labeled.len(),
            fixture.logits.len(),
            "로짓 개수가 다르다 (state={state:?}): rust={rust_labeled:?} python={:?}",
            fixture.logits
        );

        let python_by_label = label_python_logits(&question, &fixture.logits);
        let rust_by_label = label_rust_logits(&rust_labeled);

        // 참고용 진단 — 더 이상 통과/실패를 가르지 않는다(모듈 상단 주석).
        for (label, rust_val) in &rust_by_label {
            let python_val = python_by_label[label];
            let diff = (rust_val - python_val).abs();
            max_abs_logit_diff = max_abs_logit_diff.max(diff);
            eprintln!("state={state:?} label={label:?}: rust={rust_val}, python={python_val}, diff={diff}");
        }

        // 실제 게이트: postprocess::to_answer와 같은 softmax를 양쪽에
        // 적용한 뒤 질적으로 같은 결정인지 비교한다.
        let rust_logit_vec: Vec<f64> = rust_by_label.values().copied().collect();
        let rust_labels: Vec<String> = rust_by_label.keys().cloned().collect();
        let rust_probs_vec = softmax(&rust_logit_vec);
        let rust_probs: HashMap<String, f64> =
            rust_labels.into_iter().zip(rust_probs_vec).collect();

        let python_logit_vec: Vec<f64> = python_by_label.values().copied().collect();
        let python_labels: Vec<String> = python_by_label.keys().cloned().collect();
        let python_probs_vec = softmax(&python_logit_vec);
        let python_probs: HashMap<String, f64> =
            python_labels.into_iter().zip(python_probs_vec).collect();

        if let Err(reason) = qualitative_agreement(&question, &rust_probs, &python_probs) {
            failures.push(format!("state={state:?}: {reason}"));
        }
    }

    eprintln!("관찰된 최대 로짓 차이(참고용, 더 이상 게이트 기준 아님): {max_abs_logit_diff}");
    assert!(
        failures.is_empty(),
        "{}개 케이스에서 질적 판단이 Python 오라클과 달랐다:\n{}",
        failures.len(),
        failures.join("\n")
    );
}
