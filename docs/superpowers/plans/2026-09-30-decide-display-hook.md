# decide hook Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Claude Code `PostToolUse` 훅이 `decide` 도구의 질문과 결과를 사용자에게 한 줄 요약(`systemMessage`)으로 보여주도록 `decide hook` 서브커맨드를 추가한다.

**Architecture:** 서식 로직은 순수 함수 `show::render(&Value) -> Option<String>`이고, `main.rs`의 `hook` 서브커맨드는 stdin을 읽어 그 함수를 부르고 결과가 있으면 `{"systemMessage": ...}` 한 줄을 출력한다. 입력을 해석하지 못하면 아무것도 쓰지 않고 종료 코드 0이다(fail-open).

**Tech Stack:** Rust(`serde_json`), 새 의존성 없음.

**Spec:** `docs/superpowers/specs/2026-09-30-decide-display-hook-design.md`

## Global Constraints

- 대상 도구 이름은 정확히 `mcp__decide__decide`다. `decide_many`는 이번 범위 밖이다.
- 훅은 절대 실패하지 않는다. 읽을 수 없는 입력(빈 입력, 깨진 JSON, UTF-8이 아닌 바이트, 다른 도구, `isError`, 필드 누락)은 stdout에 아무것도 쓰지 않고 종료 코드 0이다. 패닉 경로가 없어야 한다(`unwrap`과 슬라이스 인덱싱 금지).
- 출력은 정확히 JSON 한 줄이다. 메시지 안의 줄바꿈은 JSON 이스케이프(`\n`)로 들어가 물리적으로 한 줄이다.
- 확률은 정수 %로 반올림한다. 질문은 80자(문자 수, 바이트 아님)를 넘으면 앞 80자에 `…`를 붙인다. choice의 나머지 후보는 확률 내림차순 최대 3개다.
- 새 파일의 첫 줄은 한 줄짜리 한국어 역할 주석이다. 새 의존성을 추가하지 않는다.
- `cargo fmt`를 저장소 전체에 실행하지 않는다(baseline이 fmt 미준수라 무관한 코드가 바뀐다). 새 코드는 rustfmt 스타일로 직접 맞춘다.
- 모든 명령은 저장소 루트에서 `--manifest-path crates/decide/Cargo.toml`로 실행한다. 기준선은 `main`에서 `cargo test`가 통과하는 것이다. 작업 전에 기준 테스트 수를 기록해 두고, 끝에서 이 계획이 추가한 테스트만큼 늘었는지 확인한다.
- 커밋 메시지 끝에는 `Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>` 줄을 붙인다.
- 머지 충돌 주의: PR #13(decide_many)도 README, CHANGELOG `[Unreleased]`, CLAUDE.md, `lib.rs`를 건드린다. 이 계획의 README 절은 `## 개발` 앞에, CLAUDE.md 줄은 `main.rs` 항목 뒤에 넣어 겹치지 않게 한다. CHANGELOG는 `## [Unreleased]`가 이미 있으면 그 `### 추가` 아래에 덧붙이고 없으면 만든다. PR을 열기 직전에 최신 `main`을 합치고 충돌이 나면 두 `[Unreleased]` 항목을 하나로 합친다.

## Review Focus

- `tool_response`가 문자열인 경우와 객체(`structuredContent`, `content[0].text`)인 경우가 같은 결과를 내야 한다. (Task 1)
- choice의 `choice` 라벨이 `probabilities`에 없으면 조용히 종료해야 한다. (Task 1)
- 긴 한글 질문을 바이트가 아니라 문자 수로 자르고, 이모지나 한글이 잘린 글자로 깨지지 않아야 한다. (Task 1)
- 확률이 같거나 NaN에 가까운 값이 있어도 패닉 없이 정렬되어야 한다. (Task 1)
- 캐시된 결과(`routing.cached == true`, `latency_ms == 0.0`)는 지연 대신 `(캐시)`를 보여야 한다. (Task 1)
- 빈 stdin, 깨진 JSON, UTF-8이 아닌 바이트 입력에도 종료 코드 0이고 stdout이 비어야 한다. (Task 2)
- 출력이 정확히 한 줄의 JSON이고 그 안의 `systemMessage`가 여러 줄 문자열이어야 한다. (Task 2)
- `decide hook`에 인자를 더 주면 기존 규칙대로 종료 코드 2여야 하고 도움말에 `hook`이 보여야 한다. (Task 2)

---

### Task 1: show — 훅 입력을 한 줄 요약으로

**Files:**
- Create: `crates/decide/src/show.rs`
- Modify: `crates/decide/src/lib.rs` (`pub mod show;` 한 줄)

**Interfaces:**
- Consumes: 없음(`serde_json::Value`만)
- Produces:
  - `pub const TOOL_NAME: &str = "mcp__decide__decide"`
  - `pub fn render(input: &Value) -> Option<String>` — `PostToolUse` 훅 입력 JSON 전체를 받아, 사용자에게 보일 여러 줄 문자열 또는 `None`(조용히 종료)을 돌려준다.

- [ ] **Step 1: Write the failing tests**

먼저 기준 테스트 수를 기록한다.

Run: `cargo test --manifest-path crates/decide/Cargo.toml 2>&1 | grep "test result" | awk '{p+=$4} END{print p}'`
Expected: 현재 통과 개수(예: 32). 이 값을 `BASELINE`으로 기록한다.

`crates/decide/src/show.rs`를 새로 만든다. 지금은 헤더 주석, `use`, 테스트만 둔다.

```rust
// PostToolUse 훅 입력에서 decide의 질문과 결과를 사용자용 요약으로 만든다
use serde_json::Value;

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const CHOICE: &str = r#"{"answer":{"type":"choice","choice":"푸시하고 PR 생성","confidence":0.94,"probabilities":{"main에 로컬로 머지":0.01,"푸시하고 PR 생성":0.96,"브랜치를 그대로 유지":0.03,"작업을 폐기":0}},"routing":{"backend":"typesafe","model":"jev-1.13.0"},"latency_ms":203.4905}"#;
    const NOUL: &str = r#"{"answer":{"type":"noul","noul":0.56},"routing":{"backend":"typesafe","model":"jev-1.13.0"},"latency_ms":460.36216700000006}"#;
    const SCORE: &str = r#"{"answer":{"type":"score","score":1.17,"confidence":0.85,"legend":{"0":"거의 없음","1":"낮음","2":"보통","3":"높음","4":"매우 높음"},"probabilities":{"0":0.01,"1":0.83,"2":0.15,"3":0.01,"4":0}},"routing":{"backend":"typesafe","model":"jev-1.13.0"},"latency_ms":181.45366700000002}"#;

    const CHOICE_TEXT: &str = "🔎 decide가 선택했습니다: \"푸시하고 PR 생성\" (96%)\n   질문: 이 브랜치를 마무리하는 가장 적절한 방법은?\n   나머지: 브랜치를 그대로 유지 3% · main에 로컬로 머지 1% · 작업을 폐기 0%\n   typesafe · jev-1.13.0 · 203ms";
    const NOUL_TEXT: &str = "🔎 decide 판단: 이 변경은 머지해도 될 만큼 검증되었는가? → 56%\n   typesafe · jev-1.13.0 · 460ms";
    const SCORE_TEXT: &str = "🔎 decide 점수: 남아 있는 위험의 크기는? → 기대값 1.17/4, 가장 가능성 높은 등급 \"낮음\" 83%\n   typesafe · jev-1.13.0 · 181ms";

    fn hook_input(instructions: &str, response: Value) -> Value {
        json!({
            "hook_event_name": "PostToolUse",
            "tool_name": TOOL_NAME,
            "tool_input": {"state": "s", "type": "x", "instructions": instructions},
            "tool_response": response
        })
    }

    fn text(raw: &str) -> Value {
        Value::String(raw.to_string())
    }

    #[test]
    fn renders_the_three_types_exactly() {
        let choice = hook_input("이 브랜치를 마무리하는 가장 적절한 방법은?", text(CHOICE));
        assert_eq!(render(&choice).unwrap(), CHOICE_TEXT);
        let noul = hook_input("이 변경은 머지해도 될 만큼 검증되었는가?", text(NOUL));
        assert_eq!(render(&noul).unwrap(), NOUL_TEXT);
        let score = hook_input("남아 있는 위험의 크기는?", text(SCORE));
        assert_eq!(render(&score).unwrap(), SCORE_TEXT);
    }

    #[test]
    fn accepts_structured_content_and_content_text() {
        let question = "이 변경은 머지해도 될 만큼 검증되었는가?";
        let structured = json!({
            "content": [{"type": "text", "text": "무시된다"}],
            "structuredContent": serde_json::from_str::<Value>(NOUL).unwrap(),
            "isError": false
        });
        assert_eq!(render(&hook_input(question, structured)).unwrap(), NOUL_TEXT);
        let content_only = json!({
            "content": [{"type": "text", "text": NOUL}],
            "isError": false
        });
        assert_eq!(render(&hook_input(question, content_only)).unwrap(), NOUL_TEXT);
    }

    #[test]
    fn stays_silent_for_anything_it_cannot_read() {
        let ok = hook_input("q", text(NOUL));
        let mut other_tool = ok.clone();
        other_tool["tool_name"] = json!("mcp__other__tool");
        assert_eq!(render(&other_tool), None);
        assert_eq!(render(&hook_input("q", text("not json"))), None);
        assert_eq!(render(&hook_input("q", text(r#"{"routing":{}}"#))), None);
        assert_eq!(
            render(&hook_input("q", text(r#"{"answer":{"type":"mystery"}}"#))),
            None
        );
        assert_eq!(
            render(&hook_input(
                "q",
                json!({"content": [{"type": "text", "text": "오류"}], "isError": true})
            )),
            None
        );
        assert_eq!(render(&hook_input("q", json!({}))), None);
        assert_eq!(render(&hook_input("q", json!(42))), None);
        let label_missing = r#"{"answer":{"type":"choice","choice":"x","probabilities":{"a":1.0}}}"#;
        assert_eq!(render(&hook_input("q", text(label_missing))), None);
        let mut no_tool_input = ok.clone();
        no_tool_input.as_object_mut().unwrap().remove("tool_input");
        assert_eq!(render(&no_tool_input), None);
        assert_eq!(render(&json!({})), None);
        assert_eq!(render(&json!("text")), None);
    }

    #[test]
    fn truncates_a_long_question_by_characters_and_limits_the_rest_to_three() {
        let long = "가".repeat(100);
        let out = render(&hook_input(&long, text(NOUL))).unwrap();
        assert!(out.contains(&format!("{}…", "가".repeat(80))));
        assert!(!out.contains(&"가".repeat(81)));

        let five = r#"{"answer":{"type":"choice","choice":"a","probabilities":{"a":0.5,"b":0.2,"c":0.15,"d":0.1,"e":0.05}},"routing":{"backend":"local","model":"m"},"latency_ms":10.0}"#;
        let out = render(&hook_input("q", text(five))).unwrap();
        assert!(out.contains("나머지: b 20% · c 15% · d 10%"));
        assert!(!out.contains("e 5%"));
    }

    #[test]
    fn equal_probabilities_do_not_panic_and_keep_input_order() {
        let tie = r#"{"answer":{"type":"choice","choice":"a","probabilities":{"a":0.4,"b":0.3,"c":0.3}},"routing":{"backend":"local","model":"m"},"latency_ms":1.0}"#;
        let out = render(&hook_input("q", text(tie))).unwrap();
        assert!(out.contains("나머지: b 30% · c 30%"));
        let score_tie = r#"{"answer":{"type":"score","score":0.5,"legend":{"0":"낮음","1":"높음"},"probabilities":{"0":0.5,"1":0.5}},"routing":{"backend":"local","model":"m"},"latency_ms":1.0}"#;
        assert!(render(&hook_input("q", text(score_tie))).is_some());
    }

    #[test]
    fn cached_results_show_a_marker_instead_of_latency() {
        let cached = r#"{"answer":{"type":"noul","noul":0.56},"routing":{"backend":"typesafe","model":"jev-1.13.0","cached":true},"latency_ms":0.0}"#;
        assert_eq!(
            render(&hook_input("q", text(cached))).unwrap(),
            "🔎 decide 판단: q → 56%\n   typesafe · jev-1.13.0 · (캐시)"
        );
    }
}
```

`crates/decide/src/lib.rs`의 `pub mod protocol;`와 `pub mod typesafe;` 사이에 한 줄을 추가한다.

```rust
pub mod show;
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test --manifest-path crates/decide/Cargo.toml show::`
Expected: 컴파일 오류 `cannot find function render in this scope`와 `cannot find value TOOL_NAME`.

- [ ] **Step 3: Write minimal implementation**

`crates/decide/src/show.rs`에서 `use serde_json::Value;` 줄을 아래 블록 전체로 바꾼다(테스트 모듈은 그대로 둔다).

```rust
use serde_json::Value;
use std::cmp::Ordering;

pub const TOOL_NAME: &str = "mcp__decide__decide";
const QUESTION_MAX_CHARS: usize = 80;
const REST_MAX: usize = 3;

pub fn render(input: &Value) -> Option<String> {
    if input.get("tool_name")?.as_str()? != TOOL_NAME {
        return None;
    }
    let result = result_json(input.get("tool_response")?)?;
    let answer = result.get("answer")?;
    let question = truncate(input.get("tool_input")?.get("instructions")?.as_str()?);
    let head = match answer.get("type")?.as_str()? {
        "choice" => choice_text(answer, &question)?,
        "noul" => format!(
            "🔎 decide 판단: {question} → {}%",
            pct(answer.get("noul")?.as_f64()?)
        ),
        "score" => score_text(answer, &question)?,
        _ => return None,
    };
    Some(format!("{head}\n   {}", footer(&result)))
}

fn result_json(response: &Value) -> Option<Value> {
    match response {
        Value::String(text) => serde_json::from_str(text).ok(),
        Value::Object(object) => {
            if object.get("isError").and_then(Value::as_bool) == Some(true) {
                return None;
            }
            if let Some(structured) = object.get("structuredContent") {
                return Some(structured.clone());
            }
            let text = object.get("content")?.get(0)?.get("text")?.as_str()?;
            serde_json::from_str(text).ok()
        }
        _ => None,
    }
}

fn truncate(question: &str) -> String {
    if question.chars().count() <= QUESTION_MAX_CHARS {
        return question.to_string();
    }
    let head: String = question.chars().take(QUESTION_MAX_CHARS).collect();
    format!("{head}…")
}

fn pct(probability: f64) -> i64 {
    (probability * 100.0).round() as i64
}

fn choice_text(answer: &Value, question: &str) -> Option<String> {
    let choice = answer.get("choice")?.as_str()?;
    let probabilities = answer.get("probabilities")?.as_object()?;
    let chosen = probabilities.get(choice)?.as_f64()?;
    let mut rest: Vec<(&str, f64)> = probabilities
        .iter()
        .filter(|(label, _)| label.as_str() != choice)
        .filter_map(|(label, value)| Some((label.as_str(), value.as_f64()?)))
        .collect();
    rest.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(Ordering::Equal));
    let others = rest
        .iter()
        .take(REST_MAX)
        .map(|(label, probability)| format!("{label} {}%", pct(*probability)))
        .collect::<Vec<_>>()
        .join(" · ");
    let mut out = format!(
        "🔎 decide가 선택했습니다: \"{choice}\" ({}%)\n   질문: {question}",
        pct(chosen)
    );
    if !others.is_empty() {
        out.push_str(&format!("\n   나머지: {others}"));
    }
    Some(out)
}

fn score_text(answer: &Value, question: &str) -> Option<String> {
    let score = answer.get("score")?.as_f64()?;
    let probabilities = answer.get("probabilities")?.as_object()?;
    let legend = answer.get("legend")?.as_object()?;
    let (top_key, top_probability) = probabilities
        .iter()
        .filter_map(|(key, value)| Some((key.as_str(), value.as_f64()?)))
        .max_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(Ordering::Equal))?;
    let label = legend.get(top_key)?.as_str()?;
    let max = legend.len().saturating_sub(1);
    Some(format!(
        "🔎 decide 점수: {question} → 기대값 {score:.2}/{max}, 가장 가능성 높은 등급 \"{label}\" {}%",
        pct(top_probability)
    ))
}

fn footer(result: &Value) -> String {
    let routing = result.get("routing");
    let field = |key: &str| {
        routing
            .and_then(|routing| routing.get(key))
            .and_then(Value::as_str)
            .unwrap_or("?")
    };
    let cached = routing
        .and_then(|routing| routing.get("cached"))
        .and_then(Value::as_bool)
        == Some(true);
    let tail = if cached {
        "(캐시)".to_string()
    } else {
        match result.get("latency_ms").and_then(Value::as_f64) {
            Some(ms) => format!("{}ms", ms.round() as i64),
            None => "?".to_string(),
        }
    };
    format!("{} · {} · {tail}", field("backend"), field("model"))
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test --manifest-path crates/decide/Cargo.toml`
Expected: 새 `show::` 테스트 6개를 포함해 전체 PASS, 경고 없음.

- [ ] **Step 5: Commit**

```bash
git add crates/decide/src/show.rs crates/decide/src/lib.rs
git commit -m "feat(decide): PostToolUse 입력을 사용자용 한 줄 요약으로 만드는 show 모듈을 추가한다

Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>"
```

---

### Task 2: `decide hook` 서브커맨드

**Files:**
- Modify: `crates/decide/src/main.rs` (`use` 한 줄, `hook` 분기, `run_hook`, 도움말 두 곳)
- Create: `crates/decide/tests/hook.rs`

**Interfaces:**
- Consumes: Task 1의 `decide::show::render`
- Produces: `decide hook` — stdin의 훅 입력 JSON을 읽어 `{"systemMessage": "<요약>"}` 한 줄을 stdout에 쓴다. 읽을 수 없으면 아무것도 쓰지 않고 종료 코드 0이다. 인자가 더 있으면 종료 코드 2(기존 `reject_extra`).

- [ ] **Step 1: Write the failing tests**

`crates/decide/tests/hook.rs`를 새로 만든다.

```rust
// decide hook 서브커맨드가 훅 입력 JSON을 systemMessage 한 줄로 바꾸는지 검사한다
use serde_json::{json, Value};
use std::io::Write;
use std::process::{Command, Stdio};

const NOUL_INPUT: &str = r#"{"hook_event_name":"PostToolUse","tool_name":"mcp__decide__decide","tool_input":{"state":"s","type":"noul","instructions":"이 변경은 머지해도 될 만큼 검증되었는가?"},"tool_response":"{\"answer\":{\"type\":\"noul\",\"noul\":0.56},\"routing\":{\"backend\":\"typesafe\",\"model\":\"jev-1.13.0\"},\"latency_ms\":460.36216700000006}"}"#;

fn run_hook(stdin: &[u8]) -> (Option<i32>, String) {
    let mut child = Command::new(env!("CARGO_BIN_EXE_decide"))
        .arg("hook")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child.stdin.take().unwrap().write_all(stdin).unwrap();
    let output = child.wait_with_output().unwrap();
    (
        output.status.code(),
        String::from_utf8(output.stdout).unwrap(),
    )
}

#[test]
fn hook_prints_exactly_one_json_line_with_a_multiline_message() {
    let (code, stdout) = run_hook(NOUL_INPUT.as_bytes());
    assert_eq!(code, Some(0));
    assert!(stdout.ends_with('\n'));
    assert_eq!(stdout.trim_end().lines().count(), 1, "{stdout}");
    let parsed: Value = serde_json::from_str(stdout.trim_end()).unwrap();
    assert_eq!(
        parsed,
        json!({"systemMessage": "🔎 decide 판단: 이 변경은 머지해도 될 만큼 검증되었는가? → 56%\n   typesafe · jev-1.13.0 · 460ms"})
    );
}

#[test]
fn hook_is_silent_and_succeeds_on_unreadable_input() {
    for input in [
        &b""[..],
        &b"not json"[..],
        &b"{\"tool_name\":\"mcp__other__tool\"}"[..],
        &[0xff, 0xfe, 0xfd][..],
    ] {
        let (code, stdout) = run_hook(input);
        assert_eq!(code, Some(0), "입력: {input:?}");
        assert_eq!(stdout, "", "입력: {input:?}");
    }
}

#[test]
fn help_lists_hook_and_extra_arguments_exit_two() {
    let help = Command::new(env!("CARGO_BIN_EXE_decide"))
        .arg("--help")
        .output()
        .unwrap();
    assert!(String::from_utf8_lossy(&help.stdout).contains("hook"));
    let extra = Command::new(env!("CARGO_BIN_EXE_decide"))
        .args(["hook", "extra"])
        .output()
        .unwrap();
    assert_eq!(extra.status.code(), Some(2));
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test --manifest-path crates/decide/Cargo.toml --test hook`
Expected: 세 테스트 모두 FAIL. `hook`은 아직 알 수 없는 명령이라 종료 코드가 2이고(`알 수 없는 명령입니다: hook`), 도움말에도 `hook`이 없다.

- [ ] **Step 3: Write minimal implementation**

`crates/decide/src/main.rs`의 `use std::io::{self, BufRead, Write};`를 바꾼다.

```rust
use std::io::{self, BufRead, Read, Write};
```

`match` 안, `Some("install") => { ... }` 분기 바로 뒤(`Some(other) =>` 앞)에 추가한다.

```rust
        Some("hook") => {
            reject_extra(args.next());
            run_hook();
        }
```

`fn reject_extra(` 바로 앞에 함수를 추가한다.

```rust
fn run_hook() {
    let mut input = String::new();
    if io::stdin().read_to_string(&mut input).is_err() {
        return;
    }
    let Ok(value) = serde_json::from_str::<Value>(&input) else {
        return;
    };
    if let Some(text) = decide::show::render(&value) {
        println!("{}", serde_json::json!({ "systemMessage": text }));
    }
}
```

도움말 텍스트 두 곳을 바꾼다. 첫 줄 `decide [mcp|daemon|install]`을

```text
decide [mcp|daemon|install|hook]
```

로 바꾸고, `  install  claude mcp add로 ...` 줄 바로 다음에 한 줄을 추가한다.

```text
  hook     Claude Code PostToolUse 훅. decide 결과를 사용자에게 한 줄로 보여준다
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test --manifest-path crates/decide/Cargo.toml`
Expected: 새 `hook` 통합 테스트 3개를 포함해 전체 PASS. 기존 `help_exits_zero_and_unknown_arguments_exit_two`(도움말에 `mcp`, `daemon`, `install` 포함)도 수정 없이 통과.

- [ ] **Step 5: Commit**

```bash
git add crates/decide/src/main.rs crates/decide/tests/hook.rs
git commit -m "feat(decide): decide hook 서브커맨드로 훅 입력을 systemMessage 한 줄로 바꾼다

Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>"
```

---

### Task 3: 문서와 실세션 확인 안내

**Files:**
- Modify: `README.md` (`## 개발` 앞에 절 추가)
- Modify: `CHANGELOG.md` (`[Unreleased]`)
- Modify: `CLAUDE.md` (Architecture에 한 줄, `main.rs` 항목 뒤)

**Interfaces:**
- Consumes: Task 1, 2의 동작
- Produces: 사용자 문서. 실세션 확인은 사용자가 설정에 훅을 넣어야 해서 이 계획에서 대신 실행하지 않는다.

- [ ] **Step 1: README에 절 추가**

`README.md`의 `## 개발` 줄 바로 앞에 다음을 추가한다.

````markdown
## decide가 고른 것을 눈으로 보기

에이전트가 `decide`를 부르면 결과는 도구 결과 안에 JSON으로만 있다. `decide hook`은 Claude Code `PostToolUse` 훅으로 붙어 질문과 결과를 한 줄 요약으로 사용자에게 보여준다. 이 요약은 사용자에게만 보이고 모델은 보지 않는다.

```text
🔎 decide가 선택했습니다: "푸시하고 PR 생성" (96%)
   질문: 이 브랜치를 마무리하는 가장 적절한 방법은?
   나머지: 브랜치를 그대로 유지 3% · main에 로컬로 머지 1% · 작업을 폐기 0%
   typesafe · jev-1.13.0 · 203ms
```

`~/.claude/settings.json`(또는 프로젝트 설정)에 다음을 넣고 세션을 다시 연다.

```json
{
  "hooks": {
    "PostToolUse": [
      {
        "matcher": "mcp__decide__decide",
        "hooks": [{"type": "command", "command": "/opt/homebrew/bin/decide hook", "timeout": 5}]
      }
    ]
  }
}
```

끄려면 이 항목을 지운다. 읽을 수 없는 입력에는 아무것도 출력하지 않고 종료 코드 0이라 훅이 에이전트 작업을 막지 않는다. `decide_many`의 표시는 아직 없다. 훅은 `decide` 바이너리에 들어 있어서 이 기능이 들어간 릴리스 이후 버전에서만 동작한다.
````

- [ ] **Step 2: CHANGELOG와 CLAUDE.md**

`CHANGELOG.md`에 `## [Unreleased]`가 없으면 `## [0.0.4] - 2026-09-30` 줄 바로 위에 만들고, 있으면 그 `### 추가` 아래에 항목만 덧붙인다.

```markdown
## [Unreleased]

### 추가

- `decide hook` 서브커맨드: Claude Code `PostToolUse` 훅 입력(`mcp__decide__decide`)을 읽어 질문, 선택과
  확률, 백엔드·모델·지연을 `systemMessage` 한 줄 요약으로 출력한다. 읽을 수 없는 입력에는 아무것도
  쓰지 않고 종료 코드 0이다.
```

`CLAUDE.md`의 Architecture 목록에서 `main.rs` 항목(`- \`main.rs\` routes ...`으로 시작하는 줄과 그 이어지는 줄들) 바로 뒤에 추가한다.

```markdown
- `show.rs` turns a `PostToolUse` hook input for `mcp__decide__decide` into a user-facing summary
  (`render(&Value) -> Option<String>`, pure). `decide hook` (`main.rs`) reads stdin, prints
  `{"systemMessage": ...}` as one JSON line, and stays silent with exit 0 on anything unreadable.
```

- [ ] **Step 3: 전체 테스트와 커밋**

Run: `cargo test --manifest-path crates/decide/Cargo.toml`
Expected: 전체 PASS. 통과 개수가 `BASELINE`보다 9개 늘었는지(Task 1의 6개 + Task 2의 3개) 확인한다.

```bash
git add README.md CHANGELOG.md CLAUDE.md
git commit -m "docs(decide): decide hook 사용법과 변경 내역을 문서화한다

Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>"
```

- [ ] **Step 4: 실세션 확인 안내(사용자 실행)**

`cargo build --manifest-path crates/decide/Cargo.toml` 뒤, 아래 스니펫의 `command`를 릴리스 전에는 빌드한 바이너리 경로(`<worktree>/crates/decide/target/debug/decide hook`)로 바꿔 `~/.claude/settings.json`에 넣고 새 세션에서 `decide`를 부르는 프롬프트를 실행한다. 확인할 것은 세 가지다: 한 줄 요약이 실제로 보이는지, 모델이 그 줄을 인용하지 않는지, 훅이 도구 호출을 지연시키지 않는지. 이 단계는 설정 변경이라 제가 대신 하지 않는다. 결과를 받아 `context-notes.md`에 기록한다.
