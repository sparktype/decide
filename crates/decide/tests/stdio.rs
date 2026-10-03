use std::io::{BufRead, BufReader, Write};
use std::process::{Command, Stdio};

#[test]
fn mcp_stdio_lists_the_tool_and_reports_local_connection_failure() {
    // 가중치가 없는 빈 디렉터리를 `CLEF_WEIGHTS`로 지정해, 네트워크 다운로드
    // 없이 결정론적으로 "가중치를 찾을 수 없다" 오류를 받는다 — 로컬
    // 백엔드가 더 이상 HTTP(`jev-style serve`)를 호출하지 않고
    // `local::infer`를 직접 호출하므로, 실패 모드도 가중치 유무로 바뀌었다.
    let weights = std::env::temp_dir().join(format!(
        "decide-stdio-empty-weights-{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&weights).unwrap();
    let mut child = Command::new(env!("CARGO_BIN_EXE_decide"))
        .arg("mcp")
        .env("DECIDE_BACKEND", "local")
        .env("CLEF_WEIGHTS", &weights)
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
        .contains("찾을 수 없습니다"));

    let _ = child.kill();
    let _ = child.wait();
    let _ = std::fs::remove_dir_all(&weights);
}

fn read_line(stdout: &mut BufReader<impl std::io::Read>) -> String {
    let mut line = String::new();
    stdout.read_line(&mut line).unwrap();
    line
}
