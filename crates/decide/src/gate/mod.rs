// Claude Code 훅에서 decide로 판정하는 게이트(설정, 판정 로직, 출력, 클라이언트)와 그 실행 흐름
pub mod bash_risk;
pub mod client;
pub mod config;
pub mod output;
pub mod rules;
pub mod stats;

use output::{Kind, Outcome};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::time::Duration;

/// 한 번의 훅 실행에 필요한 환경. 테스트가 홈, 소켓, 버전을 바꿔 끼울 수 있게 밖으로 뺀다.
#[derive(Debug, Clone)]
pub struct Context {
    pub home: Option<String>,
    pub socket: PathBuf,
    pub client_version: String,
    /// 훅 입력에 `cwd`가 없을 때 저장소 설정을 찾을 디렉터리.
    pub fallback_cwd: PathBuf,
}

/// 훅 입력(stdin 텍스트)을 받아 훅 출력 JSON을 돌려준다. 내보낼 것이 없거나 처리할 수 없는 입력이면
/// `None`이다 — 어떤 실패도 Claude Code의 기본 권한 흐름을 막지 않는다. 데몬이 필요하면 `spawn`을 부른다.
pub fn run_hook(name: &str, input: &str, ctx: &Context, spawn: &mut dyn FnMut()) -> Option<Value> {
    if name != config::BASH_RISK {
        return None;
    }
    let value: Value = serde_json::from_str(input).ok()?;
    if let Some(event) = value.get("hook_event_name").and_then(Value::as_str) {
        if event != "PreToolUse" {
            return None;
        }
    }
    let (command, hook_cwd) = bash_risk::from_hook(&value)?;
    let cwd = if hook_cwd.is_empty() { ctx.fallback_cwd.clone() } else { PathBuf::from(&hook_cwd) };
    let loaded = config::load_from_disk(ctx.home.as_deref(), &cwd);
    let settings = &loaded.config;
    if !settings.bash_risk.enabled {
        return None;
    }

    let rule = rules::judge(&command, &settings.bash_risk.deny_patterns, &settings.bash_risk.ask_patterns);
    let kind = if let Some((verdict, pattern)) = rule {
        // 정적 규칙은 사전 필터보다 먼저 보고, 걸리면 모델을 부르지 않는다.
        Kind::Rule { verdict, pattern: pattern.to_string() }
    } else if bash_risk::prefiltered(&command, &settings.bash_risk.prefilter) {
        Kind::Prefiltered
    } else {
        let request = bash_risk::request(&command, cwd.to_str().unwrap_or(""), &ctx.client_version);
        let timeout = Duration::from_millis(settings.timeout_ms);
        match client::ask_or_start(&ctx.socket, &request, timeout, spawn) {
            Ok(result) => match bash_risk::probs_from(&result) {
                Some(probs) => Kind::Judged {
                    verdict: bash_risk::judge(probs, &settings.bash_risk),
                    probs,
                    result,
                },
                None => Kind::Failed { reason: "데몬 답에서 선택지 확률을 읽지 못했습니다".to_string() },
            },
            Err(reason) => Kind::Failed { reason },
        }
    };
    let outcome = Outcome {
        gate: name.to_string(),
        mode: settings.mode,
        display: settings.display,
        command: bash_risk::redact(&command),
        kind,
        warnings: loaded.warnings.len(),
    };
    if let Some(path) = client::audit_path(ctx.home.as_deref()) {
        // 감사 로그를 못 써도 훅 판정은 계속한다.
        let _ = client::append_audit(&path, &audit_record(&outcome, &cwd));
    }
    output::hook_output(&outcome)
}

fn verdict_name(verdict: bash_risk::Verdict) -> &'static str {
    match verdict {
        bash_risk::Verdict::Allow => "allow",
        bash_risk::Verdict::Ask => "ask",
        bash_risk::Verdict::Deny => "deny",
    }
}

/// 감사 로그 한 줄. 명령은 이미 가려진 값이고 200자에서 자른다.
fn audit_record(outcome: &Outcome, cwd: &Path) -> Value {
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs())
        .unwrap_or(0);
    let mode = match outcome.mode {
        config::Mode::Audit => "audit",
        config::Mode::Enforce => "enforce",
    };
    let mut record = json!({
        "ts": ts,
        "gate": outcome.gate,
        "mode": mode,
        "verdict": null,
        "probs": null,
        "backend": null,
        "model": null,
        "latency_ms": null,
        "prefiltered": false,
        "rule": null,
        "failure": null,
        "command": bash_risk::clip(&outcome.command, 200),
        "cwd_tail": bash_risk::cwd_tail(cwd.to_str().unwrap_or("")),
    });
    match &outcome.kind {
        Kind::Prefiltered => record["prefiltered"] = json!(true),
        Kind::Rule { verdict, pattern } => {
            record["verdict"] = json!(verdict_name(*verdict));
            record["rule"] = json!(pattern);
        }
        Kind::Failed { reason } => record["failure"] = json!(reason),
        Kind::Judged { verdict, probs, result } => {
            record["verdict"] = json!(verdict_name(*verdict));
            record["probs"] = json!({"allow": probs.allow, "ask": probs.ask, "deny": probs.deny});
            record["backend"] = result["routing"]["backend"].clone();
            record["model"] = result["routing"]["model"].clone();
            record["latency_ms"] = result["latency_ms"].clone();
        }
    }
    record
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::io::{BufRead, BufReader, Write};
    use std::os::unix::net::UnixListener;

    /// 유닉스 소켓 경로 한계(약 104바이트) 때문에 이름을 짧게 한다. 레이블은 테스트마다 다르다.
    fn temp_dir(label: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("dgm-{label}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn ctx(dir: &Path) -> Context {
        Context {
            home: Some(dir.to_str().unwrap().to_string()),
            socket: dir.join("d.sock"),
            client_version: "9.9.9".into(),
            fallback_cwd: dir.to_path_buf(),
        }
    }

    fn hook_input(command: &str, cwd: &Path) -> String {
        json!({
            "hook_event_name": "PreToolUse",
            "tool_name": "Bash",
            "tool_input": {"command": command},
            "cwd": cwd.to_str().unwrap(),
        })
        .to_string()
    }

    const DENY_ANSWER: &str = r#"{"answer":{"type":"choice","choice":"deny","confidence":0.82,"probabilities":{"allow":0.03,"ask":0.15,"deny":0.82}},"routing":{"backend":"local","model":"clef-flash"},"latency_ms":540.0}"#;
    const ALLOW_ANSWER: &str = r#"{"answer":{"type":"choice","choice":"allow","confidence":0.95,"probabilities":{"allow":0.95,"ask":0.04,"deny":0.01}},"routing":{"backend":"local","model":"clef-flash"},"latency_ms":300.0}"#;

    /// 한 번만 받아 `reply`로 답하는 가짜 데몬. 받은 요청 줄을 돌려준다.
    fn fake_daemon(socket: &Path, reply: &'static str) -> std::thread::JoinHandle<String> {
        let listener = UnixListener::bind(socket).unwrap();
        std::thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            let mut line = String::new();
            BufReader::new(stream.try_clone().unwrap()).read_line(&mut line).unwrap();
            let mut writer = stream;
            writeln!(writer, "{reply}").unwrap();
            line
        })
    }

    fn audit_lines(dir: &Path) -> Vec<Value> {
        std::fs::read_to_string(dir.join(".cache/decide/gate.log"))
            .unwrap_or_default()
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect()
    }

    fn write_config(path: &Path, text: &str) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    }

    #[test]
    fn inputs_it_cannot_handle_produce_nothing_and_touch_nothing() {
        let dir = temp_dir("skip");
        let mut spawned = 0;
        let mut spawn = || spawned += 1;
        let c = ctx(&dir);
        // 알 수 없는 게이트, 깨진 JSON, Bash가 아닌 도구, 다른 훅 이벤트
        assert!(run_hook("other-gate", &hook_input("rm x", &dir), &c, &mut spawn).is_none());
        assert!(run_hook("bash-risk", "not json", &c, &mut spawn).is_none());
        let edit = json!({"hook_event_name": "PreToolUse", "tool_name": "Edit", "tool_input": {"command": "x"}});
        assert!(run_hook("bash-risk", &edit.to_string(), &c, &mut spawn).is_none());
        let post = json!({"hook_event_name": "PostToolUse", "tool_name": "Bash", "tool_input": {"command": "x"}});
        assert!(run_hook("bash-risk", &post.to_string(), &c, &mut spawn).is_none());
        assert_eq!(spawned, 0);
        assert!(audit_lines(&dir).is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_prefiltered_command_never_reaches_the_daemon_but_is_logged() {
        let dir = temp_dir("pre");
        let mut spawned = 0;
        let output = run_hook("bash-risk", &hook_input("git status --short", &dir), &ctx(&dir), &mut || spawned += 1);
        assert!(output.is_none(), "기본 표시에서는 사전 필터 통과를 보이지 않는다");
        assert_eq!(spawned, 0, "데몬을 띄우지도 않는다");
        let log = audit_lines(&dir);
        assert_eq!(log.len(), 1);
        assert_eq!(log[0]["prefiltered"], true);
        assert_eq!(log[0]["gate"], "bash-risk");
        assert_eq!(log[0]["verdict"], Value::Null);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn audit_mode_shows_the_reasoning_without_deciding_and_logs_the_verdict() {
        let dir = temp_dir("audit");
        let server = fake_daemon(&dir.join("d.sock"), DENY_ANSWER);
        let output = run_hook("bash-risk", &hook_input("rm -rf ~/Downloads/old", &dir), &ctx(&dir), &mut || {})
            .expect("deny 판정은 표시된다");
        let message = output["systemMessage"].as_str().unwrap();
        assert!(message.contains("deny (감사 모드 — 막지 않음)"), "{message}");
        assert!(message.contains("rm -rf ~/Downloads/old"), "{message}");
        assert!(output.get("hookSpecificOutput").is_none(), "감사 모드는 결정하지 않는다");
        let request: Value = serde_json::from_str(server.join().unwrap().trim()).unwrap();
        assert_eq!(request["client_version"], "9.9.9");
        assert_eq!(request["type"], "choice");
        let log = audit_lines(&dir);
        assert_eq!(log[0]["verdict"], "deny");
        assert_eq!(log[0]["mode"], "audit");
        assert_eq!(log[0]["backend"], "local");
        assert_eq!(log[0]["model"], "clef-flash");
        assert_eq!(log[0]["prefiltered"], false);
        assert_eq!(log[0]["probs"]["deny"], 0.82);
        assert!(log[0]["ts"].as_u64().unwrap() > 1_700_000_000);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn enforce_mode_from_the_user_config_makes_a_permission_decision() {
        let dir = temp_dir("enf");
        write_config(&dir.join(".config/decide/gates.json"), r#"{"mode": "enforce"}"#);
        let _server = fake_daemon(&dir.join("d.sock"), DENY_ANSWER);
        let output = run_hook("bash-risk", &hook_input("rm -rf ~/x", &dir), &ctx(&dir), &mut || {}).unwrap();
        assert_eq!(output["hookSpecificOutput"]["permissionDecision"], "deny");
        assert_eq!(audit_lines(&dir)[0]["mode"], "enforce");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_repo_config_is_read_from_the_hook_cwd_and_can_only_tighten() {
        let dir = temp_dir("repo");
        let project = dir.join("proj");
        write_config(&project.join(".decide/gates.json"), r#"{"mode": "enforce"}"#);
        let _server = fake_daemon(&dir.join("d.sock"), DENY_ANSWER);
        let output = run_hook("bash-risk", &hook_input("rm -rf ~/x", &project), &ctx(&dir), &mut || {}).unwrap();
        assert_eq!(output["hookSpecificOutput"]["permissionDecision"], "deny");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_disabled_gate_does_nothing() {
        let dir = temp_dir("off");
        write_config(
            &dir.join(".config/decide/gates.json"),
            r#"{"gates": {"bash-risk": {"enabled": false}}}"#,
        );
        let mut spawned = 0;
        assert!(run_hook("bash-risk", &hook_input("rm -rf ~/x", &dir), &ctx(&dir), &mut || spawned += 1).is_none());
        assert_eq!(spawned, 0);
        assert!(audit_lines(&dir).is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_judged_allow_is_silent_and_a_daemon_down_passes_through_with_one_line() {
        let dir = temp_dir("allow");
        let _server = fake_daemon(&dir.join("d.sock"), ALLOW_ANSWER);
        assert!(run_hook("bash-risk", &hook_input("make test", &dir), &ctx(&dir), &mut || {}).is_none());
        assert_eq!(audit_lines(&dir)[0]["verdict"], "allow");
        let _ = std::fs::remove_dir_all(&dir);

        let down = temp_dir("down");
        let mut spawned = 0;
        let output = run_hook("bash-risk", &hook_input("make deploy", &down), &ctx(&down), &mut || spawned += 1).unwrap();
        assert_eq!(spawned, 1, "꺼진 데몬은 새로 띄운다");
        let message = output["systemMessage"].as_str().unwrap();
        assert!(message.contains("판정 없이 통과") && message.contains("시작"), "{message}");
        assert!(output.get("hookSpecificOutput").is_none());
        let log = audit_lines(&down);
        assert!(log[0]["failure"].as_str().unwrap().contains("시작"));
        assert_eq!(log[0]["verdict"], Value::Null);
        let _ = std::fs::remove_dir_all(&down);
    }

    #[test]
    fn a_malformed_answer_passes_through_without_starting_a_daemon() {
        let dir = temp_dir("bad");
        let _server = fake_daemon(&dir.join("d.sock"), r#"{"answer":{"type":"choice","probabilities":{"allow":1}}}"#);
        let mut spawned = 0;
        let output = run_hook("bash-risk", &hook_input("make deploy", &dir), &ctx(&dir), &mut || spawned += 1).unwrap();
        assert!(output["systemMessage"].as_str().unwrap().contains("확률"), "{output}");
        assert_eq!(spawned, 0);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn secrets_are_redacted_before_the_daemon_and_in_the_audit_log() {
        let dir = temp_dir("sec");
        let server = fake_daemon(&dir.join("d.sock"), DENY_ANSWER);
        let command = "API_TOKEN=supersecretvalue deploy.sh";
        let _ = run_hook("bash-risk", &hook_input(command, &dir), &ctx(&dir), &mut || {});
        let sent = server.join().unwrap();
        assert!(!sent.contains("supersecretvalue"), "{sent}");
        assert!(sent.contains("API_TOKEN=***"), "{sent}");
        let log = std::fs::read_to_string(dir.join(".cache/decide/gate.log")).unwrap();
        assert!(!log.contains("supersecretvalue"), "{log}");
        assert!(log.contains("API_TOKEN=***"), "{log}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    const RULES: &str = r#"{"gates": {"bash-risk": {
        "deny_patterns": ["*mkfs*"], "ask_patterns": ["*clean -fd*", "*.env"]}}}"#;

    #[test]
    fn a_deny_rule_decides_without_the_daemon_and_is_logged_with_its_pattern() {
        let dir = temp_dir("rdeny");
        write_config(&dir.join(".config/decide/gates.json"), RULES);
        let mut spawned = 0;
        // 가짜 데몬이 없다. 규칙이 먼저 끝내므로 데몬에 연결하지도 띄우지도 않는다.
        let output = run_hook("bash-risk", &hook_input("sudo mkfs.ext4 /dev/sda", &dir), &ctx(&dir), &mut || spawned += 1)
            .expect("deny 규칙은 표시된다");
        assert_eq!(spawned, 0, "데몬을 띄우면 안 된다");
        let message = output["systemMessage"].as_str().unwrap();
        assert!(message.contains("deny (감사 모드 — 막지 않음)"), "{message}");
        assert!(message.contains("정적 규칙 `*mkfs*` (모델 호출 없음)"), "{message}");
        assert!(output.get("hookSpecificOutput").is_none(), "감사 모드는 결정하지 않는다");
        let log = audit_lines(&dir);
        assert_eq!(log.len(), 1);
        assert_eq!(log[0]["verdict"], "deny");
        assert_eq!(log[0]["rule"], "*mkfs*");
        assert_eq!(log[0]["backend"], Value::Null);
        assert_eq!(log[0]["latency_ms"], Value::Null);
        assert_eq!(log[0]["prefiltered"], false);
        assert_eq!(log[0]["failure"], Value::Null);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn an_ask_rule_becomes_a_permission_decision_in_enforce_mode() {
        let dir = temp_dir("rask");
        write_config(
            &dir.join(".config/decide/gates.json"),
            r#"{"mode": "enforce", "gates": {"bash-risk": {"ask_patterns": ["*clean -fd*"]}}}"#,
        );
        let output = run_hook("bash-risk", &hook_input("git clean -fdx", &dir), &ctx(&dir), &mut || {}).unwrap();
        assert_eq!(output["hookSpecificOutput"]["permissionDecision"], "ask");
        assert!(output["hookSpecificOutput"]["permissionDecisionReason"].as_str().unwrap().contains("정적 규칙"));
        assert_eq!(audit_lines(&dir)[0]["mode"], "enforce");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn rules_take_precedence_over_the_prefilter() {
        let dir = temp_dir("rpre");
        write_config(&dir.join(".config/decide/gates.json"), RULES);
        // `cat`은 사전 필터에 있어 규칙이 없으면 조용히 통과한다.
        let output = run_hook("bash-risk", &hook_input("cat ~/project/.env", &dir), &ctx(&dir), &mut || {})
            .expect("사전 필터가 아니라 ask 규칙이 먼저 적용된다");
        assert!(output["systemMessage"].as_str().unwrap().contains("ask (감사 모드"), "{output}");
        let log = audit_lines(&dir);
        assert_eq!(log[0]["prefiltered"], false);
        assert_eq!(log[0]["rule"], "*.env");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_deny_rule_wins_over_an_ask_rule() {
        let dir = temp_dir("rboth");
        write_config(
            &dir.join(".config/decide/gates.json"),
            r#"{"gates": {"bash-risk": {"deny_patterns": ["*mkfs*"], "ask_patterns": ["*mkfs*"]}}}"#,
        );
        let output = run_hook("bash-risk", &hook_input("mkfs.ext4 /dev/sdb", &dir), &ctx(&dir), &mut || {}).unwrap();
        assert!(output["systemMessage"].as_str().unwrap().contains("deny (감사 모드"), "{output}");
        assert_eq!(audit_lines(&dir)[0]["verdict"], "deny");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_command_no_rule_matches_still_goes_to_the_daemon() {
        let dir = temp_dir("rmiss");
        write_config(&dir.join(".config/decide/gates.json"), RULES);
        let server = fake_daemon(&dir.join("d.sock"), DENY_ANSWER);
        let output = run_hook("bash-risk", &hook_input("rm -rf ~/x", &dir), &ctx(&dir), &mut || {}).unwrap();
        server.join().unwrap();
        assert!(output["systemMessage"].as_str().unwrap().contains("deny (감사 모드"), "{output}");
        let log = audit_lines(&dir);
        assert_eq!(log[0]["rule"], Value::Null, "규칙이 아니라 모델 판정이다");
        assert_eq!(log[0]["backend"], "local");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn config_warnings_reach_the_user_message() {
        let dir = temp_dir("warn");
        write_config(&dir.join(".config/decide/gates.json"), r#"{"mode": "sometimes"}"#);
        let _server = fake_daemon(&dir.join("d.sock"), DENY_ANSWER);
        let output = run_hook("bash-risk", &hook_input("rm -rf ~/x", &dir), &ctx(&dir), &mut || {}).unwrap();
        assert!(output["systemMessage"].as_str().unwrap().contains("설정 경고 1건"), "{output}");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
