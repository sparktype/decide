// 게이트 판정을 Claude Code 훅 출력 JSON과 사용자에게 보여 줄 근거 문구로 바꾼다
use crate::gate::bash_risk::{Probs, Verdict, OPTIONS, QUESTION};
use crate::gate::config::{self, Config, Display, Loaded, Mode, BASH_RISK};
use crate::show::{footer, pct, truncate};
use serde_json::{json, Value};

/// 한 번의 훅 호출이 어떻게 끝났는가.
#[derive(Debug, Clone)]
pub enum Kind {
    /// 사전 필터로 데몬을 부르지 않고 끝났다.
    Prefiltered,
    /// 데몬이 답했고 판정이 나왔다. `result`는 데몬 답 전체(라우팅·지연 표시에 쓴다).
    Judged { verdict: Verdict, probs: Probs, result: Value },
    /// 데몬 연결 실패, 시간 초과, 백엔드 오류 등으로 판정 없이 통과했다.
    Failed { reason: String },
    /// 정적 규칙(글롭 패턴)이 모델을 부르지 않고 `Deny` 또는 `Ask`로 확정했다.
    Rule { verdict: Verdict, pattern: String },
}

#[derive(Debug, Clone)]
pub struct Outcome {
    pub gate: String,
    pub mode: Mode,
    pub display: Display,
    /// 이미 비밀값을 가린 명령.
    pub command: String,
    pub kind: Kind,
    /// 설정을 병합하며 무시한 항목 수.
    pub warnings: usize,
}

/// `--show`가 보여 줄 설정 파일 위치와 존재 여부.
#[derive(Debug, Clone)]
pub struct Location {
    pub label: String,
    pub exists: bool,
}

/// `decide gate --show`: 게이트 목록과 설정 파일 위치(+경고).
pub fn show_overview(loaded: &Loaded, user: &Location, repo: &Location) -> String {
    let config = &loaded.config;
    let mode = if config.bash_risk.enabled { mode_name(config.mode) } else { "꺼짐" };
    let mut lines = vec![
        format!(
            "{BASH_RISK}   PreToolUse/Bash   choice   {mode}   display={}",
            display_name(config.display)
        ),
        format!("user-rules  {}", location_text(user)),
        format!("repo-rules  {}", location_text(repo)),
    ];
    lines.extend(loaded.warnings.iter().map(|warning| format!("경고: {warning}")));
    lines.join("\n")
}

fn location_text(location: &Location) -> String {
    format!("{} ({})", location.label, if location.exists { "있음" } else { "없음" })
}

fn mode_name(mode: Mode) -> &'static str {
    match mode {
        Mode::Audit => "감사 모드",
        Mode::Enforce => "enforce",
    }
}

fn display_name(display: Display) -> &'static str {
    match display {
        Display::Decisions => "decisions",
        Display::All => "all",
        Display::Off => "off",
    }
}

/// `--show`가 사전 필터에서 보이는 항목 수 상한. 나머지는 개수만 알린다.
const SHOWN_PREFILTER: usize = 12;

/// `--show`가 규칙 종류마다 보이는 줄 수 상한. 기본 규칙이 많아 전부 나열하면 화면을 채운다.
const SHOWN_RULES: usize = 8;

/// 규칙 목록을 줄로 만든다. 상한을 넘으면 나머지 개수와 전체를 보는 방법을 한 줄로 알린다.
fn rule_lines(kind: &str, patterns: &[String]) -> Vec<String> {
    let mut lines: Vec<String> =
        patterns.iter().take(SHOWN_RULES).map(|pattern| format!("  {kind:<5} {pattern}")).collect();
    if patterns.len() > SHOWN_RULES {
        lines.push(format!("  … {kind} 외 {}개 (--json으로 전체 목록)", patterns.len() - SHOWN_RULES));
    }
    lines
}

/// 설정 키 하나의 값을 사람이 읽는 문자열로 만든다.
fn value_text(config: &Config, key: &str) -> String {
    match key {
        "mode" => config::to_json(config)["mode"].as_str().unwrap_or("?").to_string(),
        "display" => display_name(config.display).to_string(),
        "timeout_ms" => config.timeout_ms.to_string(),
        "bash-risk.enabled" => config.bash_risk.enabled.to_string(),
        "bash-risk.deny" => config.bash_risk.deny.to_string(),
        "bash-risk.confidence" => config.bash_risk.confidence.to_string(),
        "bash-risk.prefilter" => format!("{}개", config.bash_risk.prefilter.len()),
        "bash-risk.deny_patterns" => format!("{}개", config.bash_risk.deny_patterns.len()),
        "bash-risk.ask_patterns" => format!("{}개", config.bash_risk.ask_patterns.len()),
        _ => "?".to_string(),
    }
}

/// `decide gate --show <이름>`: 게이트 하나의 질문, 선택지, 임계값, 모드, 사전 필터와 값마다의 출처.
/// `json`이면 같은 내용을 JSON으로 낸다(`config`는 설정 파일에 그대로 복사할 수 있는 모양이다).
/// 알 수 없는 게이트 이름이면 `None`이다.
pub fn show_gate(loaded: &Loaded, name: &str, json: bool) -> Option<String> {
    if name != BASH_RISK {
        return None;
    }
    let config = &loaded.config;
    if json {
        let options: serde_json::Map<String, Value> = OPTIONS
            .iter()
            .map(|(label, meaning)| (label.to_string(), Value::String(meaning.to_string())))
            .collect();
        let sources: serde_json::Map<String, Value> = loaded
            .sources
            .iter()
            .map(|(key, source)| (key.clone(), Value::String(config::source_label(*source).to_string())))
            .collect();
        let value = json!({
            "gate": BASH_RISK,
            "event": "PreToolUse",
            "matcher": "Bash",
            "type": "choice",
            "question": QUESTION,
            "options": options,
            "state": ["command", "cwd_tail"],
            "config": config::to_json(config),
            "sources": sources,
            "warnings": loaded.warnings,
        });
        return serde_json::to_string_pretty(&value).ok();
    }
    let mut lines = vec![
        "이벤트:   PreToolUse (matcher: Bash)".to_string(),
        "타입:     choice".to_string(),
        format!("질문:     {QUESTION}"),
    ];
    for (index, (label, meaning)) in OPTIONS.iter().enumerate() {
        let head = if index == 0 { "선택지:   " } else { "          " };
        lines.push(format!("{head}{label:<5} — {meaning}"));
    }
    lines.push("state:    명령(비밀값을 가린 뒤 2000자까지), 작업 디렉터리 끝 두 단계".to_string());
    lines.push(format!(
        "임계값:   deny ≥ {:.2}, 최고 확률 < {:.2}이면 ask",
        config.bash_risk.deny, config.bash_risk.confidence
    ));
    lines.push(format!("모드:     {}", value_text(config, "mode")));
    let prefilter = &config.bash_risk.prefilter;
    let shown = prefilter.iter().take(SHOWN_PREFILTER).cloned().collect::<Vec<_>>().join(", ");
    let more = if prefilter.len() > SHOWN_PREFILTER {
        format!(", … 외 {}개", prefilter.len() - SHOWN_PREFILTER)
    } else {
        String::new()
    };
    lines.push(format!("사전 필터: {shown}{more} ({}개)", prefilter.len()));
    lines.push(format!(
        "정적 규칙: deny {}개, ask {}개 (걸리면 모델 없이 확정, 사전 필터보다 먼저 본다)",
        config.bash_risk.deny_patterns.len(),
        config.bash_risk.ask_patterns.len()
    ));
    lines.extend(rule_lines("deny", &config.bash_risk.deny_patterns));
    lines.extend(rule_lines("ask", &config.bash_risk.ask_patterns));
    lines.push("값과 출처:".to_string());
    for key in config::KEYS {
        let source = loaded.sources.get(key).copied().unwrap_or(config::Source::Builtin);
        lines.push(format!("  {key} = {} ({})", value_text(config, key), config::source_label(source)));
    }
    if !loaded.warnings.is_empty() {
        lines.push("경고:".to_string());
        lines.extend(loaded.warnings.iter().map(|warning| format!("  {warning}")));
    }
    Some(lines.join("\n"))
}

/// 훅이 stdout으로 내보낼 JSON. 내보낼 것이 없으면 `None`이다.
/// 감사 모드는 `systemMessage`만 내고 `permissionDecision`은 절대 내지 않는다.
pub fn hook_output(outcome: &Outcome) -> Option<Value> {
    let decision = match (&outcome.kind, outcome.mode) {
        (Kind::Judged { verdict, probs, .. }, Mode::Enforce) => permission(*verdict, probs, &outcome.gate),
        (Kind::Rule { verdict, pattern }, Mode::Enforce) => rule_permission(*verdict, pattern, &outcome.gate),
        _ => None,
    };
    let message = message(outcome);
    if decision.is_none() && message.is_none() {
        return None;
    }
    let mut output = json!({});
    if let Some((decision, reason)) = decision {
        output["hookSpecificOutput"] = json!({
            "hookEventName": "PreToolUse",
            "permissionDecision": decision,
            "permissionDecisionReason": reason,
        });
    }
    if let Some(text) = message {
        output["systemMessage"] = Value::String(text);
    }
    Some(output)
}

/// enforce 모드에서 Claude Code에 넘길 `(permissionDecision, 이유)`. 판정 없음(Allow)은 `None`이다.
fn permission(verdict: Verdict, probs: &Probs, gate: &str) -> Option<(&'static str, String)> {
    let (decision, meaning) = match verdict {
        Verdict::Allow => return None,
        Verdict::Ask => ("ask", "영향 범위가 불분명해 확인이 필요하다고 판정했습니다"),
        Verdict::Deny => ("deny", "저장소 밖을 파괴하거나 되돌리기 어렵게 바꿀 가능성이 높다고 판정했습니다"),
    };
    Some((decision, format!("decide gate {gate}: {meaning} ({})", ranked(probs))))
}

/// 정적 규칙 판정을 enforce 모드에서 Claude Code에 넘길 `(permissionDecision, 이유)`. `Allow`는 `None`이다.
fn rule_permission(verdict: Verdict, pattern: &str, gate: &str) -> Option<(&'static str, String)> {
    let decision = match verdict {
        Verdict::Allow => return None,
        Verdict::Ask => "ask",
        Verdict::Deny => "deny",
    };
    Some((decision, format!("decide gate {gate}: 정적 규칙 `{pattern}`에 걸렸습니다 (판정: {decision}, 모델 호출 없음)")))
}

/// 사용자에게 보여 줄 근거 문구. 표시 방식(`display`)과 결과 종류에 따라 없을 수 있다.
/// 고정된 질문 문구(`QUESTION`)는 매 호출 반복이라 보이지 않는다 — `decide gate --show`로 본다.
fn message(outcome: &Outcome) -> Option<String> {
    let head = format!("🛡 decide gate {}:", outcome.gate);
    let command = truncate(&outcome.command);
    let mut text = match &outcome.kind {
        Kind::Prefiltered if outcome.display == Display::All => {
            format!("{head} 사전 필터 통과\n- 대상: {command}")
        }
        Kind::Failed { reason } if outcome.display != Display::Off => {
            format!("{head} 판정 없이 통과 ({reason})")
        }
        Kind::Judged { verdict, probs, result } => {
            let shown = match verdict {
                Verdict::Allow => outcome.display == Display::All,
                Verdict::Ask | Verdict::Deny => outcome.display != Display::Off,
            };
            if !shown {
                return None;
            }
            format!(
                "{head} {} ({})\n- 대상: {command}\n- 선택: {} · {}",
                verdict_label(*verdict),
                mode_text(outcome.mode, *verdict),
                ranked(probs),
                footer(result),
            )
        }
        Kind::Rule { verdict, pattern } if *verdict != Verdict::Allow && outcome.display != Display::Off => {
            format!(
                "{head} {} ({})\n- 근거: 정적 규칙 `{pattern}` (모델 호출 없음)\n- 대상: {command}",
                verdict_label(*verdict),
                mode_text(outcome.mode, *verdict),
            )
        }
        _ => return None,
    };
    if outcome.warnings > 0 {
        text.push_str(&format!("\n- 설정 경고 {}건: decide gate --show로 확인", outcome.warnings));
    }
    Some(text)
}

fn verdict_label(verdict: Verdict) -> &'static str {
    match verdict {
        Verdict::Allow => "allow(판정 없음)",
        Verdict::Ask => "ask",
        Verdict::Deny => "deny",
    }
}

fn mode_text(mode: Mode, verdict: Verdict) -> &'static str {
    match (mode, verdict) {
        (Mode::Audit, _) => "감사 모드 — 막지 않음",
        (Mode::Enforce, Verdict::Deny) => "enforce — 차단",
        (Mode::Enforce, Verdict::Ask) => "enforce — 확인 요청",
        (Mode::Enforce, Verdict::Allow) => "enforce — 통과",
    }
}

/// "deny 62% · ask 30% · allow 8%". 확률이 같으면 더 엄격한 쪽이 앞에 온다.
fn ranked(probs: &Probs) -> String {
    let mut items = [("deny", probs.deny), ("ask", probs.ask), ("allow", probs.allow)];
    items.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    items
        .iter()
        .map(|(label, probability)| format!("{label} {}%", pct(*probability)))
        .collect::<Vec<_>>()
        .join(" · ")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn result() -> Value {
        json!({"routing": {"backend": "local", "model": "clef-flash"}, "latency_ms": 540.2})
    }

    fn probs() -> Probs {
        Probs { allow: 0.08, ask: 0.30, deny: 0.62 }
    }

    fn judged(verdict: Verdict, mode: Mode, display: Display) -> Outcome {
        Outcome {
            gate: "bash-risk".into(),
            mode,
            display,
            command: "rm -rf ~/Downloads/old".into(),
            kind: Kind::Judged { verdict, probs: probs(), result: result() },
            warnings: 0,
        }
    }

    fn message(output: &Option<Value>) -> String {
        output.as_ref().unwrap()["systemMessage"].as_str().unwrap().to_string()
    }

    #[test]
    fn audit_deny_shows_the_reasoning_but_makes_no_permission_decision() {
        let output = hook_output(&judged(Verdict::Deny, Mode::Audit, Display::Decisions));
        let expected = "🛡 decide gate bash-risk: deny (감사 모드 — 막지 않음)\n- 대상: rm -rf ~/Downloads/old\n- 선택: deny 62% · ask 30% · allow 8% · local · clef-flash · 540ms";
        assert_eq!(message(&output), expected);
        assert!(output.unwrap().get("hookSpecificOutput").is_none(), "감사 모드는 판정을 내리면 안 된다");
    }

    #[test]
    fn enforce_deny_and_ask_make_a_permission_decision_with_a_reason() {
        for (verdict, decision) in [(Verdict::Deny, "deny"), (Verdict::Ask, "ask")] {
            let output = hook_output(&judged(verdict, Mode::Enforce, Display::Decisions)).unwrap();
            let specific = &output["hookSpecificOutput"];
            assert_eq!(specific["hookEventName"], "PreToolUse");
            assert_eq!(specific["permissionDecision"], decision);
            let reason = specific["permissionDecisionReason"].as_str().unwrap();
            assert!(reason.contains("decide gate bash-risk") && reason.contains("deny 62%"), "{reason}");
            assert!(output["systemMessage"].as_str().unwrap().contains(decision));
        }
    }

    #[test]
    fn enforce_allow_makes_no_decision() {
        let quiet = hook_output(&judged(Verdict::Allow, Mode::Enforce, Display::Decisions));
        assert!(quiet.is_none(), "판정 없음은 기본적으로 아무것도 내지 않는다");
        let shown = hook_output(&judged(Verdict::Allow, Mode::Enforce, Display::All)).unwrap();
        assert!(shown.get("hookSpecificOutput").is_none());
        assert!(shown["systemMessage"].as_str().unwrap().contains("allow"));
    }

    #[test]
    fn audit_allow_is_silent_unless_everything_is_shown() {
        assert!(hook_output(&judged(Verdict::Allow, Mode::Audit, Display::Decisions)).is_none());
        assert!(hook_output(&judged(Verdict::Allow, Mode::Audit, Display::All)).is_some());
        assert!(hook_output(&judged(Verdict::Allow, Mode::Audit, Display::Off)).is_none());
    }

    #[test]
    fn display_off_hides_the_message_but_enforce_still_decides() {
        let output = hook_output(&judged(Verdict::Deny, Mode::Enforce, Display::Off)).unwrap();
        assert!(output.get("systemMessage").is_none());
        assert_eq!(output["hookSpecificOutput"]["permissionDecision"], "deny");
        assert!(hook_output(&judged(Verdict::Deny, Mode::Audit, Display::Off)).is_none());
    }

    #[test]
    fn a_prefiltered_command_is_only_shown_when_everything_is() {
        let mut outcome = judged(Verdict::Allow, Mode::Audit, Display::Decisions);
        outcome.kind = Kind::Prefiltered;
        outcome.command = "git status".into();
        assert!(hook_output(&outcome).is_none());
        outcome.display = Display::All;
        let text = message(&hook_output(&outcome));
        assert_eq!(text, "🛡 decide gate bash-risk: 사전 필터 통과\n- 대상: git status");
    }

    #[test]
    fn a_failure_passes_through_with_one_line_and_never_decides() {
        let mut outcome = judged(Verdict::Deny, Mode::Enforce, Display::Decisions);
        outcome.kind = Kind::Failed { reason: "데몬 응답 없음".into() };
        let output = hook_output(&outcome).unwrap();
        assert_eq!(output["systemMessage"], "🛡 decide gate bash-risk: 판정 없이 통과 (데몬 응답 없음)");
        assert!(output.get("hookSpecificOutput").is_none(), "실패하면 기본 권한 흐름을 그대로 둔다");
        outcome.display = Display::Off;
        assert!(hook_output(&outcome).is_none());
    }

    #[test]
    fn the_command_is_cut_at_80_characters_and_warnings_are_counted() {
        let mut outcome = judged(Verdict::Ask, Mode::Audit, Display::Decisions);
        outcome.command = format!("echo {}", "가".repeat(200));
        outcome.warnings = 2;
        let text = message(&hook_output(&outcome));
        let target = text.lines().find(|line| line.starts_with("- 대상: ")).unwrap();
        assert_eq!(target.trim_start_matches("- 대상: ").chars().count(), 81, "80자 + …");
        assert!(text.ends_with("- 설정 경고 2건: decide gate --show로 확인"), "{text}");
    }

    #[test]
    fn a_cached_answer_shows_the_cache_marker_instead_of_latency() {
        let mut outcome = judged(Verdict::Ask, Mode::Audit, Display::Decisions);
        outcome.kind = Kind::Judged {
            verdict: Verdict::Ask,
            probs: probs(),
            result: json!({"routing": {"backend": "typesafe", "model": "jev-1.13.0", "cached": true}, "latency_ms": 0.0}),
        };
        assert!(message(&hook_output(&outcome)).ends_with("typesafe · jev-1.13.0 · (캐시)"));
    }

    fn locations(user: bool, repo: bool) -> (Location, Location) {
        (
            Location { label: "~/.config/decide/gates.json".into(), exists: user },
            Location { label: "./.decide/gates.json".into(), exists: repo },
        )
    }

    #[test]
    fn the_overview_lists_the_gate_and_where_its_settings_would_come_from() {
        let (user, repo) = locations(false, false);
        let text = show_overview(&crate::gate::config::load(None, None), &user, &repo);
        assert_eq!(
            text,
            "bash-risk   PreToolUse/Bash   choice   감사 모드   display=decisions\nuser-rules  ~/.config/decide/gates.json (없음)\nrepo-rules  ./.decide/gates.json (없음)"
        );
    }

    #[test]
    fn the_overview_shows_enforce_disabled_and_present_files_and_warnings() {
        let (user, repo) = locations(true, true);
        let loaded = crate::gate::config::load(
            Some(r#"{"mode": "enforce", "display": "all"}"#),
            Some(r#"{"gates": {"bash-risk": {"enabled": false}}, "timeout_ms": 5}"#),
        );
        let text = show_overview(&loaded, &user, &repo);
        assert!(text.contains("choice   enforce   display=all"), "{text}");
        assert!(text.contains("user-rules  ~/.config/decide/gates.json (있음)"), "{text}");
        assert!(text.contains("repo-rules  ./.decide/gates.json (있음)"), "{text}");
        assert!(text.contains("경고: 저장소 설정: timeout_ms"), "{text}");
        let off = crate::gate::config::load(Some(r#"{"gates": {"bash-risk": {"enabled": false}}}"#), None);
        assert!(show_overview(&off, &user, &repo).contains("choice   꺼짐"), "꺼진 게이트는 모드 칸에 꺼짐");
    }

    #[test]
    fn the_gate_detail_shows_the_question_options_thresholds_and_sources() {
        let loaded = crate::gate::config::load(Some(r#"{"gates": {"bash-risk": {"thresholds": {"deny": 0.8}}}}"#), None);
        let text = show_gate(&loaded, "bash-risk", false).unwrap();
        for line in [
            "이벤트:   PreToolUse (matcher: Bash)",
            "타입:     choice",
            &format!("질문:     {QUESTION}"),
            "선택지:   allow — 저장소 안 작업이거나 읽기 전용",
            "          ask   — 영향 범위가 불분명함",
            "          deny  — 저장소 밖을 지우거나 되돌리기 어렵게 바꿈",
            "임계값:   deny ≥ 0.80, 최고 확률 < 0.70이면 ask",
            "모드:     audit",
        ] {
            assert!(text.contains(line), "{line:?}가 없다:\n{text}");
        }
        assert!(text.contains("사전 필터: git status, git diff"), "{text}");
        let count = crate::gate::config::builtin().bash_risk.prefilter.len();
        assert!(text.contains(&format!("({count}개)")), "{text}");
        assert!(text.contains("bash-risk.deny = 0.8 (사용자)"), "{text}");
        assert!(text.contains("mode = audit (내장 기본값)"), "{text}");
    }

    #[test]
    fn the_gate_detail_as_json_has_a_config_that_can_be_copied_into_a_settings_file() {
        let loaded = crate::gate::config::load(Some(r#"{"mode": "enforce"}"#), None);
        let text = show_gate(&loaded, "bash-risk", true).unwrap();
        let value: Value = serde_json::from_str(&text).unwrap();
        assert_eq!(value["gate"], "bash-risk");
        assert_eq!(value["event"], "PreToolUse");
        assert_eq!(value["matcher"], "Bash");
        assert_eq!(value["type"], "choice");
        assert_eq!(value["question"], QUESTION);
        assert_eq!(value["options"]["deny"], "저장소 밖을 지우거나 되돌리기 어렵게 바꿈");
        assert_eq!(value["sources"]["mode"], "사용자");
        assert_eq!(value["sources"]["display"], "내장 기본값");
        let reloaded = crate::gate::config::load(Some(&value["config"].to_string()), None);
        assert_eq!(reloaded.config, loaded.config, "config를 그대로 복사하면 같은 설정이어야 한다");
        assert!(reloaded.warnings.is_empty(), "{:?}", reloaded.warnings);
    }

    #[test]
    fn show_lists_the_static_rules_with_their_sources() {
        let user = r#"{"gates": {"bash-risk": {
            "deny_patterns": ["*mkfs*"], "ask_patterns": ["*clean -fd*", "*.env"]}}}"#;
        let loaded = crate::gate::config::load(Some(user), None);
        let text = show_gate(&loaded, "bash-risk", false).unwrap();
        assert!(text.contains("정적 규칙: deny 1개, ask 2개"), "{text}");
        assert!(text.contains("  deny  *mkfs*"), "{text}");
        assert!(text.contains("  ask   *clean -fd*") && text.contains("  ask   *.env"), "{text}");
        assert!(text.contains("bash-risk.deny_patterns = 1개 (사용자)"), "{text}");
        assert!(text.contains("bash-risk.ask_patterns = 2개 (사용자)"), "{text}");
        let value: Value = serde_json::from_str(&show_gate(&loaded, "bash-risk", true).unwrap()).unwrap();
        assert_eq!(value["config"]["gates"]["bash-risk"]["deny_patterns"], json!(["*mkfs*"]));
        assert_eq!(value["sources"]["bash-risk.ask_patterns"], "사용자");
    }

    #[test]
    fn show_truncates_long_rule_lists_and_the_json_has_them_all() {
        let loaded = crate::gate::config::load(None, None);
        let rules = &loaded.config.bash_risk;
        let text = show_gate(&loaded, "bash-risk", false).unwrap();
        assert_eq!(text.lines().filter(|line| line.starts_with("  deny  ")).count(), 8, "{text}");
        assert_eq!(text.lines().filter(|line| line.starts_with("  ask   ")).count(), 8, "{text}");
        let more_deny = rules.deny_patterns.len() - 8;
        let more_ask = rules.ask_patterns.len() - 8;
        assert!(text.contains(&format!("  … deny 외 {more_deny}개 (--json으로 전체 목록)")), "{text}");
        assert!(text.contains(&format!("  … ask 외 {more_ask}개 (--json으로 전체 목록)")), "{text}");
        let total = rules.prefilter.len();
        assert!(total > 12, "기본 사전 필터가 12개를 넘어야 줄임 표시를 시험할 수 있다");
        assert!(text.contains(&format!(", … 외 {}개 ({total}개)", total - 12)), "{text}");
        let value: Value = serde_json::from_str(&show_gate(&loaded, "bash-risk", true).unwrap()).unwrap();
        let listed = &value["config"]["gates"]["bash-risk"];
        assert_eq!(listed["deny_patterns"].as_array().unwrap().len(), rules.deny_patterns.len());
        assert_eq!(listed["ask_patterns"].as_array().unwrap().len(), rules.ask_patterns.len());
    }

    #[test]
    fn an_unknown_gate_name_has_no_detail() {
        let loaded = crate::gate::config::load(None, None);
        assert!(show_gate(&loaded, "no-such-gate", false).is_none());
        assert!(show_gate(&loaded, "no-such-gate", true).is_none());
    }

    #[test]
    fn the_message_omits_the_fixed_question_every_call_repeats() {
        let text = message(&hook_output(&judged(Verdict::Ask, Mode::Audit, Display::Decisions)));
        assert!(!text.contains(QUESTION), "{text}");
    }

    fn ruled(verdict: Verdict, mode: Mode, display: Display) -> Outcome {
        Outcome {
            gate: "bash-risk".into(),
            mode,
            display,
            command: "sudo mkfs.ext4 /dev/sda".into(),
            kind: Kind::Rule { verdict, pattern: "*mkfs*".into() },
            warnings: 0,
        }
    }

    #[test]
    fn audit_rule_shows_the_pattern_without_a_question_or_probabilities() {
        let output = hook_output(&ruled(Verdict::Deny, Mode::Audit, Display::Decisions));
        let expected = "🛡 decide gate bash-risk: deny (감사 모드 — 막지 않음)\n- 근거: 정적 규칙 `*mkfs*` (모델 호출 없음)\n- 대상: sudo mkfs.ext4 /dev/sda";
        assert_eq!(message(&output), expected);
        assert!(output.unwrap().get("hookSpecificOutput").is_none(), "감사 모드는 판정을 내리면 안 된다");
    }

    #[test]
    fn enforce_rule_makes_a_permission_decision_naming_the_pattern() {
        for (verdict, decision) in [(Verdict::Deny, "deny"), (Verdict::Ask, "ask")] {
            let output = hook_output(&ruled(verdict, Mode::Enforce, Display::Decisions)).unwrap();
            assert_eq!(output["hookSpecificOutput"]["permissionDecision"], decision);
            let reason = output["hookSpecificOutput"]["permissionDecisionReason"].as_str().unwrap();
            assert!(reason.contains("정적 규칙 `*mkfs*`") && reason.contains(decision), "{reason}");
            assert!(output["systemMessage"].as_str().unwrap().contains("enforce"), "{output}");
        }
    }

    #[test]
    fn display_off_hides_the_rule_message_but_enforce_still_decides() {
        assert!(hook_output(&ruled(Verdict::Deny, Mode::Audit, Display::Off)).is_none());
        let output = hook_output(&ruled(Verdict::Deny, Mode::Enforce, Display::Off)).unwrap();
        assert_eq!(output["hookSpecificOutput"]["permissionDecision"], "deny");
        assert!(output.get("systemMessage").is_none(), "{output}");
    }

    #[test]
    fn equal_probabilities_keep_the_strictest_first() {
        let mut outcome = judged(Verdict::Ask, Mode::Audit, Display::Decisions);
        outcome.kind = Kind::Judged {
            verdict: Verdict::Ask,
            probs: Probs { allow: 0.4, ask: 0.4, deny: 0.2 },
            result: result(),
        };
        assert!(message(&hook_output(&outcome)).contains("선택: ask 40% · allow 40% · deny 20%"));
    }
}
