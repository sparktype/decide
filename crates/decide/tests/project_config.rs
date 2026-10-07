use std::os::unix::fs::PermissionsExt;
use std::path::Path;

#[test]
fn mcp_json_and_stop_hook_use_the_homebrew_binary() {
    if std::env::var("GITHUB_ACTIONS").is_ok() {
        eprintln!(
            "CI 러너에는 Homebrew로 설치된 /opt/homebrew/bin/decide와 \
             .claude/(gitignore 대상)가 없다 — 스킵"
        );
        return;
    }
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let config: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(root.join(".mcp.json")).unwrap()).unwrap();
    let decide = &config["mcpServers"]["decide"];
    assert_eq!(decide["command"], "/opt/homebrew/bin/decide");
    assert_eq!(decide["args"], serde_json::json!(["mcp"]));

    let binary = Path::new("/opt/homebrew/bin/decide");
    let meta = std::fs::metadata(binary).unwrap();
    assert!(meta.is_file());
    assert_ne!(meta.permissions().mode() & 0o111, 0);

    let hook = std::fs::read_to_string(root.join(".claude/hooks/stop_verify.py")).unwrap();
    assert!(hook.contains("[\"/opt/homebrew/bin/decide\", \"daemon\"]"));
    assert!(!hook.contains("decide.decide_daemon"));
}
