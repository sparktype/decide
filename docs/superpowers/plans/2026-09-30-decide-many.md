# decide_many Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 한 state에 질문 여러 개를 한 번의 백엔드 호출로 보내는 MCP 도구 `decide_many`와 같은 모양의 데몬 소켓 줄 형식을 추가한다.

**Architecture:** 요청 본문이 이미 `questions` 맵이라 백엔드 전송 계층은 그대로 쓴다. `protocol`이 질문 맵을 입력 순서대로 파싱·검증하고, `typesafe`가 본문 생성과 응답 매핑을 여러 질문으로 일반화하고, `backend::decide_many`가 기존 `decide`와 같은 흐름을 탄다. `mcp`와 `daemon`은 이름과 키로 분기만 한다. 기존 `decide`의 동작은 바뀌지 않는다.

**Tech Stack:** Rust(`serde_json` `preserve_order`), 새 의존성 없음.

**Spec:** `docs/superpowers/specs/2026-09-30-decide-many-design.md`

## Global Constraints

- 기존 `decide` 도구의 인자, 응답, 질문 id `"q"`, 기존 줄 형식은 바이트 단위로 그대로다. 기존 테스트는 수정 없이 통과해야 한다. 단, `daemon.rs`의 `Cache`와 `call`은 내부 타입이 바뀌므로 그 내부 타입을 쓰는 테스트 코드만 고친다.
- 한글 오류 문장은 기존 것을 그대로 재사용하고, 질문 오류에는 앞에 `질문 "<id>": `를 붙인다.
- 전부 성공하거나 전부 실패한다. 부분 결과는 없다. 질문 개수 상한은 두지 않는다.
- TypeSafe 한도(choice 255, score 10)는 질문마다 검사하고, 로컬은 클라이언트가 검사하지 않는다.
- 새 파일을 만들지 않는다(기존 파일 수정만). 새 의존성을 추가하지 않는다.
- `cargo fmt`를 저장소 전체에 실행하지 않는다(baseline이 fmt 미준수라 무관한 코드가 바뀐다). 새로 쓴 코드는 rustfmt 스타일로 직접 맞춘다.
- 모든 명령은 저장소 루트에서 `--manifest-path crates/decide/Cargo.toml`로 실행한다. 기준선은 `cargo test`가 32건 통과다.
- 커밋 메시지 끝에는 `Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>` 줄을 붙인다.

## Review Focus

- `questions`가 객체가 아니라 배열(또는 null)로 오면 `questions는 객체여야 합니다`로 거절해야 한다. (Task 1)
- 질문 값이 객체가 아니거나 id가 빈 문자열이면 id를 밝힌 오류로 거절해야 한다. (Task 1)
- 한글이나 공백이 든 질문 id도 그대로 본문 키와 응답 키가 되어야 한다. (Task 2)
- 백엔드 응답의 `answers`가 없거나 객체가 아니거나, 요청한 id가 빠져 있으면 전체 실패, 요청하지 않은 여분 id는 무시해야 한다. (Task 2)
- 질문이 100개여도 입력 순서가 응답에서 유지되고 개수 제한이 걸리지 않아야 한다. (Task 3)
- 같은 질문을 다른 순서로 보내면 캐시에서 다른 항목으로 취급된다(순서도 요청의 일부). 동일 요청은 적중해야 한다. (Task 5)
- 데몬 줄에 `type`과 `questions`를 함께 주면 전송 0회로 거절해야 한다. (Task 5)
- `tools/list`에서 `decide`의 스키마는 그대로이고 `decide_many`가 추가되어야 한다. (Task 4)

---

### Task 1: protocol — 질문 맵 파싱과 검증

**Files:**
- Modify: `crates/decide/src/protocol.rs` (타입·함수 추가, 테스트 추가)

**Interfaces:**
- Consumes: 기존 `parse_arguments(&Value) -> Result<Incoming, String>`, `validate(&Incoming) -> Result<Question, String>`
- Produces:
  - `pub struct IncomingMany { pub state: String, pub questions: Vec<(String, Incoming)> }` (`Debug, Clone, PartialEq, Eq, Hash`). 각 `Incoming.state`는 공통 state와 같다.
  - `pub struct DecideManyResult { pub answers: Value, pub routing: Value, pub latency_ms: f64 }` (`Debug, Clone, PartialEq, Serialize`)
  - `pub fn parse_many(value: &Value) -> Result<IncomingMany, String>`
  - `pub fn validate_many(req: &IncomingMany) -> Result<Vec<(String, Question)>, String>`
  - 상수 `QUESTIONS_EMPTY`, `QUESTIONS_NOT_OBJECT`, `QUESTION_ID_EMPTY`

- [ ] **Step 1: Write the failing tests**

`crates/decide/src/protocol.rs`의 `#[cfg(test)] mod tests` 안, 기존 `choice_requires_two_distinct_options` 테스트 다음에 추가한다.

```rust
    fn many(value: Value) -> Result<IncomingMany, String> {
        parse_many(&value)
    }

    #[test]
    fn many_parses_questions_in_input_order_with_the_shared_state() {
        let parsed = many(json!({
            "state": "서버 다운",
            "questions": {
                "urgent": {"type": "noul", "instructions": "긴급한가?"},
                "team": {"type": "choice", "instructions": "어느 팀?", "options": ["billing", "infra"]},
                "anger": {"type": "score", "instructions": "불만?", "criteria": ["낮음", "높음"]}
            }
        }))
        .unwrap();
        assert_eq!(parsed.state, "서버 다운");
        let ids: Vec<&str> = parsed.questions.iter().map(|(id, _)| id.as_str()).collect();
        assert_eq!(ids, vec!["urgent", "team", "anger"]);
        assert!(parsed.questions.iter().all(|(_, q)| q.state == "서버 다운"));
        assert_eq!(parsed.questions[1].1.kind, Kind::Choice);
        assert_eq!(parsed.questions[1].1.options, vec!["billing", "infra"]);
    }

    #[test]
    fn many_rejects_bad_shapes() {
        assert_eq!(
            many(json!({"state": "s", "questions": []})).unwrap_err(),
            QUESTIONS_NOT_OBJECT
        );
        assert_eq!(
            many(json!({"state": "s", "questions": null})).unwrap_err(),
            QUESTIONS_NOT_OBJECT
        );
        assert_eq!(
            many(json!({"state": "s", "questions": {}})).unwrap_err(),
            QUESTIONS_EMPTY
        );
        assert_eq!(
            many(json!({"state": "s"})).unwrap_err(),
            "questions가 필요합니다"
        );
        assert_eq!(
            many(json!({"questions": {"a": {"type": "noul", "instructions": "?"}}})).unwrap_err(),
            "state가 필요합니다"
        );
        assert_eq!(
            many(json!({"state": 1, "questions": {"a": {"type": "noul", "instructions": "?"}}}))
                .unwrap_err(),
            "state는 문자열이어야 합니다"
        );
        assert_eq!(
            many(json!({"state": "s", "questions": {"": {"type": "noul", "instructions": "?"}}}))
                .unwrap_err(),
            QUESTION_ID_EMPTY
        );
        assert_eq!(
            many(json!({"state": "s", "questions": {"a": "noul"}})).unwrap_err(),
            "질문 \"a\": 질문은 객체여야 합니다"
        );
        assert_eq!(
            many(json!({"state": "s", "questions": {"a": {"type": "noul"}}})).unwrap_err(),
            "질문 \"a\": instructions가 필요합니다"
        );
    }

    #[test]
    fn many_validation_reuses_the_single_question_messages_with_the_id() {
        let bad_choice = many(json!({
            "state": "s",
            "questions": {
                "ok": {"type": "noul", "instructions": "?"},
                "team": {"type": "choice", "instructions": "?", "options": ["a"]}
            }
        }))
        .unwrap();
        assert_eq!(
            validate_many(&bad_choice).unwrap_err(),
            format!("질문 \"team\": {CHOICE_TOO_FEW}")
        );
        let bad_noul = many(json!({
            "state": "s",
            "questions": {"u": {"type": "noul", "instructions": "?", "options": ["a"]}}
        }))
        .unwrap();
        assert_eq!(
            validate_many(&bad_noul).unwrap_err(),
            format!("질문 \"u\": {NOUL_EXTRA}")
        );
        let good = many(json!({
            "state": "s",
            "questions": {
                "a": {"type": "noul", "instructions": "?"},
                "b": {"type": "score", "instructions": "?", "criteria": ["x", "y"]}
            }
        }))
        .unwrap();
        let questions = validate_many(&good).unwrap();
        assert_eq!(questions.len(), 2);
        assert_eq!(questions[0].0, "a");
        assert!(matches!(questions[1].1, Question::Score { .. }));
    }
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test --manifest-path crates/decide/Cargo.toml protocol::tests::many`
Expected: 컴파일 오류 `cannot find function parse_many`(또는 `IncomingMany` 미정의).

- [ ] **Step 3: Write minimal implementation**

`crates/decide/src/protocol.rs`에서 `pub struct DecideResult { ... }` 정의 바로 다음에 타입과 상수를 추가한다.

```rust
pub const QUESTIONS_EMPTY: &str = "questions는 비어 있을 수 없습니다";
pub const QUESTIONS_NOT_OBJECT: &str = "questions는 객체여야 합니다";
pub const QUESTION_ID_EMPTY: &str = "질문 id는 비어 있을 수 없습니다";

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct IncomingMany {
    pub state: String,
    pub questions: Vec<(String, Incoming)>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct DecideManyResult {
    pub answers: Value,
    pub routing: Value,
    pub latency_ms: f64,
}
```

`pub fn validate(...)` 함수 정의 바로 앞에 두 함수를 추가한다(`parse_arguments`와 `validate` 사이가 아니라 `validate` 뒤여도 무방하다. 여기서는 `validate` 뒤에 둔다).

```rust
pub fn parse_many(value: &Value) -> Result<IncomingMany, String> {
    let obj = value
        .as_object()
        .ok_or_else(|| "요청은 JSON 객체여야 합니다".to_string())?;
    let state = match obj.get("state") {
        Some(Value::String(s)) => s.clone(),
        Some(_) => return Err("state는 문자열이어야 합니다".to_string()),
        None => return Err("state가 필요합니다".to_string()),
    };
    let map = match obj.get("questions") {
        Some(Value::Object(map)) => map,
        Some(_) => return Err(QUESTIONS_NOT_OBJECT.to_string()),
        None => return Err("questions가 필요합니다".to_string()),
    };
    if map.is_empty() {
        return Err(QUESTIONS_EMPTY.to_string());
    }
    let mut questions = Vec::with_capacity(map.len());
    for (id, question) in map {
        if id.is_empty() {
            return Err(QUESTION_ID_EMPTY.to_string());
        }
        let mut args = match question {
            Value::Object(args) => args.clone(),
            _ => return Err(format!("질문 \"{id}\": 질문은 객체여야 합니다")),
        };
        args.insert("state".to_string(), Value::String(state.clone()));
        let incoming = parse_arguments(&Value::Object(args))
            .map_err(|err| format!("질문 \"{id}\": {err}"))?;
        questions.push((id.clone(), incoming));
    }
    Ok(IncomingMany { state, questions })
}

pub fn validate_many(req: &IncomingMany) -> Result<Vec<(String, Question)>, String> {
    req.questions
        .iter()
        .map(|(id, incoming)| {
            validate(incoming)
                .map(|question| (id.clone(), question))
                .map_err(|err| format!("질문 \"{id}\": {err}"))
        })
        .collect()
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test --manifest-path crates/decide/Cargo.toml protocol::`
Expected: 새 테스트 3개와 기존 protocol 테스트가 모두 PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/decide/src/protocol.rs
git commit -m "feat(decide): 질문 맵을 입력 순서대로 파싱하고 검증한다

Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>"
```

---

### Task 2: typesafe — 여러 질문의 본문 생성과 응답 매핑

**Files:**
- Modify: `crates/decide/src/typesafe.rs` (`request_body` 분리, `request_body_many`, `map_answers` 추가, 테스트 추가)

**Interfaces:**
- Consumes: `protocol::Question`(`Clone`), 기존 `MODEL`, `limit_error`
- Produces:
  - `pub fn request_body_many(state: &str, questions: &[(String, Question)]) -> Value`
  - `pub fn map_answers(body: &Value, questions: &[(String, Question)], label: &str) -> Result<(Value, String), String>` — 요청한 id의 답을 입력 순서대로 모은 객체와 모델 이름을 돌려준다.
  - 기존 `request_body(state, &Question) -> Value`는 결과가 이전과 바이트 단위로 같다.

- [ ] **Step 1: Write the failing tests**

`crates/decide/src/typesafe.rs`의 `mod tests` 안, `map_passes_the_answer_through` 테스트 앞에 추가한다. (`noul()` 테스트 헬퍼는 기존 것을 쓴다. `validate`, `Incoming`, `Kind`는 기존 `use`에 있다.)

```rust
    fn many_questions() -> Vec<(String, Question)> {
        vec![
            ("긴급 여부".to_string(), noul()),
            (
                "team a".to_string(),
                validate(&Incoming {
                    state: "s".into(),
                    kind: Kind::Choice,
                    instructions: "어느 팀?".into(),
                    options: vec!["billing".into(), "infra".into()],
                    criteria: vec![],
                })
                .unwrap(),
            ),
        ]
    }

    #[test]
    fn many_body_keeps_order_and_unicode_ids() {
        let body = request_body_many("상태", &many_questions());
        assert_eq!(body["model"], "jev-latest");
        assert_eq!(body["state"], "상태");
        let text = serde_json::to_string(&body["questions"]).unwrap();
        assert!(text.find("긴급 여부").unwrap() < text.find("team a").unwrap());
        assert_eq!(body["questions"]["긴급 여부"]["type"], "noul");
        assert_eq!(body["questions"]["team a"]["criteria"]["infra"], "infra");
    }

    #[test]
    fn single_body_is_the_one_question_case_of_many() {
        let question = noul();
        assert_eq!(
            request_body("s", &question),
            request_body_many("s", &[("q".to_string(), question.clone())])
        );
    }

    #[test]
    fn map_answers_picks_requested_ids_in_order_and_ignores_extras() {
        let questions = many_questions();
        let body = json!({
            "model": "jev-1.13.0",
            "answers": {
                "team a": {"type": "choice", "choice": "infra"},
                "여분": {"type": "noul", "noul": 0.1},
                "긴급 여부": {"type": "noul", "noul": 0.9}
            }
        });
        let (answers, model) = map_answers(&body, &questions, "TypeSafe").unwrap();
        assert_eq!(model, "jev-1.13.0");
        let ids: Vec<&String> = answers.as_object().unwrap().keys().collect();
        assert_eq!(ids, vec!["긴급 여부", "team a"]);
        assert_eq!(answers["긴급 여부"]["noul"], 0.9);
        assert!(answers.get("여분").is_none());
    }

    #[test]
    fn map_answers_fails_when_anything_is_missing() {
        let questions = many_questions();
        assert_eq!(
            map_answers(&json!({"answers": {}}), &questions, "TypeSafe").unwrap_err(),
            "TypeSafe 응답에 model이 없습니다"
        );
        assert_eq!(
            map_answers(&json!({"model": "m"}), &questions, "로컬").unwrap_err(),
            "로컬 응답에 answers가 없습니다"
        );
        assert_eq!(
            map_answers(&json!({"model": "m", "answers": []}), &questions, "로컬").unwrap_err(),
            "로컬 응답에 answers가 없습니다"
        );
        let missing = json!({"model": "m", "answers": {"긴급 여부": {"type": "noul", "noul": 0.5}}});
        assert_eq!(
            map_answers(&missing, &questions, "TypeSafe").unwrap_err(),
            "TypeSafe 응답에 answers.team a가 없습니다"
        );
    }
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test --manifest-path crates/decide/Cargo.toml typesafe::tests::many`
Expected: 컴파일 오류 `cannot find function request_body_many`.

- [ ] **Step 3: Write minimal implementation**

`crates/decide/src/typesafe.rs`의 기존 `request_body` 함수 전체를 아래 세 함수로 바꾼다. (기존 본문의 `match question { ... }`가 `question_json`으로 그대로 옮겨간다.)

```rust
fn question_json(question: &Question) -> Value {
    match question {
        Question::Choice {
            instructions,
            options,
        } => {
            let mut criteria = Map::new();
            for option in options {
                criteria.insert(option.clone(), Value::String(option.clone()));
            }
            json!({
                "type": "choice",
                "instructions": instructions,
                "criteria": Value::Object(criteria),
            })
        }
        Question::Score {
            instructions,
            criteria,
        } => json!({
            "type": "score",
            "instructions": instructions,
            "criteria": criteria,
        }),
        Question::Noul { instructions } => json!({
            "type": "noul",
            "instructions": instructions,
        }),
    }
}

pub fn request_body_many(state: &str, questions: &[(String, Question)]) -> Value {
    let mut map = Map::new();
    for (id, question) in questions {
        map.insert(id.clone(), question_json(question));
    }
    json!({
        "model": MODEL,
        "state": state,
        "questions": Value::Object(map),
    })
}

pub fn request_body(state: &str, question: &Question) -> Value {
    request_body_many(state, &[("q".to_string(), question.clone())])
}
```

`map_response` 함수 다음에 추가한다.

```rust
pub fn map_answers(
    body: &Value,
    questions: &[(String, Question)],
    label: &str,
) -> Result<(Value, String), String> {
    let model = body
        .get("model")
        .and_then(Value::as_str)
        .ok_or_else(|| format!("{label} 응답에 model이 없습니다"))?
        .to_string();
    let answers = body
        .get("answers")
        .and_then(Value::as_object)
        .ok_or_else(|| format!("{label} 응답에 answers가 없습니다"))?;
    let mut picked = Map::new();
    for (id, _) in questions {
        let answer = answers
            .get(id)
            .ok_or_else(|| format!("{label} 응답에 answers.{id}가 없습니다"))?;
        picked.insert(id.clone(), answer.clone());
    }
    Ok((Value::Object(picked), model))
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test --manifest-path crates/decide/Cargo.toml typesafe::`
Expected: 새 테스트 4개와 기존 typesafe 테스트(`body_shapes_match_the_three_types` 포함)가 모두 PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/decide/src/typesafe.rs
git commit -m "feat(decide): 요청 본문과 응답 매핑을 여러 질문으로 일반화한다

Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>"
```

---

### Task 3: backend — `decide_many`

**Files:**
- Modify: `crates/decide/src/backend.rs` (`labels` 분리, `decide_many` 추가, 테스트 추가)
- Modify: `crates/decide/src/lib.rs` (`decide_many` 재노출)

**Interfaces:**
- Consumes: Task 1의 `parse_many`, `validate_many`, `DecideManyResult`; Task 2의 `request_body_many`, `map_answers`; 기존 `select_backend`, `typesafe::limit_error`, `typesafe::execute`
- Produces:
  - `pub fn decide_many<T: Transport>(raw: &Value, env: &Env, transport: &mut T, millis: impl FnMut() -> f64, sleep: impl FnMut()) -> Result<DecideManyResult, String>`
  - `fn labels(backend: Backend) -> (&'static str, &'static str)` — `(routing 이름, 오류 라벨)`

- [ ] **Step 1: Write the failing tests**

`crates/decide/src/backend.rs`의 `mod tests` 끝(마지막 `}` 앞)에 추가한다. (`Script`, `env`, `ok_script`, `RawResponse`는 기존 테스트 헬퍼다.)

```rust
    fn many_raw() -> Value {
        json!({
            "state": "서버 다운",
            "questions": {
                "urgent": {"type": "noul", "instructions": "긴급한가?"},
                "team": {"type": "choice", "instructions": "어느 팀?", "options": ["billing", "infra"]}
            }
        })
    }

    const MANY_OK: &str = r#"{"model":"jev-1.13.0","answers":{"team":{"type":"choice","choice":"infra","probabilities":{"billing":0.1,"infra":0.9},"confidence":0.7},"urgent":{"type":"noul","noul":0.8}}}"#;

    #[test]
    fn many_maps_answers_in_request_order_with_one_call() {
        let mut script = ok_script(MANY_OK);
        let ticks = Cell::new(0.0);
        let result = decide_many(
            &many_raw(),
            &env(None, Some("k")),
            &mut script,
            || {
                let now = ticks.get();
                ticks.set(now + 7.5);
                now
            },
            || panic!("성공 응답은 재시도하지 않는다"),
        )
        .unwrap();
        assert_eq!(script.calls.get(), 1);
        assert_eq!(result.latency_ms, 7.5);
        assert_eq!(result.routing["backend"], "typesafe");
        assert_eq!(result.routing["model"], "jev-1.13.0");
        let ids: Vec<&String> = result.answers.as_object().unwrap().keys().collect();
        assert_eq!(ids, vec!["urgent", "team"]);
        assert_eq!(result.answers["team"]["choice"], "infra");
        assert_eq!(result.answers["urgent"]["noul"], 0.8);
    }

    #[test]
    fn many_validation_error_makes_no_call() {
        let mut script = Script {
            responses: vec![],
            calls: Cell::new(0),
        };
        let raw = json!({
            "state": "s",
            "questions": {
                "ok": {"type": "noul", "instructions": "?"},
                "team": {"type": "choice", "instructions": "?", "options": ["a"]}
            }
        });
        let err = decide_many(&raw, &env(None, Some("k")), &mut script, || 0.0, || {}).unwrap_err();
        assert!(err.starts_with("질문 \"team\": "), "{err}");
        assert_eq!(script.calls.get(), 0);
    }

    #[test]
    fn many_typesafe_limit_names_the_question_but_local_passes_through() {
        let options: Vec<String> = (0..256).map(|i| i.to_string()).collect();
        let raw = json!({
            "state": "s",
            "questions": {
                "small": {"type": "noul", "instructions": "?"},
                "big": {"type": "choice", "instructions": "?", "options": options}
            }
        });
        let mut untouched = Script {
            responses: vec![],
            calls: Cell::new(0),
        };
        let err = decide_many(
            &raw,
            &env(Some("typesafe"), Some("k")),
            &mut untouched,
            || 0.0,
            || {},
        )
        .unwrap_err();
        assert_eq!(err, format!("질문 \"big\": {}", typesafe::CHOICE_LIMIT));
        assert_eq!(untouched.calls.get(), 0);

        let mut script = ok_script(
            r#"{"model":"m","answers":{"small":{"type":"noul","noul":0.5},"big":{"type":"choice","choice":"0"}}}"#,
        );
        let result = decide_many(&raw, &env(Some("local"), None), &mut script, || 0.0, || {}).unwrap();
        assert_eq!(script.calls.get(), 1);
        assert_eq!(result.routing["backend"], "local");
    }

    #[test]
    fn many_fails_entirely_on_backend_error_or_missing_answer() {
        let mut down = Script {
            responses: vec![Err("connection refused".into())],
            calls: Cell::new(0),
        };
        let err = decide_many(
            &many_raw(),
            &env(Some("local"), None),
            &mut down,
            || 0.0,
            || panic!("재시도하면 안 된다"),
        )
        .unwrap_err();
        assert!(err.contains("로컬"), "{err}");
        assert_eq!(down.calls.get(), 1);

        let mut partial = ok_script(
            r#"{"model":"m","answers":{"urgent":{"type":"noul","noul":0.8}}}"#,
        );
        let err = decide_many(&many_raw(), &env(None, Some("k")), &mut partial, || 0.0, || {})
            .unwrap_err();
        assert_eq!(err, "TypeSafe 응답에 answers.team가 없습니다");
    }

    #[test]
    fn many_handles_a_hundred_questions_without_a_cap() {
        let mut questions = serde_json::Map::new();
        let mut answers = serde_json::Map::new();
        for i in 0..100 {
            questions.insert(
                format!("q{i}"),
                json!({"type": "noul", "instructions": "참인가?"}),
            );
            answers.insert(format!("q{i}"), json!({"type": "noul", "noul": 0.5}));
        }
        let raw = json!({"state": "s", "questions": questions});
        let body = json!({"model": "m", "answers": answers}).to_string();
        let mut script = ok_script(&body);
        let result = decide_many(&raw, &env(None, Some("k")), &mut script, || 0.0, || {}).unwrap();
        let ids: Vec<String> = result.answers.as_object().unwrap().keys().cloned().collect();
        let expected: Vec<String> = (0..100).map(|i| format!("q{i}")).collect();
        assert_eq!(ids, expected);
        assert_eq!(script.calls.get(), 1);
    }
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test --manifest-path crates/decide/Cargo.toml backend::tests::many`
Expected: 컴파일 오류 `cannot find function decide_many`.

- [ ] **Step 3: Write minimal implementation**

`crates/decide/src/backend.rs` 맨 위 `use` 두 줄을 바꾼다.

```rust
use crate::protocol::{
    parse_arguments, parse_many, validate, validate_many, DecideManyResult, DecideResult,
};
```

기존 `decide` 안의 라벨 `match`를 함수 호출로 바꾼다. 아래 세 줄을

```rust
    let (name, label) = match backend {
        Backend::Typesafe => ("typesafe", "TypeSafe"),
        Backend::Local => ("local", "로컬"),
    };
```

다음 한 줄로 교체한다.

```rust
    let (name, label) = labels(backend);
```

`pub fn live_transport` 앞에 `labels`와 `decide_many`를 추가한다.

```rust
fn labels(backend: Backend) -> (&'static str, &'static str) {
    match backend {
        Backend::Typesafe => ("typesafe", "TypeSafe"),
        Backend::Local => ("local", "로컬"),
    }
}

pub fn decide_many<T: Transport>(
    raw: &Value,
    env: &Env,
    transport: &mut T,
    mut millis: impl FnMut() -> f64,
    sleep: impl FnMut(),
) -> Result<DecideManyResult, String> {
    let incoming = parse_many(raw)?;
    let questions = validate_many(&incoming)?;
    let backend = select_backend(env.backend.as_deref(), env.api_key.as_deref())?;
    let (name, label) = labels(backend);
    if backend == Backend::Typesafe {
        for (id, question) in &questions {
            if let Some(message) = typesafe::limit_error(question) {
                return Err(format!("질문 \"{id}\": {message}"));
            }
        }
    }
    let body = typesafe::request_body_many(&incoming.state, &questions);
    let start = millis();
    let response = typesafe::execute(transport, &body, label, sleep)?;
    let latency_ms = millis() - start;
    let (answers, model) = typesafe::map_answers(&response, &questions, label)?;
    Ok(DecideManyResult {
        answers,
        routing: json!({
            "backend": name,
            "model": model,
        }),
        latency_ms,
    })
}
```

`crates/decide/src/lib.rs`의 재노출 줄을 바꾼다.

```rust
pub use backend::{decide, decide_many, Backend, Env};
pub use protocol::{DecideManyResult, DecideResult};
```

(기존 `pub use protocol::DecideResult;` 줄은 위 줄로 대체한다.)

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test --manifest-path crates/decide/Cargo.toml`
Expected: 새 backend 테스트 5개를 포함해 전체 PASS. 기존 `decide` 테스트는 수정 없이 통과.

- [ ] **Step 5: Commit**

```bash
git add crates/decide/src/backend.rs crates/decide/src/lib.rs
git commit -m "feat(decide): 한 번의 백엔드 호출로 여러 질문에 답하는 decide_many를 추가한다

Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>"
```

---

### Task 4: mcp — `decide_many` 도구

**Files:**
- Modify: `crates/decide/src/mcp.rs` (도구 목록, 분기, 성공 응답 일반화, 테스트 추가)

**Interfaces:**
- Consumes: Task 3의 `decide_many`, `DecideManyResult`
- Produces: MCP `tools/list`의 두 번째 도구 `decide_many`(첫 번째는 그대로 `decide`), `tools/call`의 이름 분기. 성공 시 `structuredContent`는 `{answers, routing, latency_ms}`다.

- [ ] **Step 1: Write the failing tests**

`crates/decide/src/mcp.rs`의 `mod tests` 끝에 추가한다. (`Script { status, body, calls }`와 `env_local()`은 기존 헬퍼다.)

```rust
    #[test]
    fn tools_list_keeps_decide_first_and_adds_decide_many() {
        let mut script = Script {
            status: 500,
            body: String::new(),
            calls: Cell::new(0),
        };
        let listed = handle_message(
            &json!({"jsonrpc": "2.0", "id": 9, "method": "tools/list"}),
            &env_local(),
            &mut script,
        )
        .unwrap();
        let tools = listed["result"]["tools"].as_array().unwrap();
        assert_eq!(tools.len(), 2);
        assert_eq!(tools[0]["name"], "decide");
        assert_eq!(
            tools[0]["inputSchema"]["required"],
            json!(["state", "type", "instructions"])
        );
        assert_eq!(tools[1]["name"], "decide_many");
        assert_eq!(
            tools[1]["inputSchema"]["required"],
            json!(["state", "questions"])
        );
        assert!(tools[1]["description"].as_str().unwrap().contains("쪼갠다"));
    }

    #[test]
    fn decide_many_call_returns_answers_or_a_tool_error() {
        let mut script = Script {
            status: 200,
            body: r#"{"model":"jev-1.13.0","answers":{"a":{"type":"noul","noul":0.25},"b":{"type":"noul","noul":0.75}}}"#.into(),
            calls: Cell::new(0),
        };
        let call = |arguments: Value, id: i64| {
            json!({
                "jsonrpc": "2.0",
                "id": id,
                "method": "tools/call",
                "params": {"name": "decide_many", "arguments": arguments}
            })
        };
        let response = handle_message(
            &call(
                json!({
                    "state": "다운",
                    "questions": {
                        "a": {"type": "noul", "instructions": "긴급한가?"},
                        "b": {"type": "noul", "instructions": "결제 장애인가?"}
                    }
                }),
                5,
            ),
            &Env {
                backend: None,
                api_key: Some("k".into()),
            },
            &mut script,
        )
        .unwrap();
        assert_eq!(response["result"]["isError"], false);
        let structured = &response["result"]["structuredContent"];
        assert_eq!(structured["answers"]["a"]["noul"], 0.25);
        assert_eq!(structured["answers"]["b"]["noul"], 0.75);
        assert_eq!(structured["routing"]["backend"], "typesafe");
        assert_eq!(script.calls.get(), 1);

        let bad = handle_message(
            &call(json!({"state": "다운", "questions": {}}), 6),
            &env_local(),
            &mut script,
        )
        .unwrap();
        assert_eq!(bad["result"]["isError"], true);
        assert_eq!(
            bad["result"]["content"][0]["text"],
            "questions는 비어 있을 수 없습니다"
        );
        assert_eq!(script.calls.get(), 1, "검증 오류는 전송을 타지 않는다");
    }
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test --manifest-path crates/decide/Cargo.toml mcp::tests`
Expected: `tools_list_keeps_decide_first_and_adds_decide_many`는 `assertion ... left: 1 right: 2`로 FAIL, `decide_many_call_...`는 `알 수 없는 도구입니다`로 FAIL. 

- [ ] **Step 3: Write minimal implementation**

`crates/decide/src/mcp.rs` 맨 위 `use` 두 줄을 바꾼다.

```rust
use crate::backend::{decide, decide_many, Env};
use crate::typesafe::Transport;
```

(`use crate::protocol::DecideResult;` 줄은 지운다. `tool_success`가 제네릭이 되어 더 이상 필요 없다.)

`TOOL_DESCRIPTION` 상수 다음에 추가한다.

```rust
const MANY_DESCRIPTION: &str = "\
한 state에 대해 질문 여러 개(choice, score, noul)를 한 번의 호출로 판단한다. \
questions는 질문 id를 키로 하는 객체이고, 각 질문은 decide의 type, instructions, \
options, criteria와 같다. 질문 하나하나는 yes/no나 단일 선택처럼 작게 쪼갠다. \
복합 질문은 정확도가 떨어진다. 하나라도 틀리거나 실패하면 전체가 오류이고 부분 결과는 \
없다. 개방형 추론/생성에는 사용하지 않는다.";
```

`tools_list` 함수 전체를 바꾼다.

```rust
fn tools_list() -> Value {
    json!({
        "tools": [
            {
                "name": "decide",
                "description": TOOL_DESCRIPTION,
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "state": {"type": "string"},
                        "type": {"type": "string", "enum": ["choice", "score", "noul"]},
                        "instructions": {"type": "string"},
                        "options": {"type": "array", "items": {"type": "string"}},
                        "criteria": {"type": "array", "items": {"type": "string"}}
                    },
                    "required": ["state", "type", "instructions"]
                }
            },
            {
                "name": "decide_many",
                "description": MANY_DESCRIPTION,
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "state": {"type": "string"},
                        "questions": {
                            "type": "object",
                            "minProperties": 1,
                            "additionalProperties": {
                                "type": "object",
                                "properties": {
                                    "type": {"type": "string", "enum": ["choice", "score", "noul"]},
                                    "instructions": {"type": "string"},
                                    "options": {"type": "array", "items": {"type": "string"}},
                                    "criteria": {"type": "array", "items": {"type": "string"}}
                                },
                                "required": ["type", "instructions"]
                            }
                        }
                    },
                    "required": ["state", "questions"]
                }
            }
        ]
    })
}
```

`tool_call` 함수의 이름 검사부터 끝까지를 바꾼다. 아래 블록

```rust
    let name = params.get("name").and_then(Value::as_str).unwrap_or("");
    if name != "decide" {
        return Ok(tool_error("알 수 없는 도구입니다"));
    }
    let arguments = params
        .get("arguments")
        .cloned()
        .unwrap_or_else(|| json!({}));
    let origin = Instant::now();
    match decide(
        &arguments,
        env,
        transport,
        || origin.elapsed().as_secs_f64() * 1000.0,
        || std::thread::sleep(Duration::from_secs(1)),
    ) {
        Ok(result) => Ok(tool_success(&result)),
        Err(message) => Ok(tool_error(&message)),
    }
```

를 다음으로 교체한다.

```rust
    let name = params.get("name").and_then(Value::as_str).unwrap_or("");
    let arguments = params
        .get("arguments")
        .cloned()
        .unwrap_or_else(|| json!({}));
    let origin = Instant::now();
    let clock = || origin.elapsed().as_secs_f64() * 1000.0;
    let pause = || std::thread::sleep(Duration::from_secs(1));
    Ok(match name {
        "decide" => match decide(&arguments, env, transport, clock, pause) {
            Ok(result) => tool_success(&result),
            Err(message) => tool_error(&message),
        },
        "decide_many" => match decide_many(&arguments, env, transport, clock, pause) {
            Ok(result) => tool_success(&result),
            Err(message) => tool_error(&message),
        },
        _ => tool_error("알 수 없는 도구입니다"),
    })
```

`tool_success` 시그니처를 제네릭으로 바꾼다.

```rust
fn tool_success<T: serde::Serialize>(result: &T) -> Value {
```

(함수 본문은 그대로다.)

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test --manifest-path crates/decide/Cargo.toml`
Expected: 전체 PASS. 기존 `initialize_echoes_the_protocol_and_lists_decide`(`tools[0]["name"] == "decide"`)와 `tool_call_routes_to_typesafe_or_local`은 수정 없이 통과. 통합 테스트 `tests/stdio.rs`도 통과.

- [ ] **Step 5: Commit**

```bash
git add crates/decide/src/mcp.rs
git commit -m "feat(decide): MCP에 decide_many 도구를 추가한다

Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>"
```

---

### Task 5: daemon — `questions` 줄 형식과 캐시

**Files:**
- Modify: `crates/decide/src/daemon.rs` (`CacheKey`/`Cache`/`call` 변경, 테스트 추가)

**Interfaces:**
- Consumes: Task 1의 `parse_many`, `IncomingMany`; Task 3의 `decide_many`; 기존 `decide`, `parse_arguments`, `select_backend`
- Produces: 소켓 줄에 `questions`가 있으면 `decide_many`와 같은 모양(`{answers, routing, latency_ms}`)으로 답한다. `type`과 `questions`를 함께 주면 `type과 questions는 함께 쓸 수 없습니다` 오류다. 캐시는 `Value`(최종 응답 JSON)를 저장하고 적중 시 `routing.cached = true`, `latency_ms = 0.0`을 덮어쓴다.

- [ ] **Step 1: Write the failing tests**

`crates/decide/src/daemon.rs`의 `mod tests` 안, `identical_requests_hit_the_cache_and_skip_the_transport` 테스트 다음에 추가한다. (`handle_line`, `Cache`, `Env`, `Cell`, `RawResponse`, `Transport`는 기존 것을 쓴다.)

```rust
    struct ManyScript {
        calls: Cell<usize>,
    }

    impl Transport for ManyScript {
        fn post_json(&mut self, _body: &Value) -> Result<RawResponse, String> {
            self.calls.set(self.calls.get() + 1);
            Ok(RawResponse {
                status: 200,
                body: r#"{"model":"jev-1.13.0","answers":{"a":{"type":"noul","noul":0.2},"b":{"type":"noul","noul":0.6},"q":{"type":"noul","noul":0.4}}}"#.into(),
            })
        }
    }

    fn typesafe_env() -> Env {
        Env {
            backend: None,
            api_key: Some("k".into()),
        }
    }

    const MANY_LINE: &str = r#"{"state":"s","questions":{"a":{"type":"noul","instructions":"참인가?"},"b":{"type":"noul","instructions":"급한가?"}}}"#;

    #[test]
    fn questions_line_returns_answers_and_hits_the_cache() {
        let mut script = ManyScript {
            calls: Cell::new(0),
        };
        let mut cache = Cache::default();
        let first = handle_line(MANY_LINE, &typesafe_env(), &mut script, &mut cache).unwrap();
        let first: Value = serde_json::from_str(&first).unwrap();
        assert_eq!(first["answers"]["a"]["noul"], 0.2);
        assert_eq!(first["answers"]["b"]["noul"], 0.6);
        assert_eq!(first["routing"]["backend"], "typesafe");
        assert_eq!(first["routing"].get("cached"), None);
        assert!(first.get("answer").is_none());
        assert_eq!(script.calls.get(), 1);

        let second = handle_line(MANY_LINE, &typesafe_env(), &mut script, &mut cache).unwrap();
        let second: Value = serde_json::from_str(&second).unwrap();
        assert_eq!(second["answers"]["b"]["noul"], 0.6);
        assert_eq!(second["routing"]["cached"], true);
        assert_eq!(second["latency_ms"], 0.0);
        assert_eq!(script.calls.get(), 1, "동일 요청은 전송을 타지 않는다");
    }

    #[test]
    fn reordered_questions_are_a_different_cache_entry() {
        let mut script = ManyScript {
            calls: Cell::new(0),
        };
        let mut cache = Cache::default();
        handle_line(MANY_LINE, &typesafe_env(), &mut script, &mut cache).unwrap();
        let reordered = r#"{"state":"s","questions":{"b":{"type":"noul","instructions":"급한가?"},"a":{"type":"noul","instructions":"참인가?"}}}"#;
        handle_line(reordered, &typesafe_env(), &mut script, &mut cache).unwrap();
        assert_eq!(script.calls.get(), 2);
    }

    #[test]
    fn type_together_with_questions_is_rejected_without_a_call() {
        let mut script = ManyScript {
            calls: Cell::new(0),
        };
        let mut cache = Cache::default();
        let line = r#"{"state":"s","type":"noul","instructions":"?","questions":{"a":{"type":"noul","instructions":"?"}}}"#;
        let out = handle_line(line, &typesafe_env(), &mut script, &mut cache).unwrap();
        let out: Value = serde_json::from_str(&out).unwrap();
        assert_eq!(out["error"], "type과 questions는 함께 쓸 수 없습니다");
        assert_eq!(script.calls.get(), 0);
    }

    #[test]
    fn single_and_many_with_the_same_content_do_not_share_a_cache_entry() {
        let mut script = ManyScript {
            calls: Cell::new(0),
        };
        let mut cache = Cache::default();
        let single = r#"{"state":"s","type":"noul","instructions":"참인가?"}"#;
        let many = r#"{"state":"s","questions":{"q":{"type":"noul","instructions":"참인가?"}}}"#;
        let one = handle_line(single, &typesafe_env(), &mut script, &mut cache).unwrap();
        let one: Value = serde_json::from_str(&one).unwrap();
        assert_eq!(one["answer"]["noul"], 0.4);
        let out = handle_line(many, &typesafe_env(), &mut script, &mut cache).unwrap();
        let out: Value = serde_json::from_str(&out).unwrap();
        assert_eq!(out["answers"]["q"]["noul"], 0.4);
        assert!(out["routing"].get("cached").is_none());
        assert_eq!(script.calls.get(), 2, "단일과 다중은 캐시를 공유하지 않는다");
    }
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test --manifest-path crates/decide/Cargo.toml daemon::tests`
Expected: `questions_line_returns_answers_and_hits_the_cache`가 `answer` 없음/`questions` 미지원으로 FAIL(현재 `call`은 `parse_arguments`만 써서 `type이 필요합니다` 오류를 돌려준다).

- [ ] **Step 3: Write minimal implementation**

`crates/decide/src/daemon.rs` 맨 위 두 `use` 줄을 바꾼다.

```rust
use crate::backend::{decide, decide_many, live_transport, select_backend, Backend, Env};
use crate::protocol::{parse_arguments, parse_many, Incoming, IncomingMany};
```

(`DecideResult` 임포트는 지운다. 이제 이 파일에서 쓰지 않는다.)

`CacheKey`와 `Cache` 정의와 impl을 아래로 바꾼다.

```rust
pub const TYPE_WITH_QUESTIONS: &str = "type과 questions는 함께 쓸 수 없습니다";

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
enum CacheKey {
    One {
        backend: Backend,
        incoming: Incoming,
    },
    Many {
        backend: Backend,
        incoming: IncomingMany,
    },
}

#[derive(Default)]
pub struct Cache {
    entries: HashMap<CacheKey, Value>,
    order: VecDeque<CacheKey>,
}

impl Cache {
    fn get(&self, key: &CacheKey) -> Option<&Value> {
        self.entries.get(key)
    }

    fn insert(&mut self, key: CacheKey, value: Value) {
        if !self.entries.contains_key(&key) {
            self.order.push_back(key.clone());
            if self.order.len() > MAX_CACHE_ENTRIES {
                if let Some(oldest) = self.order.pop_front() {
                    self.entries.remove(&oldest);
                }
            }
        }
        self.entries.insert(key, value);
    }
}
```

`call` 함수 전체를 바꾼다.

```rust
fn call<T: Transport>(
    raw: &Value,
    env: &Env,
    transport: &mut T,
    cache: &mut Cache,
) -> Result<Value, String> {
    let many = raw.get("questions").is_some();
    if many && raw.get("type").is_some() {
        return Err(TYPE_WITH_QUESTIONS.to_string());
    }
    let backend = select_backend(env.backend.as_deref(), env.api_key.as_deref()).ok();
    let cache_key = backend.and_then(|backend| {
        if many {
            parse_many(raw)
                .ok()
                .map(|incoming| CacheKey::Many { backend, incoming })
        } else {
            parse_arguments(raw)
                .ok()
                .map(|incoming| CacheKey::One { backend, incoming })
        }
    });
    if let Some(key) = &cache_key {
        if let Some(cached) = cache.get(key) {
            let mut result = cached.clone();
            result["routing"]["cached"] = json!(true);
            result["latency_ms"] = json!(0.0);
            return Ok(result);
        }
    }
    let origin = Instant::now();
    let clock = || origin.elapsed().as_secs_f64() * 1000.0;
    let pause = || std::thread::sleep(Duration::from_secs(1));
    let result = if many {
        serde_json::to_value(decide_many(raw, env, transport, clock, pause)?)
    } else {
        serde_json::to_value(decide(raw, env, transport, clock, pause)?)
    }
    .map_err(|err| err.to_string())?;
    if let Some(key) = cache_key {
        cache.insert(key, result.clone());
    }
    Ok(result)
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test --manifest-path crates/decide/Cargo.toml`
Expected: 전체 PASS. 기존 `line_protocol_returns_one_json_object`와 `identical_requests_hit_the_cache_and_skip_the_transport`는 수정 없이 통과(캐시 적중 시 `routing.cached == true`, `latency_ms`는 0.0). 통합 테스트 `tests/daemon.rs`도 통과.

- [ ] **Step 5: Commit**

```bash
git add crates/decide/src/daemon.rs
git commit -m "feat(decide): 데몬 소켓이 questions 줄을 받고 같은 캐시를 쓴다

Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>"
```

---

### Task 6: 문서와 실서버 검증

**Files:**
- Modify: `README.md` ("사용법" 절에 `decide_many` 추가)
- Modify: `CHANGELOG.md` (`[Unreleased]` 추가)
- Modify: `CLAUDE.md` (Architecture에 `decide_many` 한 줄)
- Modify: `checklist.md` (1번 체크 갱신)

**Interfaces:**
- Consumes: Task 1~5의 동작
- Produces: 사용자 문서, 실서버 검증 기록

- [ ] **Step 1: README에 사용법 추가**

`README.md`의 "### 에이전트가 쓸 때" 절 바로 앞에 다음 절을 추가한다.

````markdown
### 질문 여러 개를 한 번에 (`decide_many`)

같은 state에 대해 질문이 여러 개면 `decide_many`로 한 번에 보낸다. 백엔드 호출은 한 번이다. `questions`는 질문 id를 키로 하는 객체이고, 각 질문은 `decide`의 `type`, `instructions`, `options`, `criteria`와 같다.

```text
decide_many(
  state="서버가 다운됐습니다. 결제 API가 500을 반환합니다.",
  questions={
    "urgent": {type: "noul",   instructions: "이 요청이 긴급한가?"},
    "team":   {type: "choice", instructions: "어느 팀이 처리해야 하는가?", options: ["billing", "infra", "sales"]},
    "anger":  {type: "score",  instructions: "고객의 불만 강도는?", criteria: ["낮음", "보통", "높음"]}
  }
)
```

반환은 `answers`(질문 id → 답, 입력 순서), `routing`, `latency_ms`다. 하나라도 검증에 실패하거나 백엔드가 실패하면 전체가 도구 오류이고 부분 결과는 없다. 검증 오류 앞에는 `질문 "<id>": `가 붙는다. 질문 개수 상한은 두지 않고 백엔드가 거절하는 대로 돌려준다.

질문 하나하나는 yes/no나 단일 선택처럼 작게 쪼갠다. 복합 질문은 정확도가 떨어진다. 앞 답에 따라 뒤 질문을 정해야 하면 호출을 나눈다. `decide daemon` 소켓도 줄에 `questions`가 있으면 같은 모양으로 답한다(`type`과 함께 줄 수 없다).
````

- [ ] **Step 2: CHANGELOG, CLAUDE.md, checklist 갱신**

`CHANGELOG.md`의 `## [0.0.4] - 2026-09-30` 줄 바로 위에 추가한다.

```markdown
## [Unreleased]

### 추가

- MCP 도구 `decide_many`가 한 state에 질문 여러 개를 한 번의 백엔드 호출로 판단한다.
  `questions`(id → 질문)를 받아 `answers`(입력 순서), `routing`, `latency_ms`를 돌려준다.
  하나라도 실패하면 전체가 오류다. `decide` 도구는 바뀌지 않는다.
- `decide daemon` 소켓 줄에 `questions`가 있으면 `decide_many`와 같은 모양으로 답하고 같은
  LRU 캐시를 쓴다. `type`과 `questions`를 함께 주면 오류다.
```

`CLAUDE.md`의 Architecture 목록에서 `backend.rs` 줄 다음에 추가한다.

```markdown
- `decide_many` (`backend.rs`, `mcp.rs`, `daemon.rs`) sends several questions for one state
  in a single backend call. `protocol::parse_many`/`validate_many` keep input order and
  prefix errors with `질문 "<id>": `; `typesafe::request_body_many`/`map_answers` build the
  body and pick the requested ids. All-or-nothing; no question-count cap. `decide` is unchanged.
```

`checklist.md`의 1번 섹션에서 두 줄을 체크한다.

```markdown
- [x] 사용자가 설계 문서 검토·승인
- [x] 구현 계획 작성(`docs/superpowers/plans/2026-09-30-decide-many.md`)
```

(구현, 실서버 측정 줄은 이 계획을 끝낸 뒤 체크한다.)

- [ ] **Step 3: 실서버 검증(로컬 백엔드)**

서버를 scratchpad의 임시 venv로 띄우고(영구 설치 아님) `decide mcp`로 한글 다중 질문을 한 번 돌린다. 실행 전에 `cargo build`로 바이너리를 만든다.

```bash
cargo build --manifest-path crates/decide/Cargo.toml
S=/private/tmp/claude-501/-Users-spark-Develop-Workspaces-decide/d4b61354-8676-4c2c-bea5-3d6557ee80fd/scratchpad   # jv 가상환경이 있는 디렉터리
(cd $S && HF_HUB_OFFLINE=1 nohup jv/bin/jev-style serve --release 2b --precision 8bit > serve_many.log 2>&1 &)
for i in $(seq 1 40); do curl -s -o /dev/null --max-time 2 http://127.0.0.1:8765/ && break; sleep 2; done
printf '%s\n%s\n%s\n' \
  '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-03-26","capabilities":{},"clientInfo":{"name":"t","version":"0"}}}' \
  '{"jsonrpc":"2.0","method":"notifications/initialized"}' \
  '{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"decide_many","arguments":{"state":"배송이 2주째 안 와요. 환불해 주세요. 정말 화가 납니다.","questions":{"intent":{"type":"choice","instructions":"고객의 요청은?","options":["환불","교환","배송조회"]},"urgent":{"type":"noul","instructions":"이 요청은 긴급한가?"},"anger":{"type":"score","instructions":"고객의 불만 강도는?","criteria":["낮음","보통","높음"]}}}}}' \
  | env -u TYPESAFE_API_KEY DECIDE_BACKEND=local ./crates/decide/target/debug/decide mcp | sed -n 2p
pkill -f "jev-style serve"
```

Expected: `isError:false`이고 `structuredContent.answers`에 `intent`(choice, `환불`이 가장 높음), `urgent`(noul), `anger`(score, 높은 쪽)가 입력 순서로 있고 `routing.backend`는 `local`이다. 세 답이 모두 오는지, 한글 판단이 상식과 크게 어긋나지 않는지 확인하고 지연을 기록한다. 서버가 1~3초 안에 안 뜨면 `serve_many.log`를 읽는다.

- [ ] **Step 4: 전체 테스트와 커밋**

Run: `cargo test --manifest-path crates/decide/Cargo.toml`
Expected: 전체 PASS(기준선 32건 + 이 계획에서 추가한 테스트).

```bash
git add README.md CHANGELOG.md CLAUDE.md checklist.md
git commit -m "docs(decide): decide_many 사용법과 변경 내역을 문서화한다

Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>"
```

- [ ] **Step 5: 실서버 결과 기록**

`checklist.md`의 1번 섹션에서 구현/실측 줄을 체크하고 측정값을 덧붙이고, `context-notes.md`에 실서버 결과(지연, 답 요약, 놓친 점)를 한 단락으로 덧붙인다. TypeSafe 다중 질문 실호출(키 필요)은 체크하지 않고 사용자에게 남긴다.

```bash
git add checklist.md context-notes.md
git commit -m "docs: decide_many 실서버 검증 결과를 기록한다

Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>"
```
