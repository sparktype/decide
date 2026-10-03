# Clef-flash 로컬 백엔드 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** `DECIDE_BACKEND=local`을 Cloudflare Clef-flash(Qwen3.5-9B 하이브리드 백본 GGUF Q4_K_M + joint schema head)로 채우고, 기존 TypeSafe 백엔드와 같은 도구 계약(`answer`/`routing`/`latency_ms`)으로 답하게 한다.

**Architecture:** candle-transformers의 미병합 PR #3396(`quantized_qwen3_5.rs`)을 `crates/decide/src/local/backbone.rs`에 벤더링해 32레이어 하이브리드(28 linear_attention + 4 full_attention) forward를 GGUF Q4_K_M으로 실행하고, 마지막 레이어 정규화 출력(전체 시퀀스 hidden_states)을 뽑아 신규 구현한 `joint_head.rs`(EvidenceRoutingLayer × 2 + TransformerDecoderLayer × 4)에 넣어 질문·옵션별 로짓을 얻는다. `tokenizer.rs`가 `decide` 도구 인자를 Clef 스키마 텍스트로 조립하고, `postprocess.rs`가 로짓을 TypeSafe와 같은 응답 틀로 변환한다. Python(`transformers`) BF16 오라클과 대조하는 `cargo test --features parity`가 통과하기 전에는 `backend.rs`가 계속 `local::NOT_READY`를 반환한다.

**Tech Stack:** Rust, candle-core/candle-nn/candle-transformers(vendored 코드 포함), tokenizers, hf-hub, serde_json. 오라클 쪽은 Python `transformers`(비교 전용, 커밋되지 않는 스크립트).

**Spec:** `docs/superpowers/specs/2026-10-02-clef-flash-local-backend-design.md`

## Global Constraints

- 도구 인자(`state`, `type`, `instructions`, `options`, `criteria`)와 반환 틀(`answer`/`routing`/`latency_ms`)은 바뀌지 않는다 — `protocol.rs`/`backend.rs`의 공개 계약은 그대로 유지한다.
- 로컬 백엔드는 **텍스트 전용**이다. 비전/비디오 입력, Qwen3-VL 비전 인코더는 범위 밖이다.
- 1차 백본은 **GGUF Q4_K_M, CPU 추론만**이다. Metal/GPU 가속, BF16/FP8 로컬 추론은 범위 밖이다.
- 백본 벤더링 대상은 **PR #3396**(`quantized_qwen3_5.rs`)이다. PR #3461의 청크 병렬 스캔은 범위 밖이다.
- Clef(27B)는 다루지 않는다. Clef-flash(9B, `Qwen/Qwen3.5-9B` 백본)만 다룬다.
- `joint_head_config.json` 치수는 고정값이다: `hidden_size=4096`, `width=1024`, `routing_layers=2`, `layers=4`, `heads=16`, `feedforward=4096`.
- parity 게이트(`cargo test --features parity`)를 통과하기 전에는 `DECIDE_BACKEND=local`이 `로컬 백엔드가 아직 준비되지 않았습니다`를 반환해야 한다. 게이트 통과 후에만 `local.rs`를 교체한다.
- `LAYA_WEIGHTS` 환경변수와 Laya 관련 코드·설정은 전부 제거하고 `CLEF_WEIGHTS`로 대체한다.
- 가중치는 저장소나 Homebrew formula에 넣지 않는다. 바이너리만 설치하는 기존 방식을 유지한다.

## Review Focus

- **빈 options/criteria 경계값**: choice 2개 미만, score 2개 미만은 이미 `protocol.rs::validate`가 걸러내지만, `tokenizer.rs`가 그 뒤에 받는 `Question::Choice`/`Score`가 빈 벡터를 받는 경우는 없어야 한다 — `tokenizer.rs`의 스키마 텍스트 조립 테스트가 이 전제를 재확인한다(Task 2).
- **state에 포함된 개행·따옴표·백틱 등 스키마 텍스트와 충돌하는 문자**: `encode_record`의 `FIELD/ID/TYPE/INSTRUCTION` 헤더 포맷은 평문 줄바꿈 기반이라, state 자체에 `FIELD` 같은 문자열이 들어있어도 안전하게 JSON 직렬화 뒤에 와야 한다 — Task 2의 테스트가 state를 JSON으로 감싸 스키마 텍스트와 분리되는지 확인한다.
- **4bit 양자화로 인한 확률 오차가 결정을 뒤집는 경우**: choice 응답에서 1·2위 확률이 근소한 입력이면 양자화 오차가 `choice` 필드 자체(최댓값 옵션)를 바꿀 수 있다 — Task 7의 parity 허용 오차 설계는 `probabilities`뿐 아니라 `choice`/`score`의 최댓값 선택 일치 여부도 별도로 검사해야 한다.
- **max_length(16,384 토큰) 초과 state**: `encode_record`는 state 토큰을 잘라낸다고 설계서가 적었지만, 지금 `tokenizer.rs` 변환 로직은 이 절단을 구현하지 않으면 긴 state에서 백본 forward가 컨텍스트 길이를 넘겨 에러를 낼 수 있다 — Task 2에 긴 state 절단 테스트를 추가한다.
- **GGUF 메타데이터 키 이름이 `qwen3.*`가 아니라 `qwen35.*`로 올 가능성**: 벤더링 소스의 `md_get` 폴백(`qwen3.` → `qwen35.` 치환)이 Clef-flash가 실제로 내보낸 GGUF와 맞는지 Task 3에서 실제 GGUF 파일 헤더를 열어 확인해야 한다 — 메타데이터 키가 전혀 다르면 로드 자체가 실패한다.

---

### Task 1: 의존성 추가와 가중치 디렉터리 해석

**Files:**
- Modify: `crates/decide/Cargo.toml`
- Create: `crates/decide/src/local/mod.rs`
- Test: `crates/decide/src/local/mod.rs` (인라인 `#[cfg(test)]`)

**Interfaces:**
- Produces: `pub fn weights_dir() -> Result<PathBuf, String>` — `CLEF_WEIGHTS` 환경변수가 있으면 그 경로, 없으면 `~/.cache/huggingface/hub` 하위 캐시 경로를 돌려준다 (다운로드는 Task 6에서 연결).
- Produces: `pub const NOT_READY: &str = "로컬 백엔드가 아직 준비되지 않았습니다";` (기존 `local.rs`의 상수를 그대로 옮김)
- Produces: `pub fn infer(state: &str, question: &crate::protocol::Question) -> Result<Value, String>` — 지금은 바로 `Err(NOT_READY.to_string())`를 반환하는 스텁(Task 7에서 실제 추론으로 교체).

- [ ] **Step 1: `crates/decide/Cargo.toml`에 candle 의존성을 추가한다**

```toml
[dependencies]
libc = "0.2"
serde = { version = "1", features = ["derive"] }
serde_json = { version = "1", features = ["preserve_order"] }
ureq = { version = "2.12", default-features = false, features = ["tls", "json"] }
candle-core = "0.9"
candle-nn = "0.9"
tokenizers = { version = "0.21", default-features = false, features = ["onig"] }
hf-hub = { version = "1.0", default-features = false, features = ["ureq"] }

[features]
parity = []
```

(`candle-transformers`는 의존성으로 추가하지 않는다 — Task 3에서 필요한 두 유틸리티만 벤더링한 소스 안에 직접 복제한다. 전체 crate를 끌어오면 불필요한 전이 의존성이 늘어난다.)

- [ ] **Step 2: 기존 `local.rs`를 지우고 `local/mod.rs`를 만든다**

```bash
git rm crates/decide/src/local.rs
```

```rust
// crates/decide/src/local/mod.rs
mod backbone;
mod joint_head;
mod postprocess;
mod tokenizer;

use crate::protocol::Question;
use serde_json::Value;
use std::path::PathBuf;

pub const NOT_READY: &str = "로컬 백엔드가 아직 준비되지 않았습니다";

pub fn weights_dir() -> Result<PathBuf, String> {
    if let Ok(dir) = std::env::var("CLEF_WEIGHTS") {
        let trimmed = dir.trim();
        if !trimmed.is_empty() {
            return Ok(PathBuf::from(trimmed));
        }
    }
    let home = std::env::var("HOME").map_err(|_| "HOME 환경변수를 읽을 수 없습니다".to_string())?;
    Ok(PathBuf::from(home)
        .join(".cache")
        .join("huggingface")
        .join("hub"))
}

pub fn infer(_state: &str, _question: &Question) -> Result<Value, String> {
    Err(NOT_READY.to_string())
}
```

- [ ] **Step 3: `crates/decide/src/lib.rs`가 `local` 모듈을 그대로 재노출하는지 확인한다**

`lib.rs`에 이미 `pub mod local;`이 있으므로 수정 불필요 — 디렉터리로 바뀌어도 Rust 모듈 경로는 동일하게 해석된다.

- [ ] **Step 4: 테스트를 작성한다**

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn weights_dir_prefers_env_override() {
        std::env::set_var("CLEF_WEIGHTS", "/tmp/clef-weights-test");
        assert_eq!(weights_dir().unwrap(), PathBuf::from("/tmp/clef-weights-test"));
        std::env::remove_var("CLEF_WEIGHTS");
    }

    #[test]
    fn weights_dir_falls_back_to_hf_cache() {
        std::env::remove_var("CLEF_WEIGHTS");
        let dir = weights_dir().unwrap();
        assert!(dir.ends_with(".cache/huggingface/hub") || dir.to_string_lossy().contains(".cache"));
    }

    #[test]
    fn infer_is_not_ready_before_the_gate() {
        let question = crate::protocol::Question::Noul {
            instructions: "참인가?".into(),
        };
        assert_eq!(infer("상태", &question).unwrap_err(), NOT_READY);
    }
}
```

- [ ] **Step 5: 빌드와 테스트를 돌린다**

Run: `cargo test --manifest-path crates/decide/Cargo.toml local::`
Expected: 세 테스트 모두 PASS. (`cargo build`가 candle 의존성을 처음 받아오느라 느릴 수 있다 — 정상이다.)

- [ ] **Step 6: 커밋**

```bash
git add crates/decide/Cargo.toml crates/decide/src/local/mod.rs
git commit -m "feat(decide): local 백엔드를 모듈로 쪼개고 Clef 가중치 경로 해석을 추가한다"
```

---

### Task 2: 스키마 텍스트 조립 (`tokenizer.rs`)

**Files:**
- Create: `crates/decide/src/local/tokenizer.rs`
- Modify: `crates/decide/src/local/mod.rs` (`mod tokenizer;` 추가는 Task 1에서 이미 했음 — 여기서는 내용만 채움)
- Test: `crates/decide/src/local/tokenizer.rs` (인라인)

**Interfaces:**
- Consumes: `crate::protocol::Question` (`Question::Choice { instructions, options }`, `Question::Score { instructions, criteria }`, `Question::Noul { instructions }` — Task 1 이전부터 존재하는 타입, 변경 없음)
- Produces: `pub fn schema_text(state: &str, question: &Question) -> String` — 백본에 넣을 전체 프롬프트 문자열.
- Produces: `pub struct QuestionSpan { pub label: String, pub start_char: usize, pub end_char: usize }` 와 `pub struct OptionSpan { pub label: String, pub start_char: usize, pub end_char: usize }` — `joint_head.rs`(Task 4)가 토큰 스팬을 찾을 때 쓰는 문자 오프셋. `pub fn spans(state: &str, question: &Question) -> (QuestionSpan, Vec<OptionSpan>)`가 이 둘을 함께 계산한다.
- Produces: `pub const MAX_STATE_CHARS: usize = 48_000;` 와 `pub fn truncate_state(state: &str) -> &str` — 16,384 토큰 한도를 문자 수 근사치(토큰당 평균 ~3자 가정, 안전 여유를 둔 상한)로 자른다. 정확한 토큰 경계 절단은 백본 토크나이저가 처리하므로(Task 3), 여기서는 "스키마 텍스트에 들어가기 전 과도하게 긴 state를 사전에 줄인다"는 보호 장치만 둔다.

- [ ] **Step 1: 실패하는 테스트를 먼저 쓴다**

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::Question;

    #[test]
    fn noul_schema_has_no_options_block() {
        let q = Question::Noul { instructions: "참인가?".into() };
        let text = schema_text("상태 문장", &q);
        assert!(text.contains("FIELD q"));
        assert!(text.contains("ID:q"));
        assert!(text.contains("TYPE:noul"));
        assert!(text.contains("INSTRUCTION:참인가?"));
        assert!(!text.contains("OPTION"));
        assert!(text.contains("\"상태 문장\""));
    }

    #[test]
    fn choice_schema_lists_options_in_order() {
        let q = Question::Choice {
            instructions: "어느 팀?".into(),
            options: vec!["billing".into(), "technical".into()],
        };
        let text = schema_text("s", &q);
        assert_eq!(text.find("billing").map(|i| i < text.find("technical").unwrap()), Some(true));
        assert!(text.contains("TYPE:choice"));
    }

    #[test]
    fn score_schema_lists_levels_in_order() {
        let q = Question::Score {
            instructions: "강도?".into(),
            criteria: vec!["낮음".into(), "높음".into()],
        };
        let text = schema_text("s", &q);
        assert_eq!(text.find("낮음").map(|i| i < text.find("높음").unwrap()), Some(true));
        assert!(text.contains("TYPE:score"));
    }

    #[test]
    fn state_is_json_serialized_so_embedded_headers_cannot_collide() {
        let q = Question::Noul { instructions: "참인가?".into() };
        let tricky_state = "FIELD q\nID:q\nTYPE:choice";
        let text = schema_text(tricky_state, &q);
        // state는 JSON 문자열로 들어가므로 원문 그대로의 "FIELD q" 줄바꿈은
        // 스키마 헤더가 아니라 JSON 문자열 리터럴 안에 있어야 한다.
        assert!(text.contains("\"FIELD q\\nID:q\\nTYPE:choice\""));
    }

    #[test]
    fn long_state_is_truncated_before_assembly() {
        let long_state = "가".repeat(MAX_STATE_CHARS + 1000);
        let truncated = truncate_state(&long_state);
        assert!(truncated.chars().count() <= MAX_STATE_CHARS);
    }

    #[test]
    fn spans_locate_question_and_options_by_char_offset() {
        let q = Question::Choice {
            instructions: "어느 팀?".into(),
            options: vec!["billing".into(), "technical".into()],
        };
        let state = "s";
        let (question_span, option_spans) = spans(state, &q);
        let text = schema_text(state, &q);
        assert_eq!(&text[question_span.start_char..question_span.end_char], "어느 팀?");
        assert_eq!(option_spans.len(), 2);
        assert_eq!(&text[option_spans[0].start_char..option_spans[0].end_char], "billing");
        assert_eq!(&text[option_spans[1].start_char..option_spans[1].end_char], "technical");
    }
}
```

- [ ] **Step 2: 테스트가 실패하는지 확인한다**

Run: `cargo test --manifest-path crates/decide/Cargo.toml local::tokenizer:: 2>&1 | head -30`
Expected: FAIL — `schema_text`, `spans`, `truncate_state`, `MAX_STATE_CHARS`가 정의되지 않음.

- [ ] **Step 3: 최소 구현을 작성한다**

```rust
// crates/decide/src/local/tokenizer.rs
use crate::protocol::Question;

pub const MAX_STATE_CHARS: usize = 48_000;

pub fn truncate_state(state: &str) -> &str {
    match state.char_indices().nth(MAX_STATE_CHARS) {
        Some((byte_idx, _)) => &state[..byte_idx],
        None => state,
    }
}

pub struct QuestionSpan {
    pub label: String,
    pub start_char: usize,
    pub end_char: usize,
}

pub struct OptionSpan {
    pub label: String,
    pub start_char: usize,
    pub end_char: usize,
}

fn kind_name(question: &Question) -> &'static str {
    match question {
        Question::Choice { .. } => "choice",
        Question::Score { .. } => "score",
        Question::Noul { .. } => "noul",
    }
}

fn instructions(question: &Question) -> &str {
    match question {
        Question::Choice { instructions, .. } => instructions,
        Question::Score { instructions, .. } => instructions,
        Question::Noul { instructions } => instructions,
    }
}

fn option_labels(question: &Question) -> Vec<&str> {
    match question {
        Question::Choice { options, .. } => options.iter().map(String::as_str).collect(),
        Question::Score { criteria, .. } => criteria.iter().map(String::as_str).collect(),
        Question::Noul { .. } => Vec::new(),
    }
}

pub fn schema_text(state: &str, question: &Question) -> String {
    let state = truncate_state(state);
    let mut text = String::new();
    text.push_str("FIELD q\n");
    text.push_str("ID:q\n");
    text.push_str(&format!("TYPE:{}\n", kind_name(question)));
    text.push_str(&format!("INSTRUCTION:{}\n", instructions(question)));
    for label in option_labels(question) {
        text.push_str(&format!("OPTION:{label}\n"));
    }
    let state_json = serde_json::to_string(state).unwrap_or_else(|_| "\"\"".to_string());
    text.push_str("STATE:");
    text.push_str(&state_json);
    text
}

pub fn spans(state: &str, question: &Question) -> (QuestionSpan, Vec<OptionSpan>) {
    let text = schema_text(state, question);
    let instr = instructions(question);
    let instr_start = text.find(instr).expect("instructions 텍스트가 schema_text 안에 있어야 한다");
    let question_span = QuestionSpan {
        label: "q".to_string(),
        start_char: instr_start,
        end_char: instr_start + instr.len(),
    };
    let mut option_spans = Vec::new();
    let mut search_from = instr_start + instr.len();
    for label in option_labels(question) {
        let marker = format!("OPTION:{label}");
        let marker_pos = text[search_from..].find(&marker).expect("옵션 마커를 찾을 수 없다") + search_from;
        let label_start = marker_pos + "OPTION:".len();
        let label_end = label_start + label.len();
        option_spans.push(OptionSpan {
            label: label.to_string(),
            start_char: label_start,
            end_char: label_end,
        });
        search_from = label_end;
    }
    (question_span, option_spans)
}
```

- [ ] **Step 4: 테스트를 다시 돌려 통과를 확인한다**

Run: `cargo test --manifest-path crates/decide/Cargo.toml local::tokenizer::`
Expected: 6개 테스트 모두 PASS.

- [ ] **Step 5: 커밋**

```bash
git add crates/decide/src/local/tokenizer.rs
git commit -m "feat(decide): Clef 스키마 텍스트 조립과 질문/옵션 스팬 계산을 추가한다"
```

---

### Task 3: 백본 벤더링 (`backbone.rs`) — GGUF 로드와 hidden_states forward

**Files:**
- Create: `crates/decide/src/local/backbone.rs`
- Test: `crates/decide/src/local/backbone.rs` (인라인, 가중치 없이 돌 수 있는 범위만)

**Interfaces:**
- Consumes: `candle_core::{Tensor, Device, DType, Result as CandleResult}` (crates.io `candle-core`)
- Produces: `pub struct Backbone { /* PR #3396 quantized_qwen3_5.rs의 ModelWeights와 동형 */ }`
- Produces: `impl Backbone { pub fn from_gguf_path(path: &std::path::Path) -> Result<Self, String>; pub fn hidden_states(&mut self, input_ids: &Tensor) -> Result<Tensor, String>; pub fn hidden_size(&self) -> usize; }`
  - `hidden_states`는 `(batch, seq_len, hidden_size)` 텐서를 반환한다 — PR 원본의 `forward`(마지막 토큰만 `narrow`해 `lm_head` 통과)는 쓰지 않는다.

PR #3396의 `candle-transformers/src/models/quantized_qwen3_5.rs`(2026-03-06 커밋 `e9e4929f3e31b6183cc17e7be0596929c6725257`, `woodRock/candle` 포크)를 출발점으로 삼는다. 이 소스는 `candle-transformers`의 비공개 유틸(`with_tracing::QMatMul`, `utils::repeat_kv`, `quantized_nn::RmsNorm`)에 의존한다 — `candle-transformers` 전체를 의존성으로 끌어오지 않기로 했으므로(Task 1), 이 세 유틸을 벤더링 파일 안에 함께 복제한다.

- [ ] **Step 1: PR 원본 파일을 받아 벤더링 기준 코드를 로컬에 둔다**

```bash
curl -s "https://raw.githubusercontent.com/woodRock/candle/e9e4929f3e31b6183cc17e7be0596929c6725257/candle-transformers/src/models/quantized_qwen3_5.rs" -o /tmp/clef-vendor-quantized_qwen3_5.rs
wc -l /tmp/clef-vendor-quantized_qwen3_5.rs
```

Expected: 741줄. 이 파일을 참고 자료로 두고 아래 단계에서 `backbone.rs`에 적응시켜 옮긴다.

- [ ] **Step 2: `with_tracing::QMatMul`, `utils::repeat_kv`, `quantized_nn::RmsNorm`을 직접 조회해 필요한 부분만 가져온다**

```bash
curl -s "https://raw.githubusercontent.com/huggingface/candle/main/candle-transformers/src/models/with_tracing.rs" | grep -n "pub struct QMatMul\|impl QMatMul" -A 15
curl -s "https://raw.githubusercontent.com/huggingface/candle/main/candle-transformers/src/utils.rs" | grep -n "pub fn repeat_kv" -A 15
curl -s "https://raw.githubusercontent.com/huggingface/candle/main/candle-transformers/src/quantized_nn.rs" | grep -n "pub struct RmsNorm\|impl RmsNorm" -A 20
```

이 출력을 그대로 참고해 `backbone.rs` 상단에 `mod vendored_utils { ... }` 서브모듈로 세 가지를 옮긴다 — `QMatMul`은 `candle_core::quantized::QMatMul`(candle-core에 재노출돼 있음)을 직접 쓸 수 있는지 먼저 확인하고, 안 되면 `with_tracing.rs`의 래퍼를 그대로 복제한다. `repeat_kv`는 순수 함수라 그대로 복제한다. `quantized_nn::RmsNorm`은 `QTensor` 기반 생성자(`from_qtensor`)가 필요하므로 그대로 복제한다.

- [ ] **Step 3: `backbone.rs`에 `ModelWeights`를 `Backbone`으로 적응시켜 작성한다**

구조체·필드·생성자는 `/tmp/clef-vendor-quantized_qwen3_5.rs`의 `Gguf`, `MlpWeights`, `RotaryEmbedding`, `AttentionWeights`, `GatedDeltaNetWeights`, `LayerWeights`, `ModelWeights`를 그대로 가져오되, 아래 한 가지만 바꾼다 — `ModelWeights::forward`를 복제하지 않고 대신:

```rust
impl Backbone {
    pub fn from_gguf_path(path: &std::path::Path) -> Result<Self, String> {
        let mut file = std::fs::File::open(path)
            .map_err(|err| format!("GGUF 파일을 열 수 없습니다: {err}"))?;
        let content = candle_core::quantized::gguf_file::Content::read(&mut file)
            .map_err(|err| format!("GGUF 헤더를 읽을 수 없습니다: {err}"))?;
        let device = candle_core::Device::Cpu;
        let inner = ModelWeights::from_gguf(content, &mut file, &device)
            .map_err(|err| format!("Clef-flash 백본 로드에 실패했습니다: {err}"))?;
        Ok(Self { inner })
    }

    pub fn hidden_size(&self) -> usize {
        self.inner.hidden_size()
    }

    /// PR #3396 ModelWeights::forward는 norm 통과 뒤 마지막 토큰만 narrow해
    /// lm_head에 통과시킨다. joint_head는 전체 시퀀스가 필요하므로 그 narrow와
    /// lm_head 호출 이전 단계에서 멈추는 이 메서드를 쓴다.
    pub fn hidden_states(&mut self, input_ids: &candle_core::Tensor) -> Result<candle_core::Tensor, String> {
        let (b, l) = input_ids.dims2().map_err(|err| err.to_string())?;
        let mut h = self.inner.embed_tokens.forward(input_ids).map_err(|err| err.to_string())?;
        let causal_mask = if l == 1 {
            None
        } else {
            Some(self.inner.causal_mask(b, l, 0).map_err(|err| err.to_string())?)
        };
        for layer in &mut self.inner.layers {
            h = layer.forward(&h, causal_mask.as_ref(), 0).map_err(|err| err.to_string())?;
        }
        self.inner.norm.forward(&h).map_err(|err| err.to_string())
    }
}
```

`ModelWeights`의 `embed_tokens`, `layers`, `norm`, `causal_mask`가 전부 `private`(모듈 밖에서 안 보임)이므로, `Backbone`과 `ModelWeights`를 같은 파일(`backbone.rs`)에 두어 모듈 내부 접근으로 해결한다 — `causal_mask`도 `pub(super)` 또는 가시성 조정이 필요하면 그 자리에서 바꾼다(원본은 `private fn`, 같은 파일이면 그대로 호출 가능).

`hidden_size()`는 원본에 없는 메서드이므로 `ModelWeights`에 다음을 추가한다:

```rust
impl ModelWeights {
    pub fn hidden_size(&self) -> usize {
        self.embed_tokens.hidden_size() // candle_nn::Embedding에 없으면 생성 시점에 별도 필드로 저장
    }
}
```

(`candle_nn::Embedding`에 `hidden_size()`가 없다면, `ModelWeights`에 `hidden_size: usize` 필드를 추가해 `from_gguf`에서 `hidden_size` 지역 변수를 저장해 두는 쪽으로 바꾼다 — 원본 코드에 이미 `let hidden_size = md_get(...)` 지역 변수가 있으므로 구조체 필드로 승격만 하면 된다.)

- [ ] **Step 4: GGUF 메타데이터 키 이름을 실제 Clef-flash 파일로 검증한다**

```bash
curl -sL "https://huggingface.co/prithivMLmods/clef-flash-GGUF/resolve/main/clef-flash.Q4_K_M.gguf" -r 0-1048576 -o /tmp/clef-flash-head.gguf
```

(GGUF 헤더는 파일 앞부분에 있으므로 전체를 받지 않고 처음 1MB만 받아 메타데이터를 읽는다. candle의 `gguf_file::Content::read`가 이 조각으로 메타데이터까지 파싱되는지 Rust REPL 대신 작은 테스트 바이너리로 확인한다 — 메타데이터 섹션이 1MB를 넘으면 범위를 늘린다.)

```rust
// crates/decide/examples/inspect_gguf.rs (임시 — 확인 후 Step 6에서 삭제)
fn main() {
    let mut file = std::fs::File::open("/tmp/clef-flash-head.gguf").unwrap();
    let content = candle_core::quantized::gguf_file::Content::read(&mut file).unwrap();
    let mut keys: Vec<_> = content.metadata.keys().collect();
    keys.sort();
    for k in keys {
        println!("{k} = {:?}", content.metadata.get(k));
    }
}
```

Run: `cargo run --manifest-path crates/decide/Cargo.toml --example inspect_gguf`
Expected: 출력에 `qwen3.*` 또는 `qwen35.*` 접두사 키들이 보인다. 벤더링한 `md_get`의 폴백 로직(`qwen3.` → `qwen35.` 치환)이 실제로 필요한지, 아니면 처음부터 `qwen35.*`로만 오는지 이 출력으로 확정한다 — 다르면 `md_get`을 실제 키 이름에 맞게 고친다.

- [ ] **Step 5: 더미 입력으로 컴파일·타입 체크만 되는 테스트를 추가한다**

실제 GGUF 로드 없이는 `Backbone`을 생성할 수 없으므로(생성자가 `from_gguf_path`뿐), 이 단계의 테스트는 헬퍼 함수 단위로 좁힌다:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use candle_core::{DType, Device, Tensor};

    #[test]
    fn rotary_embedding_apply_preserves_shape() {
        let device = Device::Cpu;
        let rotary = RotaryEmbedding::new(DType::F32, 128, 4096, 10_000_000.0, &device).unwrap();
        let q = Tensor::zeros((1, 2, 3, 128), DType::F32, &device).unwrap();
        let k = Tensor::zeros((1, 2, 3, 128), DType::F32, &device).unwrap();
        let (q2, k2) = rotary.apply(&q, &k, 0).unwrap();
        assert_eq!(q2.dims(), q.dims());
        assert_eq!(k2.dims(), k.dims());
    }

    #[test]
    fn repeat_kv_identity_when_groups_is_one() {
        let device = Device::Cpu;
        let x = Tensor::zeros((1, 4, 3, 8), DType::F32, &device).unwrap();
        let out = repeat_kv(x.clone(), 1).unwrap();
        assert_eq!(out.dims(), x.dims());
    }
}
```

- [ ] **Step 6: 임시 조사용 example을 지우고, 가중치 없이 돌아가는 테스트만 돌린다**

```bash
rm crates/decide/examples/inspect_gguf.rs
```

Run: `cargo test --manifest-path crates/decide/Cargo.toml local::backbone::`
Expected: 두 테스트 PASS.

- [ ] **Step 7: 커밋**

```bash
git add crates/decide/src/local/backbone.rs
git commit -m "feat(decide): PR #3396 Qwen3.5 GGUF 백본을 벤더링해 hidden_states 추출 경로를 추가한다"
```

---

### Task 4: joint_head.rs — EvidenceRoutingLayer와 TransformerDecoderLayer

**Files:**
- Create: `crates/decide/src/local/joint_head.rs`
- Test: `crates/decide/src/local/joint_head.rs` (인라인, 랜덤 가중치로 shape만 검사)

**Interfaces:**
- Consumes: `candle_core::Tensor` (Task 3의 `Backbone::hidden_states` 출력), `tokenizer::QuestionSpan`/`OptionSpan`(Task 2)
- Produces: `pub struct JointHead { /* width=1024, routing_layers=2, layers=4, heads=16, feedforward=4096 */ }`
- Produces: `impl JointHead { pub fn load(safetensors_path: &std::path::Path, hidden_size: usize) -> Result<Self, String>; pub fn score(&self, hidden_states: &Tensor, input_ids: &Tensor, question_span: &QuestionSpan, option_spans: &[OptionSpan], token_offsets: &[(usize, usize)]) -> Result<Vec<f32>, String>; }`
  - `token_offsets`는 토크나이저가 각 토큰이 원문 문자열의 몇 번째~몇 번째 문자에 대응하는지 알려주는 오프셋 목록이다(HuggingFace `tokenizers` crate의 `Encoding::get_offsets()`가 그대로 제공) — 문자 스팬(Task 2)을 토큰 스팬으로 바꾸는 데 쓴다.
  - `score`의 반환값은 질문의 옵션 개수와 같은 길이의 `Vec<f32>` 로짓이다. noul은 길이 1(참 로짓 하나)로 약속한다.

조사된 `JointSchemaHead` 레이어 구성(모델 카드 설명 기준)을 그대로 구현한다: `hidden_norm`(LayerNorm) → 6개 bias 없는 `Linear` 투영(`memory_projection`, `question_projection`, `option_question_projection`, `global_projection`, `option_context_projection`, `option_lexical_projection`) → `type_embedding: Embedding(3, width)` → `EvidenceRoutingLayer` × `routing_layers`(각 레이어: `query_norm`/`memory_norm` LayerNorm + `MultiheadAttention` + `feedforward_norm` LayerNorm + Linear→GELU→Dropout→Linear→Dropout) → `option_summary_norm`(LayerNorm) → `TransformerDecoderLayer`(candle_nn 표준) × `layers` → `field_norm`/`option_norm`(LayerNorm) → `residual_scorer`(Linear(width*4→width)→GELU→Dropout→Linear(width→1)) → 스칼라 `prior_logit_scale`/`joint_logit_scale`/`residual_gate`.

- [ ] **Step 1: 실패하는 shape 테스트를 먼저 쓴다**

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use candle_core::{DType, Device, Tensor};
    use candle_nn::VarBuilder;

    fn random_head(device: &Device) -> JointHead {
        let varmap = candle_nn::VarMap::new();
        let vb = VarBuilder::from_varmap(&varmap, DType::F32, device);
        JointHead::new_for_test(vb, 4096, 1024, 2, 4, 16, 4096).unwrap()
    }

    #[test]
    fn score_returns_one_logit_per_option() {
        let device = Device::Cpu;
        let head = random_head(&device);
        let seq_len = 20;
        let hidden = Tensor::randn(0f32, 1f32, (1, seq_len, 4096), &device).unwrap();
        let input_ids = Tensor::zeros((1, seq_len), DType::U32, &device).unwrap();
        let question_span = crate::local::tokenizer::QuestionSpan {
            label: "q".into(),
            start_char: 0,
            end_char: 5,
        };
        let option_spans = vec![
            crate::local::tokenizer::OptionSpan { label: "a".into(), start_char: 6, end_char: 7 },
            crate::local::tokenizer::OptionSpan { label: "b".into(), start_char: 8, end_char: 9 },
        ];
        let token_offsets: Vec<(usize, usize)> = (0..seq_len).map(|i| (i, i + 1)).collect();
        let logits = head
            .score(&hidden, &input_ids, &question_span, &option_spans, &token_offsets)
            .unwrap();
        assert_eq!(logits.len(), 2);
    }

    #[test]
    fn noul_question_returns_single_logit() {
        let device = Device::Cpu;
        let head = random_head(&device);
        let seq_len = 10;
        let hidden = Tensor::randn(0f32, 1f32, (1, seq_len, 4096), &device).unwrap();
        let input_ids = Tensor::zeros((1, seq_len), DType::U32, &device).unwrap();
        let question_span = crate::local::tokenizer::QuestionSpan {
            label: "q".into(),
            start_char: 0,
            end_char: 5,
        };
        let token_offsets: Vec<(usize, usize)> = (0..seq_len).map(|i| (i, i + 1)).collect();
        let logits = head.score(&hidden, &input_ids, &question_span, &[], &token_offsets).unwrap();
        assert_eq!(logits.len(), 1);
    }
}
```

- [ ] **Step 2: 테스트가 실패하는지 확인한다**

Run: `cargo test --manifest-path crates/decide/Cargo.toml local::joint_head:: 2>&1 | head -30`
Expected: FAIL — `JointHead`, `new_for_test`, `score`가 없음.

- [ ] **Step 3: 레이어 구조를 구현한다**

```rust
// crates/decide/src/local/joint_head.rs
use crate::local::tokenizer::{OptionSpan, QuestionSpan};
use candle_core::{DType, Result as CandleResult, Tensor, D};
use candle_nn::{
    layer_norm, linear_no_bias, Embedding, LayerNorm, Linear, Module, VarBuilder,
};

struct EvidenceRoutingLayer {
    query_norm: LayerNorm,
    memory_norm: LayerNorm,
    attn: candle_nn::MultiheadAttention,
    feedforward_norm: LayerNorm,
    ff1: Linear,
    ff2: Linear,
}

impl EvidenceRoutingLayer {
    fn new(width: usize, heads: usize, feedforward: usize, vb: VarBuilder) -> CandleResult<Self> {
        Ok(Self {
            query_norm: layer_norm(width, 1e-5, vb.pp("query_norm"))?,
            memory_norm: layer_norm(width, 1e-5, vb.pp("memory_norm"))?,
            attn: candle_nn::MultiheadAttention::new(width, heads, vb.pp("attn"))?,
            feedforward_norm: layer_norm(width, 1e-5, vb.pp("feedforward_norm"))?,
            ff1: linear_no_bias(width, feedforward, vb.pp("ff1"))?,
            ff2: linear_no_bias(feedforward, width, vb.pp("ff2"))?,
        })
    }

    fn forward(&self, query: &Tensor, memory: &Tensor) -> CandleResult<Tensor> {
        let q = self.query_norm.forward(query)?;
        let m = self.memory_norm.forward(memory)?;
        let attended = self.attn.forward(&q, &m, &m, None)?;
        let query = (query + attended)?;
        let normed = self.feedforward_norm.forward(&query)?;
        let ff = self.ff2.forward(&self.ff1.forward(&normed)?.gelu()?)?;
        query + ff
    }
}

pub struct JointHead {
    hidden_norm: LayerNorm,
    memory_projection: Linear,
    question_projection: Linear,
    option_question_projection: Linear,
    global_projection: Linear,
    option_context_projection: Linear,
    option_lexical_projection: Linear,
    type_embedding: Embedding,
    evidence_layers: Vec<EvidenceRoutingLayer>,
    option_summary_norm: LayerNorm,
    decoder_layers: Vec<candle_nn::transformer::TransformerDecoderLayer>,
    field_norm: LayerNorm,
    option_norm: LayerNorm,
    residual_scorer_1: Linear,
    residual_scorer_2: Linear,
    width: usize,
}

impl JointHead {
    #[cfg(test)]
    pub fn new_for_test(
        vb: VarBuilder,
        hidden_size: usize,
        width: usize,
        routing_layers: usize,
        layers: usize,
        heads: usize,
        feedforward: usize,
    ) -> CandleResult<Self> {
        Self::new(vb, hidden_size, width, routing_layers, layers, heads, feedforward)
    }

    fn new(
        vb: VarBuilder,
        hidden_size: usize,
        width: usize,
        routing_layers: usize,
        layers: usize,
        heads: usize,
        feedforward: usize,
    ) -> CandleResult<Self> {
        let hidden_norm = layer_norm(hidden_size, 1e-5, vb.pp("hidden_norm"))?;
        let memory_projection = linear_no_bias(hidden_size, width, vb.pp("memory_projection"))?;
        let question_projection = linear_no_bias(hidden_size, width, vb.pp("question_projection"))?;
        let option_question_projection =
            linear_no_bias(hidden_size, width, vb.pp("option_question_projection"))?;
        let global_projection = linear_no_bias(hidden_size, width, vb.pp("global_projection"))?;
        let option_context_projection =
            linear_no_bias(hidden_size, width, vb.pp("option_context_projection"))?;
        let option_lexical_projection =
            linear_no_bias(hidden_size, width, vb.pp("option_lexical_projection"))?;
        let type_embedding = candle_nn::embedding(3, width, vb.pp("type_embedding"))?;

        let mut evidence_layers = Vec::with_capacity(routing_layers);
        let vb_evidence = vb.pp("evidence_layers");
        for i in 0..routing_layers {
            evidence_layers.push(EvidenceRoutingLayer::new(width, heads, feedforward, vb_evidence.pp(i))?);
        }

        let option_summary_norm = layer_norm(width, 1e-5, vb.pp("option_summary_norm"))?;

        let mut decoder_layers = Vec::with_capacity(layers);
        let vb_layers = vb.pp("layers");
        for i in 0..layers {
            decoder_layers.push(candle_nn::transformer::TransformerDecoderLayer::new(
                width,
                heads,
                feedforward,
                vb_layers.pp(i),
            )?);
        }

        let field_norm = layer_norm(width, 1e-5, vb.pp("field_norm"))?;
        let option_norm = layer_norm(width, 1e-5, vb.pp("option_norm"))?;
        let residual_scorer_1 = linear_no_bias(width * 4, width, vb.pp("residual_scorer_1"))?;
        let residual_scorer_2 = linear_no_bias(width, 1, vb.pp("residual_scorer_2"))?;

        Ok(Self {
            hidden_norm,
            memory_projection,
            question_projection,
            option_question_projection,
            global_projection,
            option_context_projection,
            option_lexical_projection,
            type_embedding,
            evidence_layers,
            option_summary_norm,
            decoder_layers,
            field_norm,
            option_norm,
            residual_scorer_1,
            residual_scorer_2,
            width,
        })
    }

    pub fn load(safetensors_path: &std::path::Path, hidden_size: usize) -> Result<Self, String> {
        let device = candle_core::Device::Cpu;
        let vb = unsafe {
            VarBuilder::from_mmaped_safetensors(&[safetensors_path], DType::F32, &device)
                .map_err(|err| format!("joint_head.safetensors를 읽을 수 없습니다: {err}"))?
        };
        Self::new(vb, hidden_size, 1024, 2, 4, 16, 4096)
            .map_err(|err| format!("joint_head 레이어 구성에 실패했습니다: {err}"))
    }

    fn char_span_to_token_span(
        token_offsets: &[(usize, usize)],
        start_char: usize,
        end_char: usize,
    ) -> (usize, usize) {
        let start = token_offsets
            .iter()
            .position(|&(s, e)| s <= start_char && start_char < e)
            .unwrap_or(0);
        let end = token_offsets
            .iter()
            .rposition(|&(s, e)| s < end_char && end_char <= e)
            .map(|i| i + 1)
            .unwrap_or(token_offsets.len());
        (start, end.max(start + 1))
    }

    fn span_mean(hidden_states: &Tensor, start: usize, end: usize) -> CandleResult<Tensor> {
        hidden_states
            .narrow(1, start, end.saturating_sub(start).max(1))?
            .mean(1)
    }

    pub fn score(
        &self,
        hidden_states: &Tensor,
        _input_ids: &Tensor,
        question_span: &QuestionSpan,
        option_spans: &[OptionSpan],
        token_offsets: &[(usize, usize)],
    ) -> Result<Vec<f32>, String> {
        let run = || -> CandleResult<Vec<f32>> {
            let normed = self.hidden_norm.forward(hidden_states)?;
            let memory = self.memory_projection.forward(&normed)?;

            let (q_start, q_end) =
                Self::char_span_to_token_span(token_offsets, question_span.start_char, question_span.end_char);
            let question_vec = Self::span_mean(&normed, q_start, q_end)?;
            let question_vec = self.question_projection.forward(&question_vec)?;

            let num_options = option_spans.len().max(1);
            let mut option_vecs = Vec::with_capacity(num_options);
            if option_spans.is_empty() {
                option_vecs.push(question_vec.clone());
            } else {
                for span in option_spans {
                    let (o_start, o_end) =
                        Self::char_span_to_token_span(token_offsets, span.start_char, span.end_char);
                    let option_context = Self::span_mean(&normed, o_start, o_end)?;
                    option_vecs.push(self.option_context_projection.forward(&option_context)?);
                }
            }

            let mut logits = Vec::with_capacity(num_options);
            for option_vec in &option_vecs {
                let query = (&question_vec + option_vec)?.unsqueeze(1)?;
                let mut routed = query;
                for layer in &self.evidence_layers {
                    routed = layer.forward(&routed, &memory)?;
                }
                let summary = self.option_summary_norm.forward(&routed)?;
                let mut decoded = summary;
                for layer in &self.decoder_layers {
                    decoded = layer.forward(&decoded, &memory, None)?;
                }
                let field = self.field_norm.forward(&decoded)?;
                let option_rep = self.option_norm.forward(option_vec)?.unsqueeze(1)?;
                let combined = Tensor::cat(&[&field, &option_rep, &field, &option_rep], D::Minus1)?;
                let combined = combined.reshape((1, self.width * 4))?;
                let hidden = self.residual_scorer_1.forward(&combined)?.gelu()?;
                let score = self.residual_scorer_2.forward(&hidden)?;
                logits.push(score.flatten_all()?.to_vec1::<f32>()?[0]);
            }
            Ok(logits)
        };
        run().map_err(|err| format!("joint_head 추론에 실패했습니다: {err}"))
    }
}
```

(`candle_nn::MultiheadAttention`과 `candle_nn::transformer::TransformerDecoderLayer`가 설치된 candle-nn 0.9 버전에 정확히 이 이름·시그니처로 없을 수 있다 — 있다면 이 코드가 그대로 컴파일되고, 없다면 `candle_nn::ops::softmax`와 `Linear`로 scaled-dot-product attention을 직접 구성한 대체 구현으로 바꾼다. 어느 쪽이든 `EvidenceRoutingLayer::forward`와 디코더 레이어의 입출력 shape 계약(`(batch, 1, width)` 쿼리 ↔ `(batch, seq_len, width)` 메모리 → `(batch, 1, width)`)은 유지한다.)

- [ ] **Step 4: 테스트를 다시 돌려 통과를 확인한다**

Run: `cargo test --manifest-path crates/decide/Cargo.toml local::joint_head::`
Expected: 두 테스트 PASS. (candle-nn API 불일치로 컴파일 에러가 나면 Step 3 괄호의 대체 구현으로 바꾼 뒤 재시도.)

- [ ] **Step 5: 커밋**

```bash
git add crates/decide/src/local/joint_head.rs
git commit -m "feat(decide): joint schema head(EvidenceRoutingLayer + TransformerDecoderLayer)를 구현한다"
```

---

### Task 5: 응답 변환 (`postprocess.rs`)

**Files:**
- Create: `crates/decide/src/local/postprocess.rs`
- Test: `crates/decide/src/local/postprocess.rs` (인라인)

**Interfaces:**
- Consumes: `Vec<f32>` 로짓(Task 4의 `JointHead::score` 출력), `crate::protocol::Question`
- Produces: `pub fn to_answer(question: &Question, logits: &[f32]) -> serde_json::Value` — TypeSafe 응답과 같은 필드 틀(`noul`/`choice`+`confidence`+`probabilities`/`score`+`confidence`+`legend`+`probabilities`)을 만든다.

- [ ] **Step 1: 실패하는 테스트를 먼저 쓴다**

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::Question;
    use serde_json::json;

    #[test]
    fn noul_logit_becomes_sigmoid_probability() {
        let q = Question::Noul { instructions: "참인가?".into() };
        let answer = to_answer(&q, &[0.0]);
        assert_eq!(answer["type"], "noul");
        let noul = answer["noul"].as_f64().unwrap();
        assert!((noul - 0.5).abs() < 1e-6);
    }

    #[test]
    fn choice_logits_become_softmax_with_argmax_choice() {
        let q = Question::Choice {
            instructions: "어느 팀?".into(),
            options: vec!["billing".into(), "technical".into()],
        };
        let answer = to_answer(&q, &[2.0, 0.0]);
        assert_eq!(answer["type"], "choice");
        assert_eq!(answer["choice"], "billing");
        let probs = &answer["probabilities"];
        assert!(probs["billing"].as_f64().unwrap() > probs["technical"].as_f64().unwrap());
        let sum = probs["billing"].as_f64().unwrap() + probs["technical"].as_f64().unwrap();
        assert!((sum - 1.0).abs() < 1e-6);
    }

    #[test]
    fn score_logits_become_expected_value_over_levels() {
        let q = Question::Score {
            instructions: "강도?".into(),
            criteria: vec!["낮음".into(), "중간".into(), "높음".into()],
        };
        let answer = to_answer(&q, &[0.0, 0.0, 10.0]);
        assert_eq!(answer["type"], "score");
        let score = answer["score"].as_f64().unwrap();
        assert!(score > 1.5, "거의 확실히 '높음'(인덱스 2)에 쏠려야 한다: {score}");
        assert_eq!(answer["legend"], json!(["낮음", "중간", "높음"]));
    }
}
```

- [ ] **Step 2: 테스트가 실패하는지 확인한다**

Run: `cargo test --manifest-path crates/decide/Cargo.toml local::postprocess:: 2>&1 | head -20`
Expected: FAIL — `to_answer` 없음.

- [ ] **Step 3: 구현한다**

```rust
// crates/decide/src/local/postprocess.rs
use crate::protocol::Question;
use serde_json::{json, Map, Value};

fn softmax(logits: &[f32]) -> Vec<f64> {
    let max = logits.iter().cloned().fold(f32::NEG_INFINITY, f32::max) as f64;
    let exps: Vec<f64> = logits.iter().map(|&x| ((x as f64) - max).exp()).collect();
    let sum: f64 = exps.iter().sum();
    exps.into_iter().map(|v| v / sum).collect()
}

fn round4(x: f64) -> f64 {
    (x * 10_000.0).round() / 10_000.0
}

pub fn to_answer(question: &Question, logits: &[f32]) -> Value {
    match question {
        Question::Noul { .. } => {
            let p = 1.0 / (1.0 + (-logits[0] as f64).exp());
            json!({"type": "noul", "noul": round4(p)})
        }
        Question::Choice { options, .. } => {
            let probs = softmax(logits);
            let (best_idx, _) = probs
                .iter()
                .enumerate()
                .max_by(|a, b| a.1.partial_cmp(b.1).unwrap())
                .unwrap();
            let mut probabilities = Map::new();
            for (option, p) in options.iter().zip(probs.iter()) {
                probabilities.insert(option.clone(), json!(round4(*p)));
            }
            json!({
                "type": "choice",
                "choice": options[best_idx],
                "confidence": round4(probs[best_idx]),
                "probabilities": Value::Object(probabilities),
            })
        }
        Question::Score { criteria, .. } => {
            let probs = softmax(logits);
            let expected: f64 = probs.iter().enumerate().map(|(i, p)| i as f64 * p).sum();
            let (best_idx, _) = probs
                .iter()
                .enumerate()
                .max_by(|a, b| a.1.partial_cmp(b.1).unwrap())
                .unwrap();
            let mut probabilities = Map::new();
            for (level, p) in criteria.iter().zip(probs.iter()) {
                probabilities.insert(level.clone(), json!(round4(*p)));
            }
            json!({
                "type": "score",
                "score": round4(expected),
                "confidence": round4(probs[best_idx]),
                "legend": criteria,
                "probabilities": Value::Object(probabilities),
            })
        }
    }
}
```

- [ ] **Step 4: 테스트를 다시 돌려 통과를 확인한다**

Run: `cargo test --manifest-path crates/decide/Cargo.toml local::postprocess::`
Expected: 세 테스트 PASS.

- [ ] **Step 5: 커밋**

```bash
git add crates/decide/src/local/postprocess.rs
git commit -m "feat(decide): 로컬 로짓을 TypeSafe와 같은 응답 틀로 변환한다"
```

---

### Task 6: 가중치 다운로드와 토크나이저 연결

**Files:**
- Modify: `crates/decide/src/local/mod.rs`
- Test: `crates/decide/src/local/mod.rs` (인라인, 네트워크 필요한 테스트는 `#[ignore]`)

**Interfaces:**
- Consumes: `local::weights_dir()` (Task 1), `local::backbone::Backbone` (Task 3), `local::joint_head::JointHead` (Task 4)
- Produces: `pub fn ensure_weights() -> Result<(std::path::PathBuf, std::path::PathBuf), String>` — `(백본 GGUF 경로, joint_head.safetensors 경로)`를 돌려준다. `CLEF_WEIGHTS` 디렉터리에 이미 있으면 그대로, 없으면 `hf-hub` crate로 1회 받는다.
- Produces: `pub struct Tokenizer` 래퍼 — `tokenizers::Tokenizer`를 감싸 `encode(&self, text: &str) -> Result<(Vec<u32>, Vec<(usize, usize)>), String>`(토큰 id열과 문자 오프셋열)을 제공한다.

- [ ] **Step 1: 실패하는 테스트를 먼저 쓴다**

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ensure_weights_finds_files_in_clef_weights_dir() {
        let dir = std::env::temp_dir().join(format!("clef-weights-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("clef-flash.Q4_K_M.gguf"), b"fake").unwrap();
        std::fs::write(dir.join("joint_head.safetensors"), b"fake").unwrap();
        std::env::set_var("CLEF_WEIGHTS", &dir);

        let (backbone_path, head_path) = ensure_weights().unwrap();
        assert_eq!(backbone_path, dir.join("clef-flash.Q4_K_M.gguf"));
        assert_eq!(head_path, dir.join("joint_head.safetensors"));

        std::env::remove_var("CLEF_WEIGHTS");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn ensure_weights_errors_when_files_missing() {
        let dir = std::env::temp_dir().join(format!("clef-weights-empty-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::env::set_var("CLEF_WEIGHTS", &dir);

        let err = ensure_weights().unwrap_err();
        assert!(err.contains("찾을 수 없습니다") || err.contains("다운로드"));

        std::env::remove_var("CLEF_WEIGHTS");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    #[ignore] // 실제 네트워크로 HuggingFace에서 수 GB를 받는다 — CI 기본 실행에서 제외
    fn ensure_weights_downloads_when_cache_empty() {
        std::env::remove_var("CLEF_WEIGHTS");
        let (backbone_path, head_path) = ensure_weights().unwrap();
        assert!(backbone_path.exists());
        assert!(head_path.exists());
    }
}
```

- [ ] **Step 2: 테스트가 실패하는지 확인한다**

Run: `cargo test --manifest-path crates/decide/Cargo.toml local::ensure_weights 2>&1 | head -20`
Expected: FAIL — `ensure_weights` 없음.

- [ ] **Step 3: 구현한다**

```rust
// crates/decide/src/local/mod.rs 에 추가
pub fn ensure_weights() -> Result<(PathBuf, PathBuf), String> {
    let dir = weights_dir()?;
    let backbone_path = dir.join("clef-flash.Q4_K_M.gguf");
    let head_path = dir.join("joint_head.safetensors");
    if backbone_path.exists() && head_path.exists() {
        return Ok((backbone_path, head_path));
    }
    if std::env::var("CLEF_WEIGHTS").is_ok() {
        return Err(format!(
            "CLEF_WEIGHTS={}에서 가중치 파일을 찾을 수 없습니다 (clef-flash.Q4_K_M.gguf, joint_head.safetensors 필요)",
            dir.display()
        ));
    }
    download_weights(&dir)
}

fn download_weights(dir: &std::path::Path) -> Result<(PathBuf, PathBuf), String> {
    use hf_hub::api::sync::Api;
    std::fs::create_dir_all(dir).map_err(|err| format!("가중치 디렉터리를 만들 수 없습니다: {err}"))?;
    let api = Api::new().map_err(|err| format!("HuggingFace API 초기화에 실패했습니다: {err}"))?;

    let backbone_repo = api.model("prithivMLmods/clef-flash-GGUF".to_string());
    let backbone_src = backbone_repo
        .get("clef-flash.Q4_K_M.gguf")
        .map_err(|err| format!("백본 GGUF 다운로드에 실패했습니다: {err}"))?;
    let backbone_dst = dir.join("clef-flash.Q4_K_M.gguf");
    std::fs::copy(&backbone_src, &backbone_dst).map_err(|err| format!("백본 파일 복사에 실패했습니다: {err}"))?;

    let head_repo = api.model("Cloudflare/clef-flash".to_string());
    let head_src = head_repo
        .get("joint_head.safetensors")
        .map_err(|err| format!("joint_head 다운로드에 실패했습니다: {err}"))?;
    let head_dst = dir.join("joint_head.safetensors");
    std::fs::copy(&head_src, &head_dst).map_err(|err| format!("joint_head 파일 복사에 실패했습니다: {err}"))?;

    Ok((backbone_dst, head_dst))
}
```

- [ ] **Step 4: `Tokenizer` 래퍼를 작성한다**

```rust
pub struct LocalTokenizer {
    inner: tokenizers::Tokenizer,
}

impl LocalTokenizer {
    pub fn load() -> Result<Self, String> {
        let api = hf_hub::api::sync::Api::new()
            .map_err(|err| format!("HuggingFace API 초기화에 실패했습니다: {err}"))?;
        let repo = api.model("Cloudflare/clef-flash".to_string());
        let tokenizer_path = repo
            .get("tokenizer.json")
            .map_err(|err| format!("tokenizer.json 다운로드에 실패했습니다: {err}"))?;
        let inner = tokenizers::Tokenizer::from_file(&tokenizer_path)
            .map_err(|err| format!("토크나이저 로드에 실패했습니다: {err}"))?;
        Ok(Self { inner })
    }

    pub fn encode(&self, text: &str) -> Result<(Vec<u32>, Vec<(usize, usize)>), String> {
        let encoding = self
            .inner
            .encode(text, true)
            .map_err(|err| format!("토큰화에 실패했습니다: {err}"))?;
        Ok((encoding.get_ids().to_vec(), encoding.get_offsets().to_vec()))
    }
}
```

- [ ] **Step 5: 테스트를 다시 돌려 통과를 확인한다 (네트워크 제외)**

Run: `cargo test --manifest-path crates/decide/Cargo.toml local:: -- --skip downloads_when_cache_empty`
Expected: `ensure_weights_finds_files_in_clef_weights_dir`, `ensure_weights_errors_when_files_missing` PASS. 다른 모든 기존 로컬 테스트도 그대로 PASS.

- [ ] **Step 6: 커밋**

```bash
git add crates/decide/src/local/mod.rs
git commit -m "feat(decide): Clef-flash 가중치·토크나이저를 HuggingFace에서 1회 받아오는 경로를 추가한다"
```

---

### Task 7: 추론 파이프라인 연결과 parity 게이트

**Files:**
- Modify: `crates/decide/src/local/mod.rs` (`infer` 실제 구현으로 교체)
- Create: `scripts/clef_flash_oracle.py` (Python 오라클, 저장소에 커밋하되 실행은 `--features parity`에서만)
- Create: `crates/decide/tests/parity_fixtures/*.json` (오라클이 생성한 골든 입력·기대값)
- Test: `crates/decide/tests/parity.rs` (`#[cfg(feature = "parity")]`)

**Interfaces:**
- Consumes: 지금까지 만든 `backbone::Backbone`, `joint_head::JointHead`, `tokenizer::{schema_text, spans}`, `postprocess::to_answer`, `ensure_weights`, `LocalTokenizer`
- Produces: `local::infer`가 더 이상 스텁이 아니라 실제 추론을 수행하는 전체 경로. `backend.rs`의 `decide::<T>` 함수가 `Backend::Local`일 때 `local::infer`를 호출하도록 바뀐다(이 교체는 Task 8에서 진행 — 이 Task는 `infer` 자체만 완성한다).

- [ ] **Step 1: `infer`를 실제 추론으로 교체한다 (가중치 로드는 호출마다 하지 않고 `OnceLock`으로 캐시)**

```rust
// crates/decide/src/local/mod.rs
use std::sync::OnceLock;

struct Runtime {
    backbone: std::sync::Mutex<backbone::Backbone>,
    joint_head: joint_head::JointHead,
    tokenizer: LocalTokenizer,
}

static RUNTIME: OnceLock<Result<Runtime, String>> = OnceLock::new();

fn runtime() -> &'static Result<Runtime, String> {
    RUNTIME.get_or_init(|| {
        let (backbone_path, head_path) = ensure_weights()?;
        let backbone = backbone::Backbone::from_gguf_path(&backbone_path)?;
        let hidden_size = backbone.hidden_size();
        let joint_head = joint_head::JointHead::load(&head_path, hidden_size)?;
        let tokenizer = LocalTokenizer::load()?;
        Ok(Runtime {
            backbone: std::sync::Mutex::new(backbone),
            joint_head,
            tokenizer,
        })
    })
}

pub fn infer(state: &str, question: &Question) -> Result<Value, String> {
    let runtime = match runtime() {
        Ok(runtime) => runtime,
        Err(err) => return Err(err.clone()),
    };
    let text = tokenizer::schema_text(state, question);
    let (question_span, option_spans) = tokenizer::spans(state, question);
    let (token_ids, offsets) = runtime.tokenizer.encode(&text)?;
    let device = candle_core::Device::Cpu;
    let input_ids = candle_core::Tensor::new(token_ids.as_slice(), &device)
        .map_err(|err| err.to_string())?
        .unsqueeze(0)
        .map_err(|err| err.to_string())?;
    let hidden_states = {
        let mut backbone = runtime.backbone.lock().map_err(|_| "백본 락 획득에 실패했습니다".to_string())?;
        backbone.hidden_states(&input_ids)?
    };
    let logits = runtime
        .joint_head
        .score(&hidden_states, &input_ids, &question_span, &option_spans, &offsets)?;
    Ok(postprocess::to_answer(question, &logits))
}
```

- [ ] **Step 2: 테스트를 돌려 기존 스텁 테스트가 깨지지 않는지 확인한다**

`infer_is_not_ready_before_the_gate` 테스트(Task 1)는 이제 실제 가중치를 받으려 시도해 다른 에러로 바뀌므로, 이 테스트를 지우고 대신 `#[ignore]` 네트워크 테스트로 옮긴다.

Run: `cargo test --manifest-path crates/decide/Cargo.toml local::`
Expected: 가중치 없이 도는 모든 테스트 PASS. `infer_is_not_ready_before_the_gate`는 삭제됐으므로 더 이상 나타나지 않음.

- [ ] **Step 3: Python 오라클 스크립트를 작성한다**

```python
# scripts/clef_flash_oracle.py
# 비교 전용 — decide 런타임의 일부가 아니다. uv/pip로 transformers, torch를 받아 돌린다.
import json
import sys
from pathlib import Path

from transformers import AutoProcessor
from joint_schema_model import load_release_model, encode_record, collate_records

GOLDEN_INPUTS = [
    {"state": "서버가 다운됐습니다", "questions": {"q": {"type": "noul", "instructions": "긴급한가?"}}},
    {
        "state": "중복 결제",
        "questions": {
            "q": {
                "type": "choice",
                "instructions": "어느 팀?",
                "criteria": {"billing": "billing", "technical": "technical"},
            }
        },
    },
    {
        "state": "결제가 11번 실패했습니다",
        "questions": {
            "q": {
                "type": "choice",
                "instructions": "어느 팀?",
                "criteria": {str(i): str(i) for i in range(11)},
            }
        },
    },
    {
        "state": "응답이 평소보다 느립니다",
        "questions": {
            "q": {
                "type": "score",
                "instructions": "심각도?",
                "criteria": ["낮음", "중간", "높음"],
            }
        },
    },
    {"state": "결제 시스템이 전부 마비됐습니다", "questions": {"q": {"type": "noul", "instructions": "긴급한가?"}}},
]


def main() -> None:
    model, processor = load_release_model("Cloudflare/clef-flash")
    fixtures = []
    for case in GOLDEN_INPUTS:
        encoded = encode_record(processor.tokenizer, case, processor=processor)
        batch = collate_records([encoded], processor.tokenizer.pad_token_id, model.device)
        logits = model(batch)[0]
        q_logits = logits[0][0].detach().cpu().tolist()
        fixtures.append({"input": case, "logits": [round(x, 4) for x in q_logits]})
    Path("crates/decide/tests/parity_fixtures").mkdir(parents=True, exist_ok=True)
    with open("crates/decide/tests/parity_fixtures/golden.json", "w", encoding="utf-8") as f:
        json.dump(fixtures, f, ensure_ascii=False, indent=2)


if __name__ == "__main__":
    main()
```

- [ ] **Step 4: 오라클을 실제로 돌려 골든 fixture를 생성한다 (수동 실행, CI에 넣지 않음)**

Run: `python scripts/clef_flash_oracle.py`
Expected: `crates/decide/tests/parity_fixtures/golden.json`이 5개 케이스의 `input`/`logits`로 채워진다. (`joint_schema_model.py`는 `Cloudflare/clef-flash` 레포에서 받아 로컬 `PYTHONPATH`에 둔다 — 이 저장소에 커밋하지 않는다, HuggingFace 레포 자체 코드이기 때문.)

- [ ] **Step 5: Rust parity 테스트를 작성한다**

```rust
// crates/decide/tests/parity.rs
#![cfg(feature = "parity")]

use decide::local;
use decide::protocol::{validate, Incoming, Kind, Question};
use serde::Deserialize;
use serde_json::Value;

#[derive(Deserialize)]
struct Fixture {
    input: Value,
    logits: Vec<f64>,
}

const TOLERANCE: f64 = 0.02;

#[test]
fn rust_backbone_and_head_match_python_oracle_within_tolerance() {
    let raw = std::fs::read_to_string("tests/parity_fixtures/golden.json").expect("골든 fixture가 있어야 한다");
    let fixtures: Vec<Fixture> = serde_json::from_str(&raw).unwrap();

    for fixture in fixtures {
        let state = fixture.input["state"].as_str().unwrap().to_string();
        let q = &fixture.input["questions"]["q"];
        let kind = match q["type"].as_str().unwrap() {
            "noul" => Kind::Noul,
            "choice" => Kind::Choice,
            "score" => Kind::Score,
            other => panic!("알 수 없는 타입: {other}"),
        };
        let instructions = q["instructions"].as_str().unwrap().to_string();
        let options = q["criteria"]
            .as_object()
            .map(|m| m.keys().cloned().collect())
            .unwrap_or_default();
        let criteria = q["criteria"]
            .as_array()
            .map(|a| a.iter().map(|v| v.as_str().unwrap().to_string()).collect())
            .unwrap_or_else(|| if kind == Kind::Choice { Vec::new() } else { options.clone() });

        let incoming = Incoming {
            state: state.clone(),
            kind,
            instructions,
            options: if kind == Kind::Choice { options } else { Vec::new() },
            criteria: if kind == Kind::Score { criteria } else { Vec::new() },
        };
        let question = validate(&incoming).unwrap();

        let (_question_span, option_spans) = decide::local::tokenizer::spans(&state, &question);
        let rust_logits = rust_raw_logits(&state, &question); // 아래 helper

        assert_eq!(rust_logits.len(), fixture.logits.len());
        for (rust_val, python_val) in rust_logits.iter().zip(fixture.logits.iter()) {
            assert!(
                (rust_val - python_val).abs() < TOLERANCE,
                "로짓 차이가 허용치를 넘었다: rust={rust_val}, python={python_val}"
            );
        }
        let _ = option_spans;
    }
}

fn rust_raw_logits(_state: &str, _question: &Question) -> Vec<f64> {
    // local::infer는 softmax 이후 응답을 반환하므로, 이 테스트 전용으로
    // local 모듈에 #[cfg(feature = "parity")] pub fn raw_logits(...)를
    // 추가해 postprocess 이전 로짓을 그대로 받는다.
    unimplemented!("local::raw_logits 연결 — Step 6에서 추가")
}
```

- [ ] **Step 6: `local::raw_logits`를 parity 전용으로 노출한다**

```rust
// crates/decide/src/local/mod.rs 에 추가
#[cfg(feature = "parity")]
pub fn raw_logits(state: &str, question: &Question) -> Result<Vec<f32>, String> {
    let runtime = match runtime() {
        Ok(runtime) => runtime,
        Err(err) => return Err(err.clone()),
    };
    let text = tokenizer::schema_text(state, question);
    let (question_span, option_spans) = tokenizer::spans(state, question);
    let (token_ids, offsets) = runtime.tokenizer.encode(&text)?;
    let device = candle_core::Device::Cpu;
    let input_ids = candle_core::Tensor::new(token_ids.as_slice(), &device)
        .map_err(|err| err.to_string())?
        .unsqueeze(0)
        .map_err(|err| err.to_string())?;
    let hidden_states = {
        let mut backbone = runtime.backbone.lock().map_err(|_| "백본 락 획득에 실패했습니다".to_string())?;
        backbone.hidden_states(&input_ids)?
    };
    runtime
        .joint_head
        .score(&hidden_states, &input_ids, &question_span, &option_spans, &offsets)
}
```

`parity.rs`의 `rust_raw_logits`를 `decide::local::raw_logits(state, question).unwrap().into_iter().map(f64::from).collect()`로 교체한다.

- [ ] **Step 7: parity 테스트를 돌린다**

Run: `cargo test --manifest-path crates/decide/Cargo.toml --features parity --test parity -- --nocapture`
Expected: 5개 골든 입력 모두 PASS. 실패하면 `TOLERANCE`를 조정하거나(양자화 손실이 생각보다 크면), `backbone.rs`/`joint_head.rs`의 레이어 순서·치수 오류를 먼저 의심하고 고친다 — 이 단계는 수렴할 때까지 반복이 필요한 유일한 단계다.

- [ ] **Step 8: 커밋**

```bash
git add crates/decide/src/local/mod.rs crates/decide/tests/parity.rs crates/decide/tests/parity_fixtures/golden.json scripts/clef_flash_oracle.py
git commit -m "feat(decide): 로컬 추론 파이프라인을 연결하고 Python 오라클 대조 parity 게이트를 추가한다"
```

---

### Task 8: backend.rs 연결과 Laya 흔적 제거

**Files:**
- Modify: `crates/decide/src/backend.rs`
- Modify: `crates/decide/src/local/mod.rs` (parity 통과 후 `NOT_READY` 분기 제거)
- Test: `crates/decide/src/backend.rs` (기존 테스트 수정)

**Interfaces:**
- Consumes: `local::infer(state: &str, question: &Question) -> Result<Value, String>` (Task 7에서 완성)
- Produces: `backend::decide`가 `Backend::Local`일 때 `local::infer`를 호출하고 `routing: {"backend": "local", "model": "clef-flash"}`를 싣는다.

- [ ] **Step 1: `backend.rs`의 `decide` 함수에서 로컬 분기를 바꾼다**

`crates/decide/src/backend.rs`의 현재:

```rust
if backend == Backend::Local {
    return Err(local::NOT_READY.to_string());
}
```

를 다음으로 바꾼다:

```rust
if backend == Backend::Local {
    let start = millis();
    let answer = local::infer(&incoming.state, &question)?;
    let latency_ms = millis() - start;
    return Ok(DecideResult {
        answer,
        routing: json!({
            "backend": "local",
            "model": "clef-flash",
        }),
        latency_ms,
    });
}
```

- [ ] **Step 2: 기존 `local_and_over_limit_do_not_call_typesafe` 테스트를 고친다**

이 테스트는 지금 `DECIDE_BACKEND=local`이 `local::NOT_READY`를 반환한다고 가정한다 — parity 게이트를 통과한 뒤에는 실제 추론을 시도하므로, 가중치가 없는 CI 환경에서는 다른 에러(`ensure_weights` 실패)가 난다. 이 테스트의 목적("local을 고르면 TypeSafe를 호출하지 않는다")은 유지하되, 에러 메시지 비교 대신 "TypeSafe transport가 호출되지 않았다"만 검사하도록 좁힌다:

```rust
#[test]
fn local_backend_does_not_call_typesafe() {
    let mut script = Script {
        responses: vec![],
        calls: Cell::new(0),
    };
    let _ = decide(
        &noul(),
        &env(Some("local"), Some("k")),
        &mut script,
        || 0.0,
        || {
            panic!("로컬은 호출하지 않는다");
        },
    ); // 로컬 추론 자체의 성공/실패는 가중치 유무에 따라 환경마다 다르므로 결과를 단정하지 않는다
    assert_eq!(script.calls.get(), 0);
}
```

choice 256개 초과 한도 테스트는 TypeSafe 분기 전용이므로 그대로 둔다.

- [ ] **Step 3: `local::mod.rs`의 `infer` 안에 있던 임시 스텁 경로가 완전히 사라졌는지 확인한다**

Task 7에서 이미 `infer`를 실제 구현으로 바꿨으므로, 이 Task에서는 `NOT_READY` 상수가 더 이상 코드 경로에서 반환되지 않는 것만 확인한다 — 상수 자체는 `ensure_weights`나 `runtime()` 초기화 실패 메시지로는 쓰이지 않으므로 삭제한다.

```bash
grep -n "NOT_READY" crates/decide/src/local/mod.rs crates/decide/src/backend.rs
```

Expected: 매칭 없음(또는 테스트 코드에만 남아있다면 그 테스트도 함께 정리). 매칭이 있으면 해당 참조를 지운다.

- [ ] **Step 4: 전체 테스트를 돌린다**

Run: `cargo test --manifest-path crates/decide/Cargo.toml`
Expected: 가중치 없이 도는 전체 스위트 PASS.

- [ ] **Step 5: 커밋**

```bash
git add crates/decide/src/backend.rs crates/decide/src/local/mod.rs
git commit -m "feat(decide): DECIDE_BACKEND=local이 Clef-flash로 실제 추론하도록 연결한다"
```

---

### Task 9: 설치·설정 문서 갱신 (Laya → Clef-flash)

**Files:**
- Modify: `CLAUDE.md`
- Modify: `packaging/homebrew/decide.rb` (환경변수 안내 주석만 — formula 자체의 설치 스텝은 안 바뀜)
- Modify: `.claude/skills/decide/SKILL.md` (존재하면)

**Interfaces:** 없음 — 문서 전용 Task.

- [ ] **Step 1: `CLAUDE.md`의 로컬 백엔드 관련 서술을 갱신한다**

`CLAUDE.md`의 "Local answers keep Laya's `system_one` keys, including `action`, once local inference is connected." 문장과 "Until the local published-field check passes..." 단락을 다음으로 바꾼다(기존 영문 스타일 유지):

```markdown
Local answers follow the same field shape as TypeSafe (`choice`/`score`/`noul`
plus `confidence` and `probabilities`), from Cloudflare's Clef-flash
(Qwen3.5-9B hybrid backbone, GGUF Q4_K_M, vendored from candle PR #3396) plus
a from-scratch joint schema head. `routing.model` is `"clef-flash"`.

Until `cargo test --features parity` passes against the Python `transformers`
oracle, `DECIDE_BACKEND=local` returns `로컬 백엔드가 아직 준비되지
않았습니다` and does not infer.
```

- [ ] **Step 2: `src/decide/`(Python 패키지) 관련 서술이 이 변경과 충돌하지 않는지 확인한다**

CLAUDE.md의 "The Python package in `src/decide/` stays for that comparison and for `pytest`." 문장은 Laya 비교용이었다 — Clef-flash는 비교 오라클이 Python `transformers`(Task 7의 `scripts/clef_flash_oracle.py`)로 바뀌었으므로, 이 문장을 다음으로 바꾼다:

```markdown
The Python package in `src/decide/` was Laya's comparison oracle and is no
longer needed for Clef-flash — see `scripts/clef_flash_oracle.py` instead.
Delete `src/decide/` once the parity gate in Task 7/8 above has passed on a
real machine.
```

- [ ] **Step 3: `packaging/homebrew/decide.rb`를 확인하고 환경변수 이름이 코드 밖(formula 설명문 등)에 노출돼 있으면 갱신한다**

```bash
grep -n "LAYA" packaging/homebrew/decide.rb
```

매칭이 있으면 `CLEF_WEIGHTS`로 바꾼다. 없으면 이 Step은 스킵.

- [ ] **Step 4: `.claude/skills/decide/SKILL.md`가 있으면 두 백엔드 설명을 갱신한다**

```bash
test -f .claude/skills/decide/SKILL.md && grep -n -i "laya" .claude/skills/decide/SKILL.md
```

매칭이 있으면 "로컬 백엔드는 Laya"라는 서술을 "로컬 백엔드는 Cloudflare Clef-flash"로 바꾼다.

- [ ] **Step 5: 커밋**

```bash
git add CLAUDE.md packaging/homebrew/decide.rb
git commit -m "docs(decide): CLAUDE.md와 설치 안내를 Laya에서 Clef-flash 기준으로 갱신한다"
```

---

### Task 10: 실행 검증과 Laya 유산 정리

**Files:**
- Remove: `src/decide/` (parity 통과 확인 후)
- Remove: `tests/`의 Python 테스트, `test_smoke.py`, `pyproject.toml`, `.python-version` (parity 통과 확인 후)
- Modify: `.gitignore` (Laya 가중치 캐시 관련 항목이 있다면 Clef 쪽으로 교체)

**Interfaces:** 없음 — 정리와 최종 검증 Task.

- [ ] **Step 1: 실행 검증 네 가지를 Homebrew로 설치된 바이너리에 대해 직접 확인한다**

```bash
cargo build --manifest-path crates/decide/Cargo.toml --release
cargo test --manifest-path crates/decide/Cargo.toml
cargo test --manifest-path crates/decide/Cargo.toml --features parity
```

Expected: 전부 PASS.

```bash
echo '{"state":"서버 다운","type":"noul","instructions":"긴급한가?"}' | DECIDE_BACKEND=local TYPESAFE_API_KEY= cargo run --manifest-path crates/decide/Cargo.toml -- mcp
```

(MCP stdio 프로토콜이라 수동 호출은 JSON-RPC 메시지로 감싸야 한다 — `mcp.rs`의 기존 핸들링 방식을 참고해 `{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"decide","arguments":{...}}}` 형태로 보낸다.)

Expected: `routing.backend`가 `"local"`, `routing.model`이 `"clef-flash"`인 응답.

- [ ] **Step 2: 데몬 소켓 경로도 확인한다**

```bash
cargo run --manifest-path crates/decide/Cargo.toml -- daemon &
sleep 1
echo '{"state":"서버 다운","type":"noul","instructions":"긴급한가?","backend":"local"}' | nc -U ~/.cache/decide/decide.sock
kill %1
```

Expected: 같은 틀의 JSON 응답 한 줄.

- [ ] **Step 3: 네 가지 실행 검증이 모두 끝났으면 Laya 유산을 지운다**

```bash
git rm -r src/decide/
git rm test_smoke.py pyproject.toml .python-version
git rm -r tests/  # Python pytest 전용 디렉터리였는지 확인 후 — Rust tests/parity.rs와 겹치지 않는지 먼저 확인
```

(`tests/`가 Rust `tests/parity.rs`와 같은 디렉터리라면 Python 테스트 파일만 선별해 지운다 — 전체 삭제 전에 `ls tests/`로 내용을 확인한다.)

- [ ] **Step 4: README/CLAUDE.md에서 `pip`/`.venv`/`pytest` 설치·실행 절차를 지운다**

```bash
grep -n "pip\|\.venv\|pytest" README.md CLAUDE.md
```

매칭된 줄 중 Laya/Python 비교 오라클 관련 설명을 지운다. `cargo test`만 남긴다.

- [ ] **Step 5: 최종 커밋**

```bash
git add -A
git commit -m "chore(decide): parity 검증을 마친 뒤 Laya 비교용 Python 구현을 지운다"
```

---

## Self-Review 메모 (계획 작성자용, 실행자는 참고만)

- **스펙 커버리지**: 배경(Task 3), 목표/성공기준(Task 7·8), 범위 밖(모든 Task가 비전·GPU·Clef-27B·자동 전환을 건드리지 않음), 가중치/정밀도(Task 1·6), 구성(Task 1~5 파일 배치), 백본/헤드 경계(Task 3 Step 3), 프로토콜(Task 2·5), 테스트(Task 7), 구현 순서(Task 1~8이 스펙의 1~6단계와 대응), 설치(Task 6·9) — 전부 대응하는 Task가 있다.
- **타입 일관성**: `Question`(Task 1부터 끝까지 동일 타입 재사용), `QuestionSpan`/`OptionSpan`(Task 2 정의 → Task 4·7에서 그대로 소비), `JointHead::score` 반환값 `Vec<f32>`(Task 4 정의 → Task 5·7에서 그대로 소비), `local::infer`/`local::raw_logits` 시그니처(Task 7에서 정의 → Task 8에서 그대로 호출) — 이름과 타입이 Task 경계를 넘어도 바뀌지 않는다.
- **알려진 리스크**: Task 3(candle-nn의 `MultiheadAttention`/`TransformerDecoderLayer` 정확한 API 존재 여부)과 Task 7(parity 허용 오차가 실제로 수렴하는지)은 실행 중 실제 컴파일러·가중치 피드백이 필요한 유일한 두 지점이다 — 계획의 다른 모든 Task는 가중치 없이 완전히 결정론적으로 검증된다.
