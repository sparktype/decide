use std::os::unix::fs::PermissionsExt;
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
    assert!(text.contains("install"));

    let short = Command::new(env!("CARGO_BIN_EXE_decide"))
        .arg("-h")
        .output()
        .unwrap();
    assert!(short.status.success());

    let no_args = Command::new(env!("CARGO_BIN_EXE_decide"))
        .output()
        .unwrap();
    assert!(no_args.status.success());
    let no_args_text = String::from_utf8_lossy(&no_args.stdout);
    assert!(no_args_text.contains("mcp"));
    assert!(no_args_text.contains("daemon"));

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

#[test]
fn install_without_claude_cli_fails_with_korean_message() {
    let output = Command::new(env!("CARGO_BIN_EXE_decide"))
        .arg("install")
        .env("PATH", "")
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("claude 명령을 찾을 수 없습니다"));
}

#[test]
fn install_treats_already_registered_as_success() {
    let dir = std::env::temp_dir().join(format!(
        "decide-install-fake-claude-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let fake_claude = dir.join("claude");
    std::fs::write(
        &fake_claude,
        "#!/bin/sh\necho 'MCP server decide already exists in user config' >&2\nexit 1\n",
    )
    .unwrap();
    std::fs::set_permissions(&fake_claude, std::fs::Permissions::from_mode(0o755)).unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_decide"))
        .arg("install")
        .env("PATH", &dir)
        .output()
        .unwrap();

    let _ = std::fs::remove_dir_all(&dir);
    assert!(output.status.success());
}
