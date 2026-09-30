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

pub fn add_hook(settings: &mut Value, command: &str) -> Result<bool, String> {
    let root = settings
        .as_object()
        .ok_or_else(|| "설정 파일이 JSON 객체가 아닙니다".to_string())?;
    match root.get("hooks") {
        None => {}
        Some(Value::Object(hooks)) => match hooks.get("PostToolUse") {
            None | Some(Value::Array(_)) => {}
            Some(_) => return Err("설정의 hooks.PostToolUse가 배열이 아닙니다".to_string()),
        },
        Some(_) => return Err("설정의 hooks가 객체가 아닙니다".to_string()),
    }
    if already_present(settings, command) {
        return Ok(false);
    }
    let group = json!({
        "matcher": HOOK_MATCHER,
        "hooks": [{"type": "command", "command": command, "timeout": HOOK_TIMEOUT_SECS}]
    });
    if let Some(root) = settings.as_object_mut() {
        if let Value::Object(hooks) = root.entry("hooks").or_insert_with(|| json!({})) {
            if let Value::Array(groups) = hooks.entry("PostToolUse").or_insert_with(|| json!([])) {
                groups.push(group);
            }
        }
    }
    Ok(true)
}

fn already_present(settings: &Value, command: &str) -> bool {
    settings
        .get("hooks")
        .and_then(|hooks| hooks.get("PostToolUse"))
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
    if !add_hook(&mut settings, command)? {
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
