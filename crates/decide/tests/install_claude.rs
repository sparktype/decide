// decide install --claude가 MCP 등록에 이어 표시 훅과 bash-risk 게이트 훅을 설정에 안전하게 넣는지 검사한다
use serde_json::{json, Value};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

const COMMAND: &str = "/opt/homebrew/bin/decide hook";
const GATE_COMMAND: &str = "/opt/homebrew/bin/decide gate bash-risk";

fn temp_dir(label: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "decide-install-claude-{label}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn fake_claude(dir: &Path, script: &str) -> PathBuf {
    let bin = dir.join("bin");
    std::fs::create_dir_all(&bin).unwrap();
    let claude = bin.join("claude");
    std::fs::write(&claude, script).unwrap();
    std::fs::set_permissions(&claude, std::fs::Permissions::from_mode(0o755)).unwrap();
    bin
}

fn run(args: &[&str], home: &Path, path: &Path) -> Output {
    Command::new(env!("CARGO_BIN_EXE_decide"))
        .args(args)
        .env("HOME", home)
        .env_remove("CLAUDE_CONFIG_DIR")
        .env("PATH", path)
        .output()
        .unwrap()
}

fn read_json(path: &Path) -> Value {
    serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
}

fn our_group() -> Value {
    json!({
        "matcher": "mcp__decide__decide",
        "hooks": [{"type": "command", "command": COMMAND, "timeout": 5}]
    })
}

fn gate_group() -> Value {
    json!({
        "matcher": "Bash",
        "hooks": [{"type": "command", "command": GATE_COMMAND, "timeout": 10}]
    })
}

#[test]
fn claude_flag_registers_the_mcp_server_then_adds_the_hooks() {
    let dir = temp_dir("fresh");
    let log = dir.join("claude-args.log");
    let bin = fake_claude(
        &dir,
        &format!("#!/bin/sh\necho \"$@\" >> '{}'\nexit 0\n", log.display()),
    );
    let output = run(&["install", "--claude"], &dir, &bin);
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    let args = std::fs::read_to_string(&log).unwrap();
    assert!(args.contains("mcp add -s user decide"), "{args}");
    let settings = dir.join(".claude").join("settings.json");
    assert_eq!(
        read_json(&settings),
        json!({"hooks": {"PostToolUse": [our_group()], "PreToolUse": [gate_group()]}})
    );
    assert!(String::from_utf8_lossy(&output.stdout).contains("훅을 등록했습니다"));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn running_it_twice_changes_nothing_the_second_time() {
    let dir = temp_dir("twice");
    let bin = fake_claude(&dir, "#!/bin/sh\nexit 0\n");
    assert!(run(&["install", "--claude"], &dir, &bin).status.success());
    let settings = dir.join(".claude").join("settings.json");
    let after_first = std::fs::read(&settings).unwrap();
    let second = run(&["install", "--claude"], &dir, &bin);
    assert!(second.status.success());
    assert_eq!(std::fs::read(&settings).unwrap(), after_first);
    assert!(String::from_utf8_lossy(&second.stdout).contains("이미 등록돼 있습니다"));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn existing_settings_are_preserved_and_backed_up() {
    let dir = temp_dir("existing");
    let bin = fake_claude(&dir, "#!/bin/sh\nexit 0\n");
    let config = dir.join(".claude");
    std::fs::create_dir_all(&config).unwrap();
    let original = "{\n  \"theme\": \"dark\",\n  \"hooks\": {\n    \"Stop\": [{\"hooks\": [{\"type\": \"command\", \"command\": \"echo stop\"}]}],\n    \"PostToolUse\": [{\"matcher\": \"Bash\", \"hooks\": [{\"type\": \"command\", \"command\": \"echo bash\"}]}]\n  }\n}\n";
    std::fs::write(config.join("settings.json"), original).unwrap();
    assert!(run(&["install", "--claude"], &dir, &bin).status.success());
    let settings = read_json(&config.join("settings.json"));
    assert_eq!(settings["theme"], "dark");
    assert_eq!(settings["hooks"]["Stop"][0]["hooks"][0]["command"], "echo stop");
    let groups = settings["hooks"]["PostToolUse"].as_array().unwrap();
    assert_eq!(groups.len(), 2);
    assert_eq!(groups[0]["matcher"], "Bash");
    assert_eq!(groups[1], our_group());
    assert_eq!(settings["hooks"]["PreToolUse"], json!([gate_group()]));
    assert_eq!(
        std::fs::read_to_string(config.join("settings.json.bak-decide")).unwrap(),
        original
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_display_hook_installed_by_an_older_version_gets_only_the_gate_added() {
    let dir = temp_dir("upgrade");
    let bin = fake_claude(&dir, "#!/bin/sh\nexit 0\n");
    let config = dir.join(".claude");
    std::fs::create_dir_all(&config).unwrap();
    let older = json!({"hooks": {"PostToolUse": [our_group()]}});
    std::fs::write(config.join("settings.json"), serde_json::to_string_pretty(&older).unwrap()).unwrap();
    let output = run(&["install", "--claude"], &dir, &bin);
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    assert_eq!(
        read_json(&config.join("settings.json")),
        json!({"hooks": {"PostToolUse": [our_group()], "PreToolUse": [gate_group()]}}),
        "표시 훅은 한 번만 있고 게이트가 추가돼야 한다"
    );
    assert!(String::from_utf8_lossy(&output.stdout).contains("훅을 등록했습니다"));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn invalid_json_fails_and_leaves_the_settings_untouched() {
    let dir = temp_dir("invalid");
    let bin = fake_claude(&dir, "#!/bin/sh\nexit 0\n");
    let config = dir.join(".claude");
    std::fs::create_dir_all(&config).unwrap();
    std::fs::write(config.join("settings.json"), "{ not json").unwrap();
    let output = run(&["install", "--claude"], &dir, &bin);
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("JSON"));
    assert_eq!(
        std::fs::read_to_string(config.join("settings.json")).unwrap(),
        "{ not json"
    );
    assert!(!config.join("settings.json.bak-decide").exists());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_failing_mcp_registration_leaves_the_settings_alone() {
    let dir = temp_dir("mcpfail");
    let bin = fake_claude(&dir, "#!/bin/sh\necho 'boom' >&2\nexit 1\n");
    let output = run(&["install", "--claude"], &dir, &bin);
    assert!(!output.status.success());
    assert!(!dir.join(".claude").join("settings.json").exists());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn without_the_claude_cli_it_fails_with_the_korean_message_and_no_settings() {
    let dir = temp_dir("nocli");
    let output = run(&["install", "--claude"], &dir, Path::new(""));
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("claude 명령을 찾을 수 없습니다"));
    assert!(!dir.join(".claude").join("settings.json").exists());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn bare_install_still_only_registers_the_mcp_server() {
    let dir = temp_dir("bare");
    let bin = fake_claude(&dir, "#!/bin/sh\nexit 0\n");
    assert!(run(&["install"], &dir, &bin).status.success());
    assert!(!dir.join(".claude").join("settings.json").exists());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn unknown_options_and_extra_arguments_exit_two() {
    let dir = temp_dir("args");
    let bin = fake_claude(&dir, "#!/bin/sh\nexit 0\n");
    assert_eq!(run(&["install", "--bogus"], &dir, &bin).status.code(), Some(2));
    assert_eq!(run(&["install", "--claude", "extra"], &dir, &bin).status.code(), Some(2));
    assert_eq!(run(&["install", "extra"], &dir, &bin).status.code(), Some(2));
    assert!(!dir.join(".claude").join("settings.json").exists());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn help_mentions_the_claude_option() {
    let help = Command::new(env!("CARGO_BIN_EXE_decide"))
        .arg("--help")
        .output()
        .unwrap();
    assert!(String::from_utf8_lossy(&help.stdout).contains("--claude"));
}
