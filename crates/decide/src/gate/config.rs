// 게이트 설정을 내장 기본값, 사용자, 저장소 층으로 병합한다(저장소는 조이기만 허용)
use serde_json::Value;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

pub const BASH_RISK: &str = "bash-risk";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Audit,
    Enforce,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Display {
    Decisions,
    All,
    Off,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    Builtin,
    User,
    Repo,
}

#[derive(Debug, Clone, PartialEq)]
pub struct GateConfig {
    pub enabled: bool,
    /// `deny` 확률이 이 값 이상이면 `deny`다(낮을수록 엄격).
    pub deny: f64,
    /// 최고 확률이 이 값 미만이면 `ask`다(높을수록 엄격).
    pub confidence: f64,
    /// 데몬을 부르지 않고 건너뛰는 명령 앞부분(적을수록 엄격).
    pub prefilter: Vec<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Config {
    pub mode: Mode,
    pub display: Display,
    pub timeout_ms: u64,
    pub bash_risk: GateConfig,
}

/// 병합 결과. `sources`는 `--show`가 값마다 출처를 보이는 데 쓰고, `warnings`는 무시한 항목을 알린다.
#[derive(Debug, Clone, PartialEq)]
pub struct Loaded {
    pub config: Config,
    pub sources: BTreeMap<String, Source>,
    pub warnings: Vec<String>,
}

const DEFAULT_PREFILTER: [&str; 12] = [
    "git status", "git diff", "git log", "git show", "git branch", "ls", "pwd", "cat", "head", "tail", "wc",
    "which",
];

/// 설정 키 이름. `sources`의 키이기도 하다.
pub const KEYS: [&str; 7] = [
    "mode",
    "display",
    "timeout_ms",
    "bash-risk.enabled",
    "bash-risk.deny",
    "bash-risk.confidence",
    "bash-risk.prefilter",
];

pub fn builtin() -> Config {
    Config {
        mode: Mode::Audit,
        display: Display::Decisions,
        timeout_ms: 2000,
        bash_risk: GateConfig {
            enabled: true,
            deny: 0.5,
            confidence: 0.7,
            prefilter: DEFAULT_PREFILTER.iter().map(|entry| entry.to_string()).collect(),
        },
    }
}

/// 사용자 설정 텍스트와 저장소 설정 텍스트(없으면 `None`)를 내장 기본값 위에 차례로 병합한다.
pub fn load(user: Option<&str>, repo: Option<&str>) -> Loaded {
    let mut loaded = Loaded {
        config: builtin(),
        sources: KEYS.iter().map(|key| (key.to_string(), Source::Builtin)).collect(),
        warnings: Vec::new(),
    };
    if let Some(text) = user {
        apply_layer(&mut loaded, text, Source::User);
    }
    if let Some(text) = repo {
        apply_layer(&mut loaded, text, Source::Repo);
    }
    loaded
}

/// 사용자 설정(`~/.config/decide/gates.json`, HOME이 없거나 비면 없음)과 저장소 설정(`<cwd>/.decide/gates.json`) 경로.
pub fn config_paths(home: Option<&str>, cwd: &Path) -> (Option<PathBuf>, PathBuf) {
    let user = home
        .filter(|home| !home.is_empty())
        .map(|home| Path::new(home).join(".config/decide/gates.json"));
    (user, cwd.join(".decide/gates.json"))
}

/// 설정 파일을 읽어 `load`로 병합한다. 파일이 없으면 그 층을 건너뛰고, 읽기에 실패하면 경고만 남긴다.
pub fn load_from_disk(home: Option<&str>, cwd: &Path) -> Loaded {
    let (user_path, repo_path) = config_paths(home, cwd);
    let mut warnings = Vec::new();
    let mut read = |path: Option<PathBuf>, name: &str| -> Option<String> {
        let path = path?;
        match std::fs::read_to_string(&path) {
            Ok(text) => Some(text),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => None,
            Err(err) => {
                warnings.push(format!("{name} 설정을 읽지 못해 건너뜁니다 ({}): {err}", path.display()));
                None
            }
        }
    };
    let user = read(user_path, "사용자");
    let repo = read(Some(repo_path), "저장소");
    let mut loaded = load(user.as_deref(), repo.as_deref());
    warnings.append(&mut loaded.warnings);
    loaded.warnings = warnings;
    loaded
}

fn layer_name(source: Source) -> &'static str {
    match source {
        Source::Builtin => "내장",
        Source::User => "사용자",
        Source::Repo => "저장소",
    }
}

/// 한 층을 병합한다. 사용자 층은 무엇이든 바꿀 수 있고, 저장소 층은 더 엄격한 쪽으로만 바꿀 수 있다.
fn apply_layer(loaded: &mut Loaded, text: &str, source: Source) {
    let name = layer_name(source);
    let root: Value = match serde_json::from_str(text) {
        Ok(value @ Value::Object(_)) => value,
        Ok(_) => {
            loaded.warnings.push(format!("{name} 설정이 JSON 객체가 아니라 건너뜁니다"));
            return;
        }
        Err(err) => {
            loaded.warnings.push(format!("{name} 설정을 JSON으로 읽지 못해 건너뜁니다: {err}"));
            return;
        }
    };
    let repo = source == Source::Repo;
    let mut layer = Layer { loaded, source, name };

    if let Some(value) = root.get("mode") {
        let current = layer.loaded.config.mode;
        match value.as_str() {
            Some("audit") if !repo => layer.set("mode", |c| c.mode = Mode::Audit),
            // 저장소가 이미 enforce인 것을 audit로 풀려는 경우만 거부하고, 같은 값 반복은 그냥 둔다.
            Some("audit") if current == Mode::Enforce => layer.refuse("mode", "audit로 풀 수 없습니다"),
            Some("audit") => {}
            Some("enforce") if repo && current == Mode::Enforce => {}
            Some("enforce") => layer.set("mode", |c| c.mode = Mode::Enforce),
            _ => layer.invalid("mode", "audit 또는 enforce여야 합니다"),
        }
    }
    if let Some(value) = root.get("display") {
        let parsed = match value.as_str() {
            Some("decisions") => Some(Display::Decisions),
            Some("all") => Some(Display::All),
            Some("off") => Some(Display::Off),
            _ => None,
        };
        match parsed {
            Some(_) if repo => layer.refuse("display", "저장소는 표시 방식을 바꿀 수 없습니다"),
            Some(display) => layer.set("display", |c| c.display = display),
            None => layer.invalid("display", "decisions, all, off 중 하나여야 합니다"),
        }
    }
    if let Some(value) = root.get("timeout_ms") {
        match value.as_u64().filter(|ms| *ms > 0) {
            Some(_) if repo => layer.refuse("timeout_ms", "저장소는 시간 제한을 바꿀 수 없습니다"),
            Some(ms) => layer.set("timeout_ms", |c| c.timeout_ms = ms),
            None => layer.invalid("timeout_ms", "0보다 큰 정수여야 합니다"),
        }
    }
    if let Some(gates) = root.get("gates") {
        match gates.as_object() {
            Some(gates) => {
                for (gate, body) in gates {
                    if gate == BASH_RISK {
                        apply_bash_risk(&mut layer, body, repo);
                    } else {
                        layer.warn(format!("gates.{gate}는 알 수 없는 게이트라 무시합니다"));
                    }
                }
            }
            None => layer.invalid("gates", "객체여야 합니다"),
        }
    }
}

fn apply_bash_risk(layer: &mut Layer, body: &Value, repo: bool) {
    if let Some(value) = body.get("enabled") {
        let current = layer.loaded.config.bash_risk.enabled;
        match value.as_bool() {
            Some(false) if repo && current => layer.refuse("enabled", "저장소가 게이트를 끌 수 없습니다"),
            Some(false) if repo => {}
            Some(true) if repo && current => {}
            Some(enabled) => layer.set("bash-risk.enabled", |c| c.bash_risk.enabled = enabled),
            None => layer.invalid("enabled", "true 또는 false여야 합니다"),
        }
    }
    if let Some(thresholds) = body.get("thresholds") {
        let Some(thresholds) = thresholds.as_object() else {
            layer.invalid("thresholds", "객체여야 합니다");
            return;
        };
        if let Some(value) = thresholds.get("deny") {
            let current = layer.loaded.config.bash_risk.deny;
            match probability(value) {
                Some(deny) if repo && deny > current => {
                    layer.refuse("deny", "deny 임계값을 올릴 수 없습니다(낮출수록 엄격)")
                }
                Some(deny) => layer.set("bash-risk.deny", |c| c.bash_risk.deny = deny),
                None => layer.invalid("deny", "0과 1 사이 숫자여야 합니다"),
            }
        }
        if let Some(value) = thresholds.get("confidence") {
            let current = layer.loaded.config.bash_risk.confidence;
            match probability(value) {
                Some(confidence) if repo && confidence < current => {
                    layer.refuse("confidence", "confidence 임계값을 내릴 수 없습니다(올릴수록 엄격)")
                }
                Some(confidence) => layer.set("bash-risk.confidence", |c| c.bash_risk.confidence = confidence),
                None => layer.invalid("confidence", "0과 1 사이 숫자여야 합니다"),
            }
        }
    }
    if let Some(value) = body.get("prefilter") {
        let entries: Option<Vec<String>> = value
            .as_array()
            .and_then(|items| items.iter().map(|item| item.as_str().map(str::to_string)).collect());
        match entries {
            Some(entries) if repo => {
                let current = &layer.loaded.config.bash_risk.prefilter;
                if entries.iter().all(|entry| current.contains(entry)) {
                    layer.set("bash-risk.prefilter", |c| c.bash_risk.prefilter = entries.clone());
                } else {
                    layer.refuse("prefilter", "저장소는 사전 필터에 항목을 추가할 수 없습니다(줄이기만 가능)");
                }
            }
            Some(entries) => layer.set("bash-risk.prefilter", |c| c.bash_risk.prefilter = entries.clone()),
            None => layer.invalid("prefilter", "문자열 배열이어야 합니다"),
        }
    }
}

fn probability(value: &Value) -> Option<f64> {
    value.as_f64().filter(|p| (0.0..=1.0).contains(p))
}

/// 한 층을 병합하는 동안의 상태. 값을 쓰면서 출처를 함께 기록한다.
struct Layer<'a> {
    loaded: &'a mut Loaded,
    source: Source,
    name: &'static str,
}

impl Layer<'_> {
    fn set(&mut self, key: &str, apply: impl FnOnce(&mut Config)) {
        apply(&mut self.loaded.config);
        self.loaded.sources.insert(key.to_string(), self.source);
    }

    fn warn(&mut self, message: String) {
        self.loaded.warnings.push(format!("{} 설정: {message}", self.name));
    }

    fn refuse(&mut self, field: &str, reason: &str) {
        self.warn(format!("{field}를 적용하지 않습니다: {reason}"));
    }

    fn invalid(&mut self, field: &str, expected: &str) {
        self.warn(format!("{field} 값이 올바르지 않아 무시합니다: {expected}"));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn loaded(user: Option<&str>, repo: Option<&str>) -> Loaded {
        load(user, repo)
    }

    fn has_warning(loaded: &Loaded, needle: &str) -> bool {
        loaded.warnings.iter().any(|warning| warning.contains(needle))
    }

    #[test]
    fn defaults_are_audit_decisions_and_the_bash_risk_gate_on() {
        let result = loaded(None, None);
        assert_eq!(result.config, builtin());
        assert_eq!(result.config.mode, Mode::Audit);
        assert_eq!(result.config.display, Display::Decisions);
        assert_eq!(result.config.timeout_ms, 2000);
        assert!(result.config.bash_risk.enabled);
        assert_eq!(result.config.bash_risk.deny, 0.5);
        assert_eq!(result.config.bash_risk.confidence, 0.7);
        assert_eq!(result.config.bash_risk.prefilter.len(), 12);
        assert!(result.sources.values().all(|source| *source == Source::Builtin));
        assert_eq!(result.sources.len(), KEYS.len());
        assert!(result.warnings.is_empty());
    }

    #[test]
    fn the_user_layer_can_change_every_field_and_is_the_source() {
        let user = r#"{
            "mode": "enforce", "display": "all", "timeout_ms": 5000,
            "gates": {"bash-risk": {
                "enabled": false,
                "thresholds": {"deny": 0.8, "confidence": 0.4},
                "prefilter": ["ls", "pwd", "make test"]
            }}
        }"#;
        let result = loaded(Some(user), None);
        let config = &result.config;
        assert_eq!(config.mode, Mode::Enforce);
        assert_eq!(config.display, Display::All);
        assert_eq!(config.timeout_ms, 5000);
        assert!(!config.bash_risk.enabled);
        assert_eq!(config.bash_risk.deny, 0.8);
        assert_eq!(config.bash_risk.confidence, 0.4);
        assert_eq!(config.bash_risk.prefilter, vec!["ls", "pwd", "make test"]);
        assert!(result.sources.values().all(|source| *source == Source::User));
        assert!(result.warnings.is_empty(), "{:?}", result.warnings);
    }

    #[test]
    fn the_repo_layer_cannot_loosen_anything_the_user_set() {
        let user = r#"{"mode": "enforce", "gates": {"bash-risk": {
            "thresholds": {"deny": 0.4, "confidence": 0.8}, "prefilter": ["ls", "pwd"]}}}"#;
        let repo = r#"{"mode": "audit", "display": "off", "timeout_ms": 1, "gates": {"bash-risk": {
            "enabled": false,
            "thresholds": {"deny": 0.9, "confidence": 0.3},
            "prefilter": ["ls", "pwd", "rm"]}}}"#;
        let result = loaded(Some(user), Some(repo));
        let config = &result.config;
        assert_eq!(config.mode, Mode::Enforce, "audit로 풀 수 없다");
        assert_eq!(config.display, Display::Decisions, "표시 방식은 저장소가 못 바꾼다");
        assert_eq!(config.timeout_ms, 2000, "timeout은 저장소가 못 바꾼다");
        assert!(config.bash_risk.enabled, "저장소가 게이트를 끌 수 없다");
        assert_eq!(config.bash_risk.deny, 0.4, "deny 임계값을 올릴 수 없다");
        assert_eq!(config.bash_risk.confidence, 0.8, "confidence 임계값을 내릴 수 없다");
        assert_eq!(config.bash_risk.prefilter, vec!["ls", "pwd"], "사전 필터에 추가할 수 없다");
        for field in ["mode", "display", "timeout_ms", "enabled", "deny", "confidence", "prefilter"] {
            assert!(has_warning(&result, field), "{field} 경고가 없다: {:?}", result.warnings);
        }
        assert_eq!(result.sources["mode"], Source::User);
        assert_eq!(result.sources["bash-risk.deny"], Source::User);
    }

    #[test]
    fn the_repo_layer_can_tighten() {
        let user = r#"{"gates": {"bash-risk": {"enabled": false}}}"#;
        let repo = r#"{"mode": "enforce", "gates": {"bash-risk": {
            "enabled": true,
            "thresholds": {"deny": 0.3, "confidence": 0.9},
            "prefilter": ["ls", "pwd"]}}}"#;
        let result = loaded(Some(user), Some(repo));
        let config = &result.config;
        assert_eq!(config.mode, Mode::Enforce);
        assert!(config.bash_risk.enabled);
        assert_eq!(config.bash_risk.deny, 0.3);
        assert_eq!(config.bash_risk.confidence, 0.9);
        assert_eq!(config.bash_risk.prefilter, vec!["ls", "pwd"]);
        for key in ["mode", "bash-risk.enabled", "bash-risk.deny", "bash-risk.confidence", "bash-risk.prefilter"] {
            assert_eq!(result.sources[key], Source::Repo, "{key}");
        }
        assert_eq!(result.sources["display"], Source::Builtin);
        assert!(result.warnings.is_empty(), "{:?}", result.warnings);
    }

    #[test]
    fn the_repo_restating_the_current_value_is_neither_a_warning_nor_a_source_change() {
        // 이미 audit이고 이미 꺼진 상태에서 저장소가 같은 값을 쓰는 것은 풀려는 시도가 아니다.
        let user = r#"{"gates": {"bash-risk": {"enabled": false}}}"#;
        let repo = r#"{"mode": "audit", "gates": {"bash-risk": {"enabled": false}}}"#;
        let result = loaded(Some(user), Some(repo));
        assert!(result.warnings.is_empty(), "{:?}", result.warnings);
        assert_eq!(result.sources["mode"], Source::Builtin);
        assert_eq!(result.sources["bash-risk.enabled"], Source::User);
        assert_eq!(result.config.mode, Mode::Audit);
        assert!(!result.config.bash_risk.enabled);
    }

    fn temp_dir(label: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "decide-gate-config-{label}-{}-{}",
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
    fn config_paths_follow_the_documented_locations() {
        let (user, repo) = config_paths(Some("/home/u"), std::path::Path::new("/work/proj"));
        assert_eq!(user, Some(std::path::PathBuf::from("/home/u/.config/decide/gates.json")));
        assert_eq!(repo, std::path::PathBuf::from("/work/proj/.decide/gates.json"));
        let (user, _) = config_paths(None, std::path::Path::new("/work/proj"));
        assert_eq!(user, None);
        let (user, _) = config_paths(Some(""), std::path::Path::new("/work/proj"));
        assert_eq!(user, None, "HOME이 비어 있으면 사용자 층은 없다");
    }

    #[test]
    fn load_from_disk_reads_both_layers_and_skips_missing_files() {
        let home = temp_dir("home");
        let cwd = temp_dir("cwd");
        // 파일이 하나도 없으면 내장 기본값이고 경고도 없다.
        let none = load_from_disk(Some(home.to_str().unwrap()), &cwd);
        assert_eq!(none.config, builtin());
        assert!(none.warnings.is_empty(), "{:?}", none.warnings);

        std::fs::create_dir_all(home.join(".config/decide")).unwrap();
        std::fs::write(home.join(".config/decide/gates.json"), r#"{"display": "all"}"#).unwrap();
        std::fs::create_dir_all(cwd.join(".decide")).unwrap();
        std::fs::write(cwd.join(".decide/gates.json"), r#"{"mode": "enforce"}"#).unwrap();
        let both = load_from_disk(Some(home.to_str().unwrap()), &cwd);
        assert_eq!(both.config.display, Display::All);
        assert_eq!(both.config.mode, Mode::Enforce);
        assert_eq!(both.sources["display"], Source::User);
        assert_eq!(both.sources["mode"], Source::Repo);
        let _ = std::fs::remove_dir_all(&home);
        let _ = std::fs::remove_dir_all(&cwd);
    }

    #[test]
    fn an_unreadable_config_path_is_a_warning_not_a_failure() {
        let home = temp_dir("unreadable");
        // 설정 경로가 파일이 아니라 디렉터리면 읽기에 실패한다.
        std::fs::create_dir_all(home.join(".config/decide/gates.json")).unwrap();
        let result = load_from_disk(Some(home.to_str().unwrap()), &home);
        assert_eq!(result.config, builtin());
        assert!(has_warning(&result, "읽지 못"), "{:?}", result.warnings);
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn invalid_json_skips_that_layer_with_a_warning() {
        let result = loaded(Some("{ not json"), Some(r#"{"mode": "enforce"}"#));
        assert_eq!(result.config.mode, Mode::Enforce);
        assert!(has_warning(&result, "사용자"), "{:?}", result.warnings);
        let result = loaded(None, Some("[1, 2]"));
        assert_eq!(result.config, builtin());
        assert!(has_warning(&result, "저장소"), "{:?}", result.warnings);
    }

    #[test]
    fn bad_values_are_ignored_but_good_siblings_still_apply() {
        let user = r#"{"mode": "sometimes", "display": "all", "timeout_ms": -5, "gates": {
            "bash-risk": {"thresholds": {"deny": 1.5, "confidence": 0.6}, "prefilter": "ls"},
            "no-such-gate": {}}}"#;
        let result = loaded(Some(user), None);
        let config = &result.config;
        assert_eq!(config.mode, Mode::Audit);
        assert_eq!(config.display, Display::All);
        assert_eq!(config.timeout_ms, 2000);
        assert_eq!(config.bash_risk.deny, 0.5);
        assert_eq!(config.bash_risk.confidence, 0.6);
        assert_eq!(config.bash_risk.prefilter.len(), 12);
        for needle in ["mode", "timeout_ms", "deny", "prefilter", "no-such-gate"] {
            assert!(has_warning(&result, needle), "{needle} 경고가 없다: {:?}", result.warnings);
        }
        assert_eq!(result.sources["display"], Source::User);
        assert_eq!(result.sources["bash-risk.confidence"], Source::User);
        assert_eq!(result.sources["mode"], Source::Builtin);
    }
}
