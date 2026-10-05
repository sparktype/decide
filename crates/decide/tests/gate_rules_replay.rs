// 실제 감사 로그(`gate.log`)의 명령에 내장 정적 규칙과 사전 필터를 재생해 어떤 명령이 걸리는지 보여 주는 점검 도구
//
// 실행: cargo test --test gate_rules_replay -- --ignored --nocapture
// 로그 위치는 `GATE_LOG`(없으면 `~/.cache/decide/gate.log`)다. 사람이 걸린 명령을 읽고 오탐인지 판단하므로
// 단언하지 않는다. 로그의 명령은 이미 비밀값을 가렸고 200자에서 잘려 있다.
use decide::gate::bash_risk::{self, Verdict};
use decide::gate::config;
use decide::gate::rules;
use serde_json::Value;
use std::collections::BTreeSet;

fn load_records() -> (String, Vec<Value>) {
    let path = std::env::var("GATE_LOG").unwrap_or_else(|_| {
        format!("{}/.cache/decide/gate.log", std::env::var("HOME").expect("HOME이 없다"))
    });
    let text = std::fs::read_to_string(&path).unwrap_or_else(|err| panic!("{path}: {err}"));
    let records = text.lines().filter_map(|line| serde_json::from_str(line).ok()).collect();
    (path, records)
}

#[test]
#[ignore] // 사용자의 실제 로그가 필요하다 — 기본 스위트에서 제외
fn replay_the_gate_log_through_the_default_rules() {
    let (path, records) = load_records();
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
    println!("\n== 감사 로그 재생: 정적 규칙 ({path}) ==");
    println!("로그 {}줄, 서로 다른 명령 {}개, 규칙에 걸린 명령 {}개", records.len(), commands.len(), hits.len());
    for hit in &hits {
        println!("{hit}");
    }
}

/// 새 기본 사전 필터가 실제 사용 호출을 얼마나 모델에서 빼는지, 그동안 어떤 판정을 받았는지 잰다.
/// 주의: 로그는 명령을 200자에서 잘라 `…`로 끝내 기록한다. 원래 뒤쪽에 있던 `&&`·`|` 같은 메타문자가 사라져,
/// 긴 명령은 재생에서 사전 필터에 걸리는 것처럼 보이는 착시가 생길 수 있다(`…`로 끝나는 항목을 의심한다).
#[test]
#[ignore] // 사용자의 실제 로그가 필요하다 — 기본 스위트에서 제외
fn replay_the_gate_log_through_the_default_prefilter() {
    let (path, records) = load_records();
    let settings = config::builtin().bash_risk;
    // 기록 시점에 모델이 판정했던 호출만 본다(그때 이미 사전 필터·규칙·실패였던 호출은 제외).
    let judged: Vec<&Value> = records.iter().filter(|record| record["verdict"].is_string() && record["rule"].is_null()).collect();
    let newly_skipped: Vec<&&Value> = judged
        .iter()
        .filter(|record| {
            let command = record["command"].as_str().unwrap_or("");
            rules::judge(command, &settings.deny_patterns, &settings.ask_patterns).is_none()
                && bash_risk::prefiltered(command, &settings.prefilter)
        })
        .collect();
    let latency: f64 = newly_skipped.iter().filter_map(|record| record["latency_ms"].as_f64()).sum();
    let count = |verdict: &str| newly_skipped.iter().filter(|record| record["verdict"] == verdict).count();
    println!("\n== 감사 로그 재생: 새 기본 사전 필터 ({path}) ==");
    println!("모델이 판정했던 호출 {}건 중 새로 사전 필터에 걸려 모델을 안 부르게 되는 호출 {}건", judged.len(), newly_skipped.len());
    if !judged.is_empty() {
        println!("비율 {:.1}%", newly_skipped.len() as f64 * 100.0 / judged.len() as f64);
    }
    println!(
        "그 호출들이 그동안 받은 판정: allow {}건, ask {}건, deny {}건 (ask·deny는 사라지는 마찰)",
        count("allow"),
        count("ask"),
        count("deny")
    );
    println!("그 호출들의 모델 지연 합계 {:.1}초(평균 {:.0}ms)", latency / 1000.0, latency / newly_skipped.len().max(1) as f64);
    println!("-- 새로 건너뛰는 명령 목록(위험한 것이 없는지 사람이 읽는다) --");
    let mut seen = BTreeSet::new();
    for record in &newly_skipped {
        let command = record["command"].as_str().unwrap_or("");
        if seen.insert(command.to_string()) {
            println!("[{}] {command}", record["verdict"].as_str().unwrap_or("?"));
        }
    }
}
