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
    /// 모델 없이 `deny`로 확정하는 명령 글롭 패턴(많을수록 엄격).
    pub deny_patterns: Vec<String>,
    /// 모델 없이 `ask`로 확정하는 명령 글롭 패턴(많을수록 엄격).
    pub ask_patterns: Vec<String>,
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

/// 모델을 부르지 않고 건너뛰는 읽기 전용 명령. 기준은 인자로 파일을 쓰거나 지우거나 보낼 수 없고 목적상 비밀을 드러내지
/// 않는 것이다. 그래서 `find`(`-delete`, `-exec`), `sort`·`uniq`(`-o`), `sed`·`awk`, `env`, `docker logs`,
/// `kubectl get`(`get secret -o yaml`이 비밀을 낸다)은 뺐고 `kubectl get`은 종류를 명시해 둔다. 비밀 파일을 읽는
/// 경우는 정적 규칙이 사전 필터보다 먼저 잡는다. 사전 필터는 셸 메타문자가 있는 명령을 건너뛰지 않는다.
const DEFAULT_PREFILTER: [&str; 39] = [
    "git status", "git diff", "git log", "git show", "git branch", "ls", "pwd", "cat", "head", "tail", "wc",
    "which",
    "grep", "rg", "jq", "tree", "file", "stat", "du", "df", "ps", "whoami", "date", "uname", "echo",
    "docker ps", "docker images",
    "kubectl get pods", "kubectl get nodes", "kubectl get services", "kubectl get deployments",
    "kubectl get namespaces",
    "git rev-parse", "git ls-files", "git blame", "git shortlog", "git describe", "git stash list",
    "git remote -v",
];

/// 모델 없이 `deny`로 확정하는 기본 규칙. 되돌릴 수 없고 거의 항상 의도하지 않은 명령만 둔다. 글롭 문법과 한계는
/// `gate::rules`를 본다(`\*`는 글자 별표, 끝의 ` *`는 인자 없는 경우도 맞춘다).
const DEFAULT_DENY: [&str; 29] = [
    // 루트와 홈 전체 삭제, 루트 권한 개방
    "rm -* / *", "rm -* /\\*", "rm -* ~ *", "rm -* ~/ *", "rm -* ~/\\*", "rm -* $home *", "rm -* $home/ *",
    "rm -* $home/\\*", "chmod -* * / *",
    // 디스크와 파일시스템, 포크 폭탄
    "dd * of=/dev/*", "mkfs*", "diskutil erase*", "diskutil secure*", "shred * /dev/*", "*:(){*:|:&*}*",
    // 원격 스크립트를 셸에 파이프
    "curl * | sh *", "curl * | bash *", "curl * | sudo sh *", "curl * | sudo bash *",
    "wget * | sh *", "wget * | bash *", "wget * | sudo sh *", "wget * | sudo bash *",
    // 데이터베이스 통째 삭제, 루트 전체에서 삭제
    "psql *drop database*", "mysql *drop database*", "mongosh *dropdatabase*", "redis-cli *flushall*",
    "find / *-delete*", "find / *-exec rm*",
];

/// 모델 없이 `ask`로 확정하는 기본 규칙. 의도일 수 있지만 확인이 필요한 명령이다.
const DEFAULT_ASK: [&str; 12] = [
    // 강제 푸시(`--force-with-lease`는 더 안전한 변형이라 뺀다), 추적되지 않는 파일 삭제, 저장하지 않은 작업 폐기
    "git push --force *", "git push * --force *", "git push -f *", "git push * -f *", "git clean -*f*",
    "git reset --hard*",
    // 개인 키와 자격증명 파일(공개 키 `.pub`는 이름이 달라 걸리지 않는다)
    "*/.ssh/id_rsa *", "*/.ssh/id_ed25519 *", "*/.ssh/id_ecdsa *", "*/.aws/credentials *", "*/.netrc *",
    "*/.pgpass *",
];

/// 최상위 시스템 디렉터리. 이 디렉터리 자체의 삭제와 권한 변경 규칙을 만든다(`/usr/local/...` 같은 하위 경로의
/// 삭제는 일반 작업일 수 있어 삭제 규칙에서는 뺀다).
const SYSTEM_DIRS: [&str; 8] = ["/etc", "/usr", "/bin", "/sbin", "/var", "/system", "/library", "/applications"];

/// `.env`를 읽거나 복사하는 명령. `.env.example` 같은 다른 이름은 걸리지 않는다. `grep`·`rg`·`jq`는 사전 필터에
/// 있는 읽기 명령이라 규칙이 먼저 잡지 않으면 비밀 파일을 조용히 읽고 건너뛰어진다.
const ENV_READERS: [&str; 11] = ["cat", "head", "tail", "less", "bat", "cp", "scp", "base64", "grep", "rg", "jq"];

fn default_deny_patterns() -> Vec<String> {
    let mut patterns: Vec<String> = DEFAULT_DENY.iter().map(|pattern| pattern.to_string()).collect();
    for dir in SYSTEM_DIRS {
        patterns.push(format!("rm -* {dir} *"));
        patterns.push(format!("chmod -* * {dir}*"));
    }
    patterns
}

fn default_ask_patterns() -> Vec<String> {
    let mut patterns: Vec<String> = DEFAULT_ASK.iter().map(|pattern| pattern.to_string()).collect();
    patterns.extend(ENV_READERS.iter().map(|reader| format!("{reader} *.env *")));
    patterns
}

/// 설정 키 이름. `sources`의 키이기도 하다.
pub const KEYS: [&str; 9] = [
    "mode",
    "display",
    "timeout_ms",
    "bash-risk.enabled",
    "bash-risk.deny",
    "bash-risk.confidence",
    "bash-risk.prefilter",
    "bash-risk.deny_patterns",
    "bash-risk.ask_patterns",
];

pub fn builtin() -> Config {
    Config {
        mode: Mode::Audit,
        display: Display::Decisions,
        timeout_ms: 2000,
        bash_risk: GateConfig {
            enabled: true,
            deny: 0.7,
            confidence: 0.7,
            prefilter: DEFAULT_PREFILTER.iter().map(|entry| entry.to_string()).collect(),
            deny_patterns: default_deny_patterns(),
            ask_patterns: default_ask_patterns(),
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

/// 설정을 설정 파일과 같은 모양의 JSON으로 되돌린다. `load`가 이 JSON을 읽으면 같은 값이 된다.
pub fn to_json(config: &Config) -> Value {
    let mode = match config.mode {
        Mode::Audit => "audit",
        Mode::Enforce => "enforce",
    };
    let display = match config.display {
        Display::Decisions => "decisions",
        Display::All => "all",
        Display::Off => "off",
    };
    serde_json::json!({
        "mode": mode,
        "display": display,
        "timeout_ms": config.timeout_ms,
        "gates": {
            BASH_RISK: {
                "enabled": config.bash_risk.enabled,
                "thresholds": {"deny": config.bash_risk.deny, "confidence": config.bash_risk.confidence},
                "prefilter": config.bash_risk.prefilter,
                "deny_patterns": config.bash_risk.deny_patterns,
                "ask_patterns": config.bash_risk.ask_patterns,
            }
        }
    })
}

/// 출처의 사람이 읽는 이름.
pub fn source_label(source: Source) -> &'static str {
    match source {
        Source::Builtin => "내장 기본값",
        Source::User => "사용자",
        Source::Repo => "저장소",
    }
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
    apply_patterns(layer, body, "deny_patterns", repo, |c| &mut c.deny_patterns);
    apply_patterns(layer, body, "ask_patterns", repo, |c| &mut c.ask_patterns);
}

/// 규칙 목록 하나를 병합한다. 사용자 층은 통째로 바꿀 수 있고(기본 규칙을 뺄 수도 있다), 저장소 층은 적은 항목을 기존
/// 목록에 더하기만 한다(합집합). 저장소가 기본 규칙을 다시 적을 필요도, 뺄 방법도 없다.
fn apply_patterns(
    layer: &mut Layer,
    body: &Value,
    field: &str,
    repo: bool,
    list: fn(&mut GateConfig) -> &mut Vec<String>,
) {
    let Some(value) = body.get(field) else {
        return;
    };
    let entries: Option<Vec<String>> = value.as_array().and_then(|items| {
        items
            .iter()
            .map(|item| item.as_str().filter(|text| !text.trim().is_empty()).map(str::to_string))
            .collect()
    });
    let key = format!("bash-risk.{field}");
    match entries {
        Some(entries) if repo => {
            let mut merged = list(&mut layer.loaded.config.bash_risk).clone();
            let before = merged.len();
            for entry in entries {
                if !merged.contains(&entry) {
                    merged.push(entry);
                }
            }
            // 새로 더한 항목이 없으면 값도 출처도 그대로 둔다.
            if merged.len() > before {
                layer.set(&key, |c| *list(&mut c.bash_risk) = merged.clone());
            }
        }
        Some(entries) => layer.set(&key, |c| *list(&mut c.bash_risk) = entries.clone()),
        None => layer.invalid(field, "비어 있지 않은 문자열 배열이어야 합니다"),
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
        assert_eq!(result.config.bash_risk.deny, 0.7);
        assert_eq!(result.config.bash_risk.confidence, 0.7);
        assert_eq!(result.config.bash_risk.prefilter.len(), DEFAULT_PREFILTER.len());
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
                "prefilter": ["ls", "pwd", "make test"],
                "deny_patterns": ["*mkfs*"],
                "ask_patterns": ["*clean -fd*"]
            }}
        }"#;
        let result = loaded(Some(user), None);
        let config = &result.config;
        assert_eq!(config.bash_risk.deny_patterns, vec!["*mkfs*"]);
        assert_eq!(config.bash_risk.ask_patterns, vec!["*clean -fd*"]);
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
    fn to_json_round_trips_through_the_loader() {
        // `--show --json`이 낸 설정을 그대로 설정 파일로 복사해도 같은 값이 되어야 한다.
        let user = r#"{"mode": "enforce", "display": "all", "timeout_ms": 3000, "gates": {"bash-risk": {
            "enabled": true, "thresholds": {"deny": 0.6, "confidence": 0.8}, "prefilter": ["ls", "pwd"]}}}"#;
        let first = loaded(Some(user), None).config;
        let text = to_json(&first).to_string();
        let second = loaded(Some(&text), None).config;
        assert_eq!(first, second);
        assert_eq!(to_json(&builtin())["gates"]["bash-risk"]["thresholds"]["deny"], 0.7);
        assert_eq!(to_json(&builtin())["mode"], "audit");
        assert_eq!(to_json(&builtin())["display"], "decisions");
    }

    #[test]
    fn source_labels_are_korean_and_distinct() {
        let labels = [Source::Builtin, Source::User, Source::Repo].map(source_label);
        assert_eq!(labels, ["내장 기본값", "사용자", "저장소"]);
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
        assert_eq!(config.bash_risk.deny, 0.7);
        assert_eq!(config.bash_risk.confidence, 0.6);
        assert_eq!(config.bash_risk.prefilter.len(), DEFAULT_PREFILTER.len());
        for needle in ["mode", "timeout_ms", "deny", "prefilter", "no-such-gate"] {
            assert!(has_warning(&result, needle), "{needle} 경고가 없다: {:?}", result.warnings);
        }
        assert_eq!(result.sources["display"], Source::User);
        assert_eq!(result.sources["bash-risk.confidence"], Source::User);
        assert_eq!(result.sources["mode"], Source::Builtin);
    }

    fn patterns(items: &[&str]) -> Vec<String> {
        items.iter().map(|item| item.to_string()).collect()
    }

    #[test]
    fn the_user_layer_replaces_the_rule_lists_and_is_their_source() {
        let user = r#"{"gates": {"bash-risk": {
            "deny_patterns": ["*drop database*", "*mkfs*"], "ask_patterns": ["git push*--force*"]}}}"#;
        let result = loaded(Some(user), None);
        assert_eq!(result.config.bash_risk.deny_patterns, patterns(&["*drop database*", "*mkfs*"]));
        assert_eq!(result.config.bash_risk.ask_patterns, patterns(&["git push*--force*"]));
        assert_eq!(result.sources["bash-risk.deny_patterns"], Source::User);
        assert_eq!(result.sources["bash-risk.ask_patterns"], Source::User);
        assert!(result.warnings.is_empty(), "{:?}", result.warnings);
    }

    #[test]
    fn the_repo_layer_adds_rules_to_the_current_list_and_can_never_remove() {
        let user = r#"{"gates": {"bash-risk": {"deny_patterns": ["a*", "b*"]}}}"#;
        // 저장소가 적은 항목이 기존 목록 뒤에 더해진다. 저장소 목록에 없는 기존 항목도 그대로 남는다.
        let repo = r#"{"gates": {"bash-risk": {"deny_patterns": ["c*"], "ask_patterns": ["x*"]}}}"#;
        let result = loaded(Some(user), Some(repo));
        assert_eq!(result.config.bash_risk.deny_patterns, patterns(&["a*", "b*", "c*"]));
        let ask = &result.config.bash_risk.ask_patterns;
        assert_eq!(ask.last().map(String::as_str), Some("x*"));
        assert_eq!(&ask[..ask.len() - 1], builtin().bash_risk.ask_patterns.as_slice(), "내장 기본 규칙은 그대로다");
        assert_eq!(result.sources["bash-risk.deny_patterns"], Source::Repo);
        assert_eq!(result.sources["bash-risk.ask_patterns"], Source::Repo);
        assert!(result.warnings.is_empty(), "{:?}", result.warnings);
        // 이미 있는 항목만 적으면 아무것도 바뀌지 않고 출처도 그대로다.
        let same = r#"{"gates": {"bash-risk": {"deny_patterns": ["a*"]}}}"#;
        let result = loaded(Some(user), Some(same));
        assert_eq!(result.config.bash_risk.deny_patterns, patterns(&["a*", "b*"]));
        assert_eq!(result.sources["bash-risk.deny_patterns"], Source::User);
        assert!(result.warnings.is_empty(), "{:?}", result.warnings);
        // 중복은 한 번만 더해진다.
        let dup = r#"{"gates": {"bash-risk": {"deny_patterns": ["a*", "c*", "c*"]}}}"#;
        let result = loaded(Some(user), Some(dup));
        assert_eq!(result.config.bash_risk.deny_patterns, patterns(&["a*", "b*", "c*"]));
    }

    #[test]
    fn the_builtin_rules_are_not_empty_and_have_a_builtin_source() {
        let result = loaded(None, None);
        assert!(!result.config.bash_risk.deny_patterns.is_empty());
        assert!(!result.config.bash_risk.ask_patterns.is_empty());
        assert_eq!(result.sources["bash-risk.deny_patterns"], Source::Builtin);
        assert!(result.warnings.is_empty());
    }

    #[test]
    fn invalid_rule_lists_are_ignored_with_a_warning() {
        for body in [
            r#"{"deny_patterns": "*mkfs*"}"#,
            r#"{"deny_patterns": [1, 2]}"#,
            r#"{"deny_patterns": ["*mkfs*", ""]}"#,
            r#"{"deny_patterns": ["   "]}"#,
            r#"{"ask_patterns": {"a": "b"}}"#,
        ] {
            let user = format!(r#"{{"gates": {{"bash-risk": {body}}}}}"#);
            let result = loaded(Some(&user), None);
            assert_eq!(result.config.bash_risk.deny_patterns, builtin().bash_risk.deny_patterns, "{body}");
            assert_eq!(result.config.bash_risk.ask_patterns, builtin().bash_risk.ask_patterns, "{body}");
            assert!(has_warning(&result, "_patterns"), "{body}: {:?}", result.warnings);
        }
    }

    #[test]
    fn rule_lists_survive_a_round_trip_through_to_json() {
        let user = r#"{"gates": {"bash-risk": {"deny_patterns": ["*mkfs*"], "ask_patterns": ["*clean -fd*", "*.env"]}}}"#;
        let first = loaded(Some(user), None).config;
        let json = to_json(&first);
        assert_eq!(json["gates"]["bash-risk"]["deny_patterns"], serde_json::json!(["*mkfs*"]));
        let again = loaded(Some(&json.to_string()), None).config;
        assert_eq!(again, first);
    }

    #[test]
    fn the_rule_lists_have_source_keys() {
        assert!(KEYS.contains(&"bash-risk.deny_patterns"));
        assert!(KEYS.contains(&"bash-risk.ask_patterns"));
    }
}
