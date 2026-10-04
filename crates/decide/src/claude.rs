// Claude Code 사용자 설정(settings.json)에 decide 표시 훅을 안전하게 병합해 넣는다
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

pub const HOOK_MATCHER: &str = "mcp__decide__decide";
const HOOK_TIMEOUT_SECS: u64 = 5;
const BACKUP_SUFFIX: &str = ".bak-decide";
const TEMP_SUFFIX: &str = ".decide-tmp";

#[derive(Debug, PartialEq, Eq)]
pub enum Installed {
    Added,
    AlreadyPresent,
}

pub fn hook_command(bin: &str) -> String {
    format!("{bin} hook")
}

pub fn settings_path() -> Result<PathBuf, String> {
    let config_dir = std::env::var("CLAUDE_CONFIG_DIR").ok();
    let home = std::env::var("HOME").ok();
    let has_config_dir = config_dir.as_deref().is_some_and(|dir| !dir.is_empty());
    let has_home = home.as_deref().is_some_and(|dir| !dir.is_empty());
    if !has_config_dir && !has_home {
        return Err("HOME 또는 CLAUDE_CONFIG_DIR를 알 수 없어 설정 경로를 정할 수 없습니다".to_string());
    }
    Ok(settings_path_from(config_dir.as_deref(), home.as_deref()))
}

pub fn settings_path_from(config_dir: Option<&str>, home: Option<&str>) -> PathBuf {
    match config_dir.filter(|dir| !dir.is_empty()) {
        Some(dir) => Path::new(dir).join("settings.json"),
        None => Path::new(home.filter(|dir| !dir.is_empty()).unwrap_or("."))
            .join(".claude")
            .join("settings.json"),
    }
}

/// 설정에 넣을 훅 하나. 같은 명령이 같은 이벤트에 이미 있으면 matcher와 상관없이 다시 넣지 않는다.
#[derive(Debug, Clone, Copy)]
pub struct HookSpec<'a> {
    pub event: &'a str,
    pub matcher: &'a str,
    pub command: &'a str,
    pub timeout: u64,
}

const GATE_MATCHER: &str = "Bash";
const GATE_TIMEOUT_SECS: u64 = 10;

pub fn gate_command(bin: &str) -> String {
    format!("{bin} gate bash-risk")
}

/// `decide install --claude`가 한 번에 넣을 훅: 결과 표시(PostToolUse)와 bash-risk 게이트(PreToolUse, Bash).
pub fn hook_specs<'a>(display_command: &'a str, gate_command: &'a str) -> [HookSpec<'a>; 2] {
    [
        display_spec(display_command),
        HookSpec {
            event: "PreToolUse",
            matcher: GATE_MATCHER,
            command: gate_command,
            timeout: GATE_TIMEOUT_SECS,
        },
    ]
}

fn display_spec(command: &str) -> HookSpec<'_> {
    HookSpec {
        event: "PostToolUse",
        matcher: HOOK_MATCHER,
        command,
        timeout: HOOK_TIMEOUT_SECS,
    }
}

pub fn add_hook(settings: &mut Value, command: &str) -> Result<bool, String> {
    add_hook_spec(settings, &display_spec(command))
}

pub fn add_hook_spec(settings: &mut Value, spec: &HookSpec) -> Result<bool, String> {
    let root = settings
        .as_object()
        .ok_or_else(|| "설정 파일이 JSON 객체가 아닙니다".to_string())?;
    match root.get("hooks") {
        None => {}
        Some(Value::Object(hooks)) => match hooks.get(spec.event) {
            None | Some(Value::Array(_)) => {}
            Some(_) => return Err(format!("설정의 hooks.{}가 배열이 아닙니다", spec.event)),
        },
        Some(_) => return Err("설정의 hooks가 객체가 아닙니다".to_string()),
    }
    if already_present(settings, spec.event, spec.command) {
        return Ok(false);
    }
    let group = json!({
        "matcher": spec.matcher,
        "hooks": [{"type": "command", "command": spec.command, "timeout": spec.timeout}]
    });
    if let Some(root) = settings.as_object_mut() {
        if let Value::Object(hooks) = root.entry("hooks").or_insert_with(|| json!({})) {
            if let Value::Array(groups) = hooks.entry(spec.event).or_insert_with(|| json!([])) {
                groups.push(group);
            }
        }
    }
    Ok(true)
}

fn already_present(settings: &Value, event: &str, command: &str) -> bool {
    settings
        .get("hooks")
        .and_then(|hooks| hooks.get(event))
        .and_then(Value::as_array)
        .is_some_and(|groups| {
            groups
                .iter()
                .filter_map(|group| group.get("hooks")?.as_array())
                .flatten()
                .any(|hook| hook.get("command").and_then(Value::as_str) == Some(command))
        })
}

pub fn install_hook(path: &Path, command: &str) -> Result<Installed, String> {
    install_hooks(path, &[display_spec(command)])
}

/// 훅 여러 개를 한 번에 병합한다. 파일은 한 번만 읽고 쓰며 백업은 처음 원본이다.
pub fn install_hooks(path: &Path, specs: &[HookSpec]) -> Result<Installed, String> {
    let original = match std::fs::read_to_string(path) {
        Ok(text) => Some(text),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => None,
        Err(err) => {
            return Err(format!("설정 파일을 읽지 못했습니다 ({}): {err}", path.display()));
        }
    };
    let mut settings: Value = match &original {
        Some(text) => serde_json::from_str(text).map_err(|err| {
            format!("설정 파일을 JSON으로 읽지 못했습니다 ({}): {err}", path.display())
        })?,
        None => json!({}),
    };
    let mut added = false;
    for spec in specs {
        added |= add_hook_spec(&mut settings, spec)?;
    }
    if !added {
        return Ok(Installed::AlreadyPresent);
    }
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| format!("설정 경로가 올바르지 않습니다: {}", path.display()))?;
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)
            .map_err(|err| format!("설정 디렉터리를 만들지 못했습니다 ({}): {err}", dir.display()))?;
    }
    if let Some(text) = &original {
        let backup = path.with_file_name(format!("{file_name}{BACKUP_SUFFIX}"));
        std::fs::write(&backup, text)
            .map_err(|err| format!("백업을 쓰지 못했습니다 ({}): {err}", backup.display()))?;
    }
    let rendered = serde_json::to_string_pretty(&settings)
        .map_err(|err| format!("설정을 JSON으로 만들지 못했습니다: {err}"))?;
    let temp = path.with_file_name(format!("{file_name}{TEMP_SUFFIX}"));
    std::fs::write(&temp, format!("{rendered}\n"))
        .map_err(|err| format!("임시 파일을 쓰지 못했습니다 ({}): {err}", temp.display()))?;
    if let Ok(meta) = std::fs::metadata(path) {
        let _ = std::fs::set_permissions(&temp, meta.permissions());
    }
    std::fs::rename(&temp, path)
        .map_err(|err| format!("설정 파일을 교체하지 못했습니다 ({}): {err}", path.display()))?;
    Ok(Installed::Added)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const COMMAND: &str = "/opt/homebrew/bin/decide hook";

    fn our_group() -> Value {
        json!({
            "matcher": HOOK_MATCHER,
            "hooks": [{"type": "command", "command": COMMAND, "timeout": 5}]
        })
    }

    fn temp_dir() -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "decide-claude-settings-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn hook_command_appends_the_subcommand_to_the_binary() {
        assert_eq!(
            hook_command("/opt/homebrew/bin/decide"),
            "/opt/homebrew/bin/decide hook"
        );
    }

    #[test]
    fn gate_command_and_the_install_specs_pair_the_display_hook_with_the_bash_gate() {
        assert_eq!(gate_command("/opt/homebrew/bin/decide"), "/opt/homebrew/bin/decide gate bash-risk");
        let display = hook_command("/b/decide");
        let gate = gate_command("/b/decide");
        let [first, second] = hook_specs(&display, &gate);
        assert_eq!((first.event, first.matcher), ("PostToolUse", HOOK_MATCHER));
        assert_eq!((second.event, second.matcher, second.timeout), ("PreToolUse", "Bash", 10));
        assert_eq!(second.command, "/b/decide gate bash-risk");
    }

    #[test]
    fn adds_the_group_to_an_empty_settings_object() {
        let mut settings = json!({});
        assert_eq!(add_hook(&mut settings, COMMAND), Ok(true));
        assert_eq!(settings, json!({"hooks": {"PostToolUse": [our_group()]}}));
    }

    #[test]
    fn keeps_everything_else_and_appends_after_existing_groups() {
        let mut settings = json!({
            "permissions": {"allow": ["Bash(ls)"]},
            "hooks": {
                "Stop": [{"hooks": [{"type": "command", "command": "echo stop"}]}],
                "PostToolUse": [{"matcher": "Bash", "hooks": [{"type": "command", "command": "echo bash"}]}]
            },
            "theme": "dark"
        });
        let before = settings.clone();
        assert_eq!(add_hook(&mut settings, COMMAND), Ok(true));

        let keys: Vec<&String> = settings.as_object().unwrap().keys().collect();
        assert_eq!(keys, vec!["permissions", "hooks", "theme"]);
        assert_eq!(settings["permissions"], before["permissions"]);
        assert_eq!(settings["theme"], before["theme"]);
        assert_eq!(settings["hooks"]["Stop"], before["hooks"]["Stop"]);
        let groups = settings["hooks"]["PostToolUse"].as_array().unwrap();
        assert_eq!(groups.len(), 2);
        assert_eq!(groups[0], before["hooks"]["PostToolUse"][0]);
        assert_eq!(groups[1], our_group());
    }

    #[test]
    fn is_idempotent() {
        let mut settings = json!({});
        assert_eq!(add_hook(&mut settings, COMMAND), Ok(true));
        let after_first = settings.clone();
        assert_eq!(add_hook(&mut settings, COMMAND), Ok(false));
        assert_eq!(settings, after_first);
    }

    #[test]
    fn recognizes_the_same_command_under_any_matcher() {
        let mut settings = json!({"hooks": {"PostToolUse": [
            {"matcher": "*", "hooks": [{"type": "command", "command": COMMAND}]}
        ]}});
        let before = settings.clone();
        assert_eq!(add_hook(&mut settings, COMMAND), Ok(false));
        assert_eq!(settings, before);
    }

    #[test]
    fn a_different_command_that_merely_contains_the_text_does_not_count() {
        let mut settings = json!({"hooks": {"PostToolUse": [
            {"matcher": "Bash", "hooks": [{"type": "command", "command": "echo /opt/homebrew/bin/decide hook"}]}
        ]}});
        assert_eq!(add_hook(&mut settings, COMMAND), Ok(true));
        assert_eq!(settings["hooks"]["PostToolUse"].as_array().unwrap().len(), 2);
    }

    #[test]
    fn tolerates_groups_without_a_hooks_array() {
        let mut settings = json!({"hooks": {"PostToolUse": [{"matcher": "x"}, "junk", 7]}});
        assert_eq!(add_hook(&mut settings, COMMAND), Ok(true));
        assert_eq!(settings["hooks"]["PostToolUse"].as_array().unwrap().len(), 4);
    }

    #[test]
    fn rejects_shapes_it_cannot_merge_into_without_touching_them() {
        for bad in [
            json!([1, 2]),
            json!("text"),
            json!({"hooks": "nope"}),
            json!({"hooks": {"PostToolUse": {"a": 1}}}),
        ] {
            let mut settings = bad.clone();
            let err = add_hook(&mut settings, COMMAND).unwrap_err();
            assert!(!err.is_empty());
            assert_eq!(settings, bad, "입력이 바뀌면 안 된다: {bad}");
        }
    }

    #[test]
    fn install_creates_a_missing_file_and_parent_directory() {
        let dir = temp_dir();
        let path = dir.join(".claude").join("settings.json");
        assert_eq!(install_hook(&path, COMMAND), Ok(Installed::Added));
        let written: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(written, json!({"hooks": {"PostToolUse": [our_group()]}}));
        assert!(!dir.join(".claude").join("settings.json.bak-decide").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn install_backs_up_the_original_and_preserves_other_settings() {
        let dir = temp_dir();
        let path = dir.join("settings.json");
        let original = "{\n  \"theme\": \"dark\",\n  \"hooks\": {\"Stop\": []}\n}\n";
        std::fs::write(&path, original).unwrap();
        assert_eq!(install_hook(&path, COMMAND), Ok(Installed::Added));
        assert_eq!(
            std::fs::read_to_string(dir.join("settings.json.bak-decide")).unwrap(),
            original
        );
        let written: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(written["theme"], "dark");
        assert_eq!(written["hooks"]["Stop"], json!([]));
        assert_eq!(written["hooks"]["PostToolUse"][0], our_group());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn install_leaves_the_file_untouched_when_it_is_not_valid_json() {
        let dir = temp_dir();
        let path = dir.join("settings.json");
        std::fs::write(&path, "{ not json").unwrap();
        let err = install_hook(&path, COMMAND).unwrap_err();
        assert!(err.contains("JSON"), "{err}");
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "{ not json");
        assert!(!dir.join("settings.json.bak-decide").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn install_twice_changes_nothing_the_second_time() {
        let dir = temp_dir();
        let path = dir.join("settings.json");
        std::fs::write(&path, "{}").unwrap();
        assert_eq!(install_hook(&path, COMMAND), Ok(Installed::Added));
        let after_first = std::fs::read(&path).unwrap();
        let backup_after_first = std::fs::read(dir.join("settings.json.bak-decide")).unwrap();
        assert_eq!(install_hook(&path, COMMAND), Ok(Installed::AlreadyPresent));
        assert_eq!(std::fs::read(&path).unwrap(), after_first);
        assert_eq!(
            std::fs::read(dir.join("settings.json.bak-decide")).unwrap(),
            backup_after_first
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    const GATE_COMMAND: &str = "/opt/homebrew/bin/decide gate bash-risk";

    fn gate_spec() -> HookSpec<'static> {
        HookSpec {
            event: "PreToolUse",
            matcher: "Bash",
            command: GATE_COMMAND,
            timeout: 10,
        }
    }

    fn gate_group() -> Value {
        json!({
            "matcher": "Bash",
            "hooks": [{"type": "command", "command": GATE_COMMAND, "timeout": 10}]
        })
    }

    #[test]
    fn a_spec_goes_under_its_own_event_and_matcher() {
        let mut settings = json!({});
        assert_eq!(add_hook_spec(&mut settings, &gate_spec()), Ok(true));
        assert_eq!(settings, json!({"hooks": {"PreToolUse": [gate_group()]}}));
    }

    #[test]
    fn the_same_command_under_another_event_does_not_count() {
        let mut settings = json!({"hooks": {"PostToolUse": [
            {"matcher": "Bash", "hooks": [{"type": "command", "command": GATE_COMMAND}]}
        ]}});
        assert_eq!(add_hook_spec(&mut settings, &gate_spec()), Ok(true));
        assert_eq!(settings["hooks"]["PreToolUse"], json!([gate_group()]));
        assert_eq!(settings["hooks"]["PostToolUse"].as_array().unwrap().len(), 1);
    }

    #[test]
    fn a_spec_is_idempotent_and_refuses_a_non_array_event() {
        let mut settings = json!({});
        assert_eq!(add_hook_spec(&mut settings, &gate_spec()), Ok(true));
        let after_first = settings.clone();
        assert_eq!(add_hook_spec(&mut settings, &gate_spec()), Ok(false));
        assert_eq!(settings, after_first);

        let bad = json!({"hooks": {"PreToolUse": {"a": 1}}});
        let mut settings = bad.clone();
        let err = add_hook_spec(&mut settings, &gate_spec()).unwrap_err();
        assert!(err.contains("PreToolUse"), "{err}");
        assert_eq!(settings, bad);
    }

    #[test]
    fn install_hooks_writes_every_spec_in_one_pass_with_one_backup() {
        let dir = temp_dir();
        let path = dir.join("settings.json");
        let original = "{\n  \"theme\": \"dark\"\n}\n";
        std::fs::write(&path, original).unwrap();
        let display = HookSpec {
            event: "PostToolUse",
            matcher: HOOK_MATCHER,
            command: COMMAND,
            timeout: HOOK_TIMEOUT_SECS,
        };
        assert_eq!(install_hooks(&path, &[display, gate_spec()]), Ok(Installed::Added));
        // 백업은 처음 원본이어야 한다(두 번째 스펙이 첫 스펙의 결과를 덮어쓰면 안 된다).
        assert_eq!(
            std::fs::read_to_string(dir.join("settings.json.bak-decide")).unwrap(),
            original
        );
        let written: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(written["theme"], "dark");
        assert_eq!(written["hooks"]["PostToolUse"], json!([our_group()]));
        assert_eq!(written["hooks"]["PreToolUse"], json!([gate_group()]));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn install_hooks_adds_only_the_missing_spec_and_is_idempotent() {
        let dir = temp_dir();
        let path = dir.join("settings.json");
        std::fs::write(&path, "{}").unwrap();
        assert_eq!(install_hook(&path, COMMAND), Ok(Installed::Added));
        let display = HookSpec {
            event: "PostToolUse",
            matcher: HOOK_MATCHER,
            command: COMMAND,
            timeout: HOOK_TIMEOUT_SECS,
        };
        assert_eq!(install_hooks(&path, &[display, gate_spec()]), Ok(Installed::Added));
        let written: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(written["hooks"]["PostToolUse"], json!([our_group()]));
        assert_eq!(written["hooks"]["PreToolUse"], json!([gate_group()]));
        let after = std::fs::read(&path).unwrap();
        let display = HookSpec {
            event: "PostToolUse",
            matcher: HOOK_MATCHER,
            command: COMMAND,
            timeout: HOOK_TIMEOUT_SECS,
        };
        assert_eq!(
            install_hooks(&path, &[display, gate_spec()]),
            Ok(Installed::AlreadyPresent)
        );
        assert_eq!(std::fs::read(&path).unwrap(), after);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn settings_path_prefers_the_config_dir_override() {
        assert_eq!(
            settings_path_from(Some("/tmp/cfg"), Some("/home/u")),
            PathBuf::from("/tmp/cfg/settings.json")
        );
        assert_eq!(
            settings_path_from(None, Some("/home/u")),
            PathBuf::from("/home/u/.claude/settings.json")
        );
        assert_eq!(
            settings_path_from(Some(""), Some("/home/u")),
            PathBuf::from("/home/u/.claude/settings.json")
        );
    }
}
