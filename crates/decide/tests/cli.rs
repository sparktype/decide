use std::process::Command;

#[test]
fn help_exits_zero_and_unknown_arguments_exit_two() {
    let help = Command::new(env!("CARGO_BIN_EXE_decide"))
        .arg("--help")
        .output()
        .unwrap();
    assert!(help.status.success());
    let text = String::from_utf8_lossy(&help.stdout);
    assert!(text.contains("mcp"));
    assert!(text.contains("daemon"));

    let short = Command::new(env!("CARGO_BIN_EXE_decide"))
        .arg("-h")
        .output()
        .unwrap();
    assert!(short.status.success());

    let unknown = Command::new(env!("CARGO_BIN_EXE_decide"))
        .arg("nope")
        .output()
        .unwrap();
    assert_eq!(unknown.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&unknown.stderr).contains("알 수 없는 명령"));

    let extra = Command::new(env!("CARGO_BIN_EXE_decide"))
        .args(["mcp", "extra"])
        .output()
        .unwrap();
    assert_eq!(extra.status.code(), Some(2));
}
