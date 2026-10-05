// 내장 정적 규칙이 평가 세트에서 어떻게 반응하는지 모델 없이 확정적으로 검사한다(기본 테스트 스위트에 들어간다)
//
// 두 가지를 지킨다. (1) 어느 세트든 정상(allow 라벨) 명령에 규칙이 걸리지 않는다(오탐 0건).
// (2) heldout3의 `rule_expect`(규칙을 쓰기 전에 미리 적어 둔 반응)와 규칙의 실제 반응이 같다.
use decide::gate::bash_risk::Verdict;
use decide::gate::config;
use decide::gate::rules;
use serde_json::Value;

const SETS: [&str; 4] =
    ["bash_risk_dev.json", "bash_risk_heldout.json", "bash_risk_heldout2.json", "bash_risk_heldout3.json"];

fn load(file: &str) -> Vec<Value> {
    let path = format!("{}/tests/gate_fixtures/{file}", env!("CARGO_MANIFEST_DIR"));
    let text = std::fs::read_to_string(&path).unwrap_or_else(|err| panic!("{path}: {err}"));
    serde_json::from_str(&text).unwrap()
}

fn rule_reaction(command: &str) -> Option<(Verdict, String)> {
    let settings = config::builtin().bash_risk;
    rules::judge(command, &settings.deny_patterns, &settings.ask_patterns)
        .map(|(verdict, pattern)| (verdict, pattern.to_string()))
}

fn name(verdict: Option<Verdict>) -> &'static str {
    match verdict {
        Some(Verdict::Deny) => "deny",
        Some(Verdict::Ask) => "ask",
        _ => "none",
    }
}

#[test]
fn no_allow_labelled_command_in_any_set_is_caught_by_a_default_rule() {
    let mut caught = Vec::new();
    let mut checked = 0;
    for file in SETS {
        for item in load(file) {
            if item["expect"] != "allow" {
                continue;
            }
            checked += 1;
            let command = item["command"].as_str().unwrap();
            if let Some((verdict, pattern)) = rule_reaction(command) {
                caught.push(format!("{file}: {command:?} → {} (패턴 {pattern:?})", name(Some(verdict))));
            }
        }
    }
    assert!(checked >= 40, "정상 명령을 {checked}건만 검사했다");
    assert!(caught.is_empty(), "정상 명령에 규칙이 걸렸다 ({}건)\n{}", caught.len(), caught.join("\n"));
}

#[test]
fn the_rules_match_the_preregistered_rule_expect_on_heldout3() {
    let mut mismatches = Vec::new();
    let items = load("bash_risk_heldout3.json");
    for item in &items {
        let command = item["command"].as_str().unwrap();
        let expected = item["rule_expect"].as_str().unwrap();
        let got = rule_reaction(command).map(|(verdict, _)| verdict);
        if name(got) != expected {
            mismatches.push(format!("{command:?}: 사전 등록 {expected}, 실제 {}", name(got)));
        }
    }
    assert_eq!(items.len(), 30);
    assert!(mismatches.is_empty(), "{}건 어긋남\n{}", mismatches.len(), mismatches.join("\n"));
}
