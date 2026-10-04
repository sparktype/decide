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

// ---- 도움말과 버전 출력 ----

use decide::help;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::{Duration, Instant};

/// 유닉스 소켓 경로 한계(약 104바이트) 때문에 짧은 이름을 쓴다. 레이블은 테스트마다 다르다.
fn isolated_home(label: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("dcl-{label}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

struct Ran {
    code: Option<i32>,
    stdout: String,
    stderr: String,
}

/// 격리된 HOME에서 실행하고 5초 안에 안 끝나면 죽인다 — 서버가 잘못 시작되면 영원히 안 끝나기 때문이다.
fn run_cli(args: &[&str], home: &Path, path: Option<&str>) -> Ran {
    let mut command = Command::new(env!("CARGO_BIN_EXE_decide"));
    command
        .args(args)
        .env("HOME", home)
        .env_remove("CLAUDE_CONFIG_DIR")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if let Some(path) = path {
        command.env("PATH", path);
    }
    let mut child = command.spawn().unwrap();
    let started = Instant::now();
    let status = loop {
        if let Some(status) = child.try_wait().unwrap() {
            break Some(status);
        }
        if started.elapsed() > Duration::from_secs(5) {
            let _ = child.kill();
            let _ = child.wait();
            break None;
        }
        std::thread::sleep(Duration::from_millis(20));
    };
    let mut stdout = String::new();
    let mut stderr = String::new();
    child.stdout.take().unwrap().read_to_string(&mut stdout).unwrap();
    child.stderr.take().unwrap().read_to_string(&mut stderr).unwrap();
    assert!(status.is_some(), "5초 안에 끝나야 한다(서버가 시작됐을 수 있다): {args:?}");
    Ran { code: status.unwrap().code(), stdout, stderr }
}

#[test]
fn version_flags_print_the_name_and_version_on_stdout() {
    let home = isolated_home("ver");
    let expected = format!("decide {}\n", env!("CARGO_PKG_VERSION"));
    for flag in ["--version", "-V"] {
        let ran = run_cli(&[flag], &home, None);
        assert_eq!(ran.code, Some(0), "{flag}");
        assert_eq!(ran.stdout, expected, "{flag}");
        assert_eq!(ran.stderr, "", "{flag}");
    }
    let extra = run_cli(&["--version", "extra"], &home, None);
    assert_eq!(extra.code, Some(2));
    assert!(extra.stderr.contains("인자가 너무 많습니다"), "{}", extra.stderr);
    assert_eq!(extra.stdout, "");
    let _ = std::fs::remove_dir_all(&home);
}

#[test]
fn the_overview_is_the_same_for_no_arguments_help_flags_and_the_help_command() {
    let home = isolated_home("overview");
    let expected = format!("{}\n", help::overview());
    for args in [&[][..], &["-h"], &["--help"], &["help"], &["--help", "extra"]] {
        let ran = run_cli(args, &home, None);
        assert_eq!(ran.code, Some(0), "{args:?}");
        assert_eq!(ran.stdout, expected, "{args:?}");
        assert_eq!(ran.stderr, "", "{args:?}");
    }
    assert!(expected.starts_with(&format!("decide {}", env!("CARGO_PKG_VERSION"))));
    let _ = std::fs::remove_dir_all(&home);
}

#[test]
fn every_command_answers_help_flags_without_starting_anything() {
    let home = isolated_home("percmd");
    for (name, _) in help::COMMANDS {
        let expected = format!("{}\n", help::for_command(name).unwrap());
        for args in [vec![name, "--help"], vec![name, "-h"], vec!["help", name]] {
            let ran = run_cli(&args, &home, Some(""));
            assert_eq!(ran.code, Some(0), "{args:?}: {}", ran.stderr);
            assert_eq!(ran.stdout, expected, "{args:?}");
            assert_eq!(ran.stderr, "", "{args:?}");
        }
    }
    assert!(!home.join(".cache").exists(), "도움말이 데몬이나 로그를 만들면 안 된다");
    let _ = std::fs::remove_dir_all(&home);
}

#[test]
fn a_help_flag_wins_over_other_arguments_of_the_command() {
    let home = isolated_home("wins");
    let expected = format!("{}\n", help::for_command("gate").unwrap());
    for args in [&["gate", "--show", "--help"][..], &["gate", "bash-risk", "-h"]] {
        let ran = run_cli(args, &home, None);
        assert_eq!(ran.code, Some(0), "{args:?}");
        assert_eq!(ran.stdout, expected, "{args:?}");
    }
    let _ = std::fs::remove_dir_all(&home);
}

#[test]
fn unknown_commands_exit_two_with_a_hint_to_the_help() {
    let home = isolated_home("unknown");
    for args in [&["nope"][..], &["help", "nope"]] {
        let ran = run_cli(args, &home, None);
        assert_eq!(ran.code, Some(2), "{args:?}");
        assert_eq!(ran.stdout, "", "{args:?}");
        assert!(ran.stderr.contains("알 수 없는 명령입니다: nope"), "{args:?}: {}", ran.stderr);
        assert!(ran.stderr.contains("decide --help"), "{args:?}: {}", ran.stderr);
    }
    let bad_option = run_cli(&["--bogus"], &home, None);
    assert_eq!(bad_option.code, Some(2));
    assert!(bad_option.stderr.contains("decide --help"), "{}", bad_option.stderr);
    let _ = std::fs::remove_dir_all(&home);
}
