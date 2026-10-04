// decide gate 서브커맨드가 훅 입력을 받아 판정을 내고 --show로 설정을 보여 주는지 바이너리로 검사한다
use serde_json::{json, Value};
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixListener;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

const DENY_ANSWER: &str = r#"{"answer":{"type":"choice","choice":"deny","confidence":0.62,"probabilities":{"allow":0.08,"ask":0.3,"deny":0.62}},"routing":{"backend":"local","model":"clef-flash"},"latency_ms":540.0}"#;

/// 유닉스 소켓 경로 한계(약 104바이트) 때문에 HOME 이름을 짧게 한다. 레이블은 테스트마다 다르다.
fn home(label: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("dgi-{label}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn run(args: &[&str], home: &Path, stdin: &str) -> Output {
    let mut child = Command::new(env!("CARGO_BIN_EXE_decide"))
        .args(args)
        .env("HOME", home)
        .env_remove("CLAUDE_CONFIG_DIR")
        .current_dir(home)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child.stdin.take().unwrap().write_all(stdin.as_bytes()).unwrap();
    child.wait_with_output().unwrap()
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

/// `$HOME/.cache/decide/decide.sock`에서 한 번만 받아 `reply`로 답하는 가짜 데몬.
fn fake_daemon(home: &Path, reply: &'static str) -> std::thread::JoinHandle<()> {
    let socket = home.join(".cache/decide/decide.sock");
    std::fs::create_dir_all(socket.parent().unwrap()).unwrap();
    let listener = UnixListener::bind(&socket).unwrap();
    listener.set_nonblocking(true).unwrap();
    std::thread::spawn(move || {
        // 연결이 오지 않으면 영원히 기다리지 않고 실패한다.
        let started = std::time::Instant::now();
        let stream = loop {
            match listener.accept() {
                Ok((stream, _)) => break stream,
                Err(_) if started.elapsed() < std::time::Duration::from_secs(5) => {
                    std::thread::sleep(std::time::Duration::from_millis(10));
                }
                Err(err) => panic!("5초 안에 게이트가 데몬에 연결하지 않았다: {err}"),
            }
        };
        stream.set_nonblocking(false).unwrap();
        let mut line = String::new();
        BufReader::new(stream.try_clone().unwrap()).read_line(&mut line).unwrap();
        let mut writer = stream;
        writeln!(writer, "{reply}").unwrap();
    })
}

fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).to_string()
}

#[test]
fn a_hook_input_gets_a_judgement_and_an_audit_line() {
    let home = home("hook");
    let server = fake_daemon(&home, DENY_ANSWER);
    let output = run(&["gate", "bash-risk"], &home, &hook_input("rm -rf ~/Downloads/old", &home));
    server.join().unwrap();
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    let printed: Value = serde_json::from_str(stdout(&output).trim()).unwrap();
    let message = printed["systemMessage"].as_str().unwrap();
    assert!(message.contains("deny (감사 모드 — 막지 않음)"), "{message}");
    assert!(printed.get("hookSpecificOutput").is_none());
    let log = std::fs::read_to_string(home.join(".cache/decide/gate.log")).unwrap();
    assert!(log.contains("\"verdict\":\"deny\""), "{log}");
    let _ = std::fs::remove_dir_all(&home);
}

#[test]
fn a_prefiltered_command_prints_nothing_and_needs_no_daemon() {
    let home = home("pre");
    let output = run(&["gate", "bash-risk"], &home, &hook_input("git status", &home));
    assert!(output.status.success());
    assert_eq!(stdout(&output), "");
    assert!(!home.join(".cache/decide/decide.sock").exists(), "데몬을 띄우면 안 된다");
    let _ = std::fs::remove_dir_all(&home);
}

#[test]
fn unusable_input_prints_nothing_and_exits_zero() {
    let home = home("junk");
    for stdin in ["", "not json", "{}", r#"{"tool_name":"Edit","tool_input":{}}"#] {
        let output = run(&["gate", "bash-risk"], &home, stdin);
        assert!(output.status.success(), "{stdin:?}");
        assert_eq!(stdout(&output), "", "{stdin:?}");
    }
    let _ = std::fs::remove_dir_all(&home);
}

#[test]
fn an_unknown_gate_name_is_an_error_not_silence() {
    let home = home("unk");
    let output = run(&["gate", "no-such-gate"], &home, &hook_input("ls", &home));
    assert_eq!(output.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&output.stderr).contains("알 수 없는 게이트"));
    assert_eq!(stdout(&output), "");
    let _ = std::fs::remove_dir_all(&home);
}

#[test]
fn gate_without_a_name_is_a_usage_error() {
    let home = home("noarg");
    let output = run(&["gate"], &home, "");
    assert_eq!(output.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&output.stderr).contains("gate"));
    let _ = std::fs::remove_dir_all(&home);
}

#[test]
fn show_prints_the_overview_the_detail_and_json() {
    let home = home("show");
    let overview = run(&["gate", "--show"], &home, "");
    assert!(overview.status.success());
    let text = stdout(&overview);
    assert!(text.starts_with("bash-risk   PreToolUse/Bash   choice   감사 모드   display=decisions"), "{text}");
    assert!(text.contains("user-rules  ~/.config/decide/gates.json (없음)"), "{text}");
    assert!(text.contains("repo-rules  ./.decide/gates.json (없음)"), "{text}");

    let detail = run(&["gate", "--show", "bash-risk"], &home, "");
    assert!(stdout(&detail).contains("이벤트:   PreToolUse (matcher: Bash)"));

    let as_json = run(&["gate", "--show", "bash-risk", "--json"], &home, "");
    let value: Value = serde_json::from_str(&stdout(&as_json)).unwrap();
    assert_eq!(value["gate"], "bash-risk");
    assert_eq!(value["config"]["mode"], "audit");

    // 설정 파일이 있으면 '있음'과 출처가 바뀐다.
    std::fs::create_dir_all(home.join(".config/decide")).unwrap();
    std::fs::write(home.join(".config/decide/gates.json"), r#"{"mode": "enforce"}"#).unwrap();
    let after = stdout(&run(&["gate", "--show"], &home, ""));
    assert!(after.contains("(있음)") && after.contains("enforce"), "{after}");
    let _ = std::fs::remove_dir_all(&home);
}

#[test]
fn show_rejects_an_unknown_gate_and_a_json_flag_without_a_name() {
    let home = home("showbad");
    let unknown = run(&["gate", "--show", "nope"], &home, "");
    assert_eq!(unknown.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&unknown.stderr).contains("알 수 없는 게이트"));
    let json_only = run(&["gate", "--show", "--json"], &home, "");
    assert_eq!(json_only.status.code(), Some(2));
    let _ = std::fs::remove_dir_all(&home);
}

#[test]
fn the_help_text_mentions_gate() {
    let home = home("help");
    let output = run(&["--help"], &home, "");
    assert!(stdout(&output).contains("gate"), "{}", stdout(&output));
    let _ = std::fs::remove_dir_all(&home);
}
