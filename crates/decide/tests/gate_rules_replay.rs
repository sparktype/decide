// 실제 감사 로그(`gate.log`)의 명령에 내장 정적 규칙을 재생해 어떤 명령이 걸리는지 보여 주는 점검 도구
//
// 실행: cargo test --test gate_rules_replay -- --ignored --nocapture
// 로그 위치는 `GATE_LOG`(없으면 `~/.cache/decide/gate.log`)다. 사람이 걸린 명령을 읽고 오탐인지 판단하므로
// 단언하지 않는다. 로그의 명령은 이미 비밀값을 가렸고 200자에서 잘려 있다.
use decide::gate::bash_risk::Verdict;
use decide::gate::config;
use decide::gate::rules;
use serde_json::Value;
use std::collections::BTreeSet;

#[test]
#[ignore] // 사용자의 실제 로그가 필요하다 — 기본 스위트에서 제외
fn replay_the_gate_log_through_the_default_rules() {
    let path = std::env::var("GATE_LOG").unwrap_or_else(|_| {
        format!("{}/.cache/decide/gate.log", std::env::var("HOME").expect("HOME이 없다"))
    });
    let text = std::fs::read_to_string(&path).unwrap_or_else(|err| panic!("{path}: {err}"));
    let records: Vec<Value> = text.lines().filter_map(|line| serde_json::from_str(line).ok()).collect();
    let commands: BTreeSet<String> =
        records.iter().filter_map(|record| record["command"].as_str().map(str::to_string)).collect();
    let settings = config::builtin().bash_risk;
    let mut hits = Vec::new();
    for command in &commands {
        if let Some((verdict, pattern)) = rules::judge(command, &settings.deny_patterns, &settings.ask_patterns) {
            let label = if verdict == Verdict::Deny { "deny" } else { "ask" };
            hits.push(format!("[{label}] {command}\n        ← 패턴 {pattern:?}"));
        }
    }
    println!("\n== 감사 로그 재생 ({path}) ==");
    println!("로그 {}줄, 서로 다른 명령 {}개, 규칙에 걸린 명령 {}개", records.len(), commands.len(), hits.len());
    for hit in &hits {
        println!("{hit}");
    }
}
