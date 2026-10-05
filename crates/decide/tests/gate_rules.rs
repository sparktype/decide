// 내장 정적 규칙이 평가 세트에서 어떻게 반응하는지 모델 없이 확정적으로 검사한다(기본 테스트 스위트에 들어간다)
//
// 두 가지를 지킨다. (1) 어느 세트든 정상(allow 라벨) 명령에 규칙이 걸리지 않는다(오탐 0건).
// (2) heldout3의 `rule_expect`(규칙을 쓰기 전에 미리 적어 둔 반응)와 규칙의 실제 반응이 같다.
use decide::gate::bash_risk::{self, Verdict};
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

/// 실제 훅처럼 정적 규칙이 먼저이고, 규칙이 없을 때 사전 필터가 건너뛰는지 본다.
fn silently_skipped(command: &str) -> bool {
    let settings = config::builtin().bash_risk;
    rule_reaction(command).is_none() && bash_risk::prefiltered(command, &settings.prefilter)
}

#[test]
fn no_ask_or_deny_labelled_command_in_any_set_is_silently_skipped() {
    let mut skipped = Vec::new();
    for file in SETS {
        for item in load(file) {
            let command = item["command"].as_str().unwrap();
            if item["expect"] != "allow" && silently_skipped(command) {
                skipped.push(format!("{file}: {command:?} (기대 {})", item["expect"]));
            }
        }
    }
    assert!(skipped.is_empty(), "위험·확인 명령이 조용히 건너뛰어졌다 ({}건)\n{}", skipped.len(), skipped.join("\n"));
}

#[test]
fn the_default_prefilter_skips_read_only_commands_and_nothing_that_can_write_or_leak() {
    let skipped_expected = [
        // 기존
        "ls -la",
        "git status",
        "cat README.md",
        "git log --oneline -20",
        // 읽기 전용 도구
        "grep -rn TODO src",
        "rg --files",
        "jq . package.json",
        "tree -L 2",
        "file a.bin",
        "stat README.md",
        "du -sh .",
        "df -h",
        "ps aux",
        "whoami",
        "date",
        "uname -a",
        "echo hello",
        "docker ps -a",
        "docker images",
        "kubectl get pods -n staging",
        "kubectl get nodes",
        "kubectl get services",
        "kubectl get deployments",
        "kubectl get namespaces",
        "git rev-parse HEAD",
        "git ls-files",
        "git blame src/main.rs",
        "git shortlog -sn",
        "git describe --tags",
        "git stash list",
        "git remote -v",
    ];
    let not_skipped_expected = [
        // 셸 메타문자가 있으면 건너뛰지 않는다(쓰기·파이프·치환)
        "grep x file > out",
        "echo hi > f",
        "ps aux | grep x",
        "cat a; rm b",
        "grep $(whoami) x",
        "echo $HOME",
        // 쓰기·삭제·실행·전송이 가능한 명령은 목록에 없다
        "find . -delete",
        "find . -name x",
        "sort -o out in",
        "sed -i s/a/b/ f",
        "awk 1 f",
        "env",
        "docker rm x",
        "docker run alpine",
        "docker logs x",
        "docker psx",
        "kubectl get",
        "kubectl delete pod x",
        "kubectl get secret x -o yaml",
        "git remote add x y",
        "git tag v1",
        "git config user.name x",
        "npm install",
        // 읽기 명령이라도 비밀 파일이면 정적 규칙이 먼저 잡는다
        "grep SECRET .env",
        "rg KEY .env",
        "jq . .env",
        "cat .env",
        "grep password ~/.aws/credentials",
        "cat ~/.ssh/id_rsa",
        "git reset --hard",
    ];
    let wrongly_not_skipped: Vec<&str> = skipped_expected.iter().copied().filter(|c| !silently_skipped(c)).collect();
    let wrongly_skipped: Vec<&str> = not_skipped_expected.iter().copied().filter(|c| silently_skipped(c)).collect();
    assert!(
        wrongly_not_skipped.is_empty() && wrongly_skipped.is_empty(),
        "건너뛰어야 하는데 안 건너뜀: {wrongly_not_skipped:?}\n건너뛰면 안 되는데 건너뜀: {wrongly_skipped:?}"
    );
}
