// decide hook 서브커맨드가 훅 입력 JSON을 systemMessage 한 줄로 바꾸는지 검사한다
use serde_json::{json, Value};
use std::io::Write;
use std::process::{Command, Stdio};

const NOUL_INPUT: &str = r#"{"hook_event_name":"PostToolUse","tool_name":"mcp__decide__decide","tool_input":{"state":"s","type":"noul","instructions":"이 변경은 머지해도 될 만큼 검증되었는가?"},"tool_response":"{\"answer\":{\"type\":\"noul\",\"noul\":0.56},\"routing\":{\"backend\":\"typesafe\",\"model\":\"jev-1.13.0\"},\"latency_ms\":460.36216700000006}"}"#;

fn run_hook(stdin: &[u8]) -> (Option<i32>, String) {
    let mut child = Command::new(env!("CARGO_BIN_EXE_decide"))
        .arg("hook")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child.stdin.take().unwrap().write_all(stdin).unwrap();
    let output = child.wait_with_output().unwrap();
    (
        output.status.code(),
        String::from_utf8(output.stdout).unwrap(),
    )
}

#[test]
fn hook_prints_exactly_one_json_line_with_a_multiline_message() {
    let (code, stdout) = run_hook(NOUL_INPUT.as_bytes());
    assert_eq!(code, Some(0));
    assert!(stdout.ends_with('\n'));
    assert_eq!(stdout.trim_end().lines().count(), 1, "{stdout}");
    let parsed: Value = serde_json::from_str(stdout.trim_end()).unwrap();
    assert_eq!(
        parsed,
        json!({"systemMessage": "🔎 decide 판단: 이 변경은 머지해도 될 만큼 검증되었는가? → 56%\n   typesafe · jev-1.13.0 · 460ms"})
    );
}

#[test]
fn hook_is_silent_and_succeeds_on_unreadable_input() {
    for input in [
        &b""[..],
        &b"not json"[..],
        &b"{\"tool_name\":\"mcp__other__tool\"}"[..],
        &[0xff, 0xfe, 0xfd][..],
    ] {
        let (code, stdout) = run_hook(input);
        assert_eq!(code, Some(0), "입력: {input:?}");
        assert_eq!(stdout, "", "입력: {input:?}");
    }
}

#[test]
fn help_lists_hook_and_extra_arguments_exit_two() {
    let help = Command::new(env!("CARGO_BIN_EXE_decide"))
        .arg("--help")
        .output()
        .unwrap();
    assert!(String::from_utf8_lossy(&help.stdout).contains("hook"));
    let extra = Command::new(env!("CARGO_BIN_EXE_decide"))
        .args(["hook", "extra"])
        .output()
        .unwrap();
    assert_eq!(extra.status.code(), Some(2));
}
