// 게이트 판정을 Claude Code 훅 출력 JSON과 사용자에게 보여 줄 근거 문구로 바꾼다
use crate::gate::bash_risk::{Probs, Verdict, QUESTION};
use crate::gate::config::{Display, Mode};
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

/// 훅이 stdout으로 내보낼 JSON. 내보낼 것이 없으면 `None`이다.
/// 감사 모드는 `systemMessage`만 내고 `permissionDecision`은 절대 내지 않는다.
pub fn hook_output(outcome: &Outcome) -> Option<Value> {
    let decision = match (&outcome.kind, outcome.mode) {
        (Kind::Judged { verdict, probs, .. }, Mode::Enforce) => permission(*verdict, probs, &outcome.gate),
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

/// 사용자에게 보여 줄 근거 문구. 표시 방식(`display`)과 결과 종류에 따라 없을 수 있다.
fn message(outcome: &Outcome) -> Option<String> {
    let head = format!("🛡 decide gate {}:", outcome.gate);
    let command = truncate(&outcome.command);
    let mut text = match &outcome.kind {
        Kind::Prefiltered if outcome.display == Display::All => {
            format!("{head} 사전 필터 통과\n   대상: {command}")
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
                "{head} {} ({})\n   질문: {QUESTION}\n   대상: {command}\n   선택: {}\n   {}",
                verdict_label(*verdict),
                mode_text(outcome.mode, *verdict),
                ranked(probs),
                footer(result),
            )
        }
        _ => return None,
    };
    if outcome.warnings > 0 {
        text.push_str(&format!("\n   설정 경고 {}건: decide gate --show로 확인", outcome.warnings));
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

    const QUESTION_LINE: &str = "   질문: 이 셸 명령은 저장소 밖의 데이터나 상태를 파괴하거나 되돌리기 어렵게 바꾸는가?";

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
        let expected = format!(
            "🛡 decide gate bash-risk: deny (감사 모드 — 막지 않음)\n{QUESTION_LINE}\n   대상: rm -rf ~/Downloads/old\n   선택: deny 62% · ask 30% · allow 8%\n   local · clef-flash · 540ms"
        );
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
        assert_eq!(text, "🛡 decide gate bash-risk: 사전 필터 통과\n   대상: git status");
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
        let target = text.lines().find(|line| line.starts_with("   대상: ")).unwrap();
        assert_eq!(target.trim_start_matches("   대상: ").chars().count(), 81, "80자 + …");
        assert!(text.ends_with("   설정 경고 2건: decide gate --show로 확인"), "{text}");
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

    #[test]
    fn the_message_names_the_question_the_model_was_asked() {
        let text = message(&hook_output(&judged(Verdict::Ask, Mode::Audit, Display::Decisions)));
        assert!(text.contains(QUESTION));
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
