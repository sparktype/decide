use std::io::{BufRead, BufReader, Write};
use std::process::{Command, Stdio};

#[test]
fn mcp_stdio_lists_the_tool_and_reports_local_not_ready() {
    let mut child = Command::new(env!("CARGO_BIN_EXE_decide"))
        .arg("mcp")
        .env("DECIDE_BACKEND", "local")
        .env_remove("TYPESAFE_API_KEY")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut stdin = child.stdin.take().unwrap();
    let mut stdout = BufReader::new(child.stdout.take().unwrap());

    writeln!(
        stdin,
        r#"{{"jsonrpc":"2.0","id":1,"method":"initialize","params":{{"protocolVersion":"2025-03-26","capabilities":{{}},"clientInfo":{{"name":"t","version":"0"}}}}}}"#
    )
    .unwrap();
    stdin.flush().unwrap();
    let init = read_line(&mut stdout);
    let init: serde_json::Value = serde_json::from_str(&init).unwrap();
    assert_eq!(init["result"]["serverInfo"]["name"], "decide");

    writeln!(
        stdin,
        r#"{{"jsonrpc":"2.0","method":"notifications/initialized"}}"#
    )
    .unwrap();
    writeln!(
        stdin,
        r#"{{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{{"name":"decide","arguments":{{"state":"다운","type":"noul","instructions":"긴급한가?"}}}}}}"#
    )
    .unwrap();
    stdin.flush().unwrap();
    let call = read_line(&mut stdout);
    let call: serde_json::Value = serde_json::from_str(&call).unwrap();
    assert_eq!(call["result"]["isError"], true);
    assert!(call["result"]["content"][0]["text"]
        .as_str()
        .unwrap()
        .contains("로컬"));

    let _ = child.kill();
    let _ = child.wait();
}

fn read_line(stdout: &mut BufReader<impl std::io::Read>) -> String {
    let mut line = String::new();
    stdout.read_line(&mut line).unwrap();
    line
}
