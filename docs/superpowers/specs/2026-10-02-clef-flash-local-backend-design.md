# Clef-flash 로컬 백엔드

## 배경

TypeSafe Jev는 API 전용이고 공개 가중치가 없어 로컬 실행이 불가능하다. 이전
설계(`2026-09-29-decide-rust-runtime-design.md`)는 이 자리를 Convai
Innovations의 Laya(ModernBERT 기반, 영어/다국어 두 체크포인트)로 채우기로
했으나, Laya를 Rust 런타임에 연결하는 5단계(ONNX 비교 → 실패 시 Candle)는
아직 통과하지 못했고 `local.rs`는 지금도 `로컬 백엔드가 아직 준비되지
않았습니다`를 반환한다.

2026-10-01 Cloudflare가 Jev와 같은 System One 계약(`state` + 타입이 있는
`questions` → `answers`)을 쓰는 오픈 가중치 모델 Clef(27B, Qwen3.8-27B
백본)와 Clef-flash(9B, Qwen3.5-9B 백본)를 Apache 2.0으로 공개했다. 이 설계는
Laya 대신 Clef-flash를 로컬 백엔드로 채택한다.

Clef-flash의 베이스는 Qwen3.5-9B다. `config.json` 기준 32개 레이어 중 28개가
`linear_attention`(Gated DeltaNet류), 4개가 `full_attention`인 하이브리드
구조고, `partial_rotary_factor: 0.25`에 `mrope_section: [11, 11, 10]`을 쓴다.
candle 공식 저장소는 이 구조를 아직 포함하지 않는다. PR #3461
(`quantized_qwen35.rs`)이 "Candle 최초로 작동하는 Gated DeltaNet + partial
RoPE 구현"이라고 자평하는 미병합 커뮤니티 구현이다. 이 설계는 그 PR을
벤더링해 그대로 쓰는 위험을 감수하기로 결정했다 — candle이 공식 지원할 때까지
기다리지 않는다.

백본 위에는 `JointSchemaHead`가 있다. 이름과 달리 작은 선형 레이어가 아니라
그 자체로 작은 트랜스포머다: `LayerNorm` + bias 없는 `Linear` 투영 6개,
`EvidenceRoutingLayer`(`MultiheadAttention` + GELU FFN) 여러 겹, 표준
`TransformerDecoderLayer` 여러 겹, 코사인 유사도 기반 "joint" 점수와 어휘
기반 "prior" 점수를 섞는 `residual_scorer`로 구성된다. 질문-옵션 스팬을
평균 풀링해 벡터를 만들고, 전체 시퀀스(`memory`)에 대해 cross-attention으로
증거를 모아, 질문마다 옵션 개수 크기의 로짓 벡터를 낸다.

Clef-flash는 멀티모달(이미지/비디오)이지만 `decide` 도구의 입력은 전부
문자열(`state`, `instructions`, `options`, `criteria`)이다. 이 설계는 비전
인코더를 포팅하지 않는다 — candle에 Qwen3-VL 비전 타워 공식 지원이 없고,
`decide`가 그 입력을 받지도 않는다.

## 목표

`DECIDE_BACKEND=local`을 고르면 TypeSafe와 같은 도구 계약으로 Clef-flash가
답한다. Laya 관련 설정·코드·용어는 전부 Clef-flash로 교체되고 남지 않는다.

성공 기준:

- 로컬 백엔드는 Python `transformers` + 공개 가중치로 돌린 `systemone()`
  오라클과, 정해진 허용 오차 안에서 같은 noul/choice/score 답을 낸다.
- `DECIDE_BACKEND=local`로 호출하면 `routing.backend`가 `"local"`,
  `routing.model`이 `"clef-flash"`인 답이 온다.
- 한글 state도 같은 체크포인트로 처리된다 (Laya의 영어/다국어 분기는 없다
  — Clef-flash는 원래 다국어다).

## 범위 밖

- 비전/비디오 입력, Qwen3-VL 비전 인코더 포팅.
- Clef(27B, Clef-flash의 상위 모델) — 이 설계는 Clef-flash(9B)만 다룬다.
- Candle PR #3461이 상류에서 바뀌거나 끊기는 것에 대한 장기 추적. 벤더링한
  코드는 이 저장소가 직접 소유하고 고친다.
- 로컬·TypeSafe 두 백엔드를 호출마다 자동으로 오가는 것. 지금처럼
  `DECIDE_BACKEND`가 한 번 고르면 바뀌지 않는다.
- 두 로컬 체크포인트(Laya와 Clef-flash) 동시 지원. Laya는 완전히 들어낸다.
- GPU/Metal 가속, FP8/BF16 로컬 추론. 1차 기준은 CPU + GGUF Q4_K_M이다.
- Cloudflare 쪽 모델 자체의 정확도·보정 개선. 공개된 가중치를 그대로 쓴다.
- TypeSafe를 Cloudflare Workers AI API 호출로 바꾸는 것(별도 주제 — 이
  설계는 로컬 실행만 다룬다).

## 가중치와 정밀도

9B 모델을 Mac CPU에서 돌리려면 양자화가 거의 필수다. 백본은 GGUF Q4_K_M
(커뮤니티 레포 `prithivMLmods/clef-flash-GGUF` 기준 수 GB대)을 쓴다.
`joint_head.safetensors`는 백본보다 훨씬 작아 양자화하지 않고 원본 그대로
읽는다. 두 파일 모두 `CLEF_WEIGHTS`가 가리키는 디렉터리에서 찾고, 없으면
`~/.cache/huggingface/hub`의 `Cloudflare/clef-flash`(헤드·토크나이저)와
GGUF 레포 스냅샷을 로컬 백엔드 첫 호출 시 1회 받는다. 기존 `LAYA_WEIGHTS`를
대체한다.

품질 보증은 BF16 Python 오라클과 4bit Rust 구현을 대조하는 parity 게이트로
한다 — 양자화 손실 때문에 완전 일치는 기대하지 않고, 실제로 돌려 본 뒤
허용 오차(확률 기준, 예를 들어 ±0.02 안쪽)를 정한다.

## 구성

```
crates/decide/
  src/
    local/
      mod.rs           local::infer(state, question) -> Value, 진입점
      backbone.rs       PR #3461 벤더링. 하이브리드 레이어 forward, hidden_states 반환
      joint_head.rs      신규 구현. EvidenceRoutingLayer, TransformerDecoderLayer, residual scorer
      tokenizer.rs       state/questions를 joint_schema_model.py와 같은 스키마 텍스트로 조립
      postprocess.rs     로짓 → softmax → noul/choice/score 응답
    backend.rs           기존 유지. local 분기만 local::infer 호출로 교체
packaging/homebrew/decide.rb   바이너리만 설치, 기존 방식 유지
```

기존 설계가 예정했던 `sequence.rs` / `postprocess.rs`(Laya 전제) /
`route.rs`(영어/다국어 라우팅)는 폐기한다. 이 네 파일로 대체한다.

## 백본과 헤드의 경계

`backbone.rs`는 최종 로짓이 아니라 **마지막 레이어의 hidden_states**를
노출해야 한다. PR #3461이 이 인터페이스를 그대로 주는지가 가장 큰 미지수다
— 주지 않으면 벤더링한 코드를 고쳐서 뽑아내야 한다. `joint_head.rs`는 이
hidden_states와 `input_ids`, `attention_mask`, 질문/옵션의 토큰 스팬
위치를 받아 질문마다 `(옵션 개수,)` 크기의 로짓 텐서를 낸다.

## 프로토콜

도구 인자는 바뀌지 않는다(`state`, `type`, `instructions`, `options`,
`criteria`). 로컬 백엔드 내부에서만 Clef 형식으로 오간다.

**요청 변환** (`tokenizer.rs`) — `joint_schema_model.py`의 스키마 텍스트
포맷을 그대로 재현한다.

```
FIELD q
ID:q
TYPE:{noul|choice|score}
INSTRUCTION:{instructions}
```

choice는 옵션을 `{option: option}` 딕셔너리로(지금 `typesafe.rs::request_body`
와 같은 변환 로직을 함수로 뽑아 공유), score는 등급 리스트 그대로, noul은
옵션 없이 조립한다. state는 UTF-8 그대로 JSON으로 직렬화해 뒤에 붙인다
(Python의 `ensure_ascii=False`와 동등).

**응답 변환** (`postprocess.rs`) — TypeSafe 응답과 같은 필드 틀로 통일한다.

```json
// noul
{"type": "noul", "noul": 0.0}

// choice
{"type": "choice", "choice": "billing", "confidence": 0.0, "probabilities": {}}

// score
{"type": "score", "score": 0.0, "confidence": 0.0, "legend": [], "probabilities": {}}
```

이전 설계는 로컬 답이 Laya `system_one`의 키를 유지하고 `action.act_probability`
를 포함한다고 적었다. Clef-flash 출력에는 그 필드가 없다 — 이 설계는 그
요구를 지운다. 로컬 답도 TypeSafe와 같은 필드 틀을 쓰고, `routing.backend`
만 `"local"`, `routing.model`은 `"clef-flash"`다.

## 테스트

기본 `cargo test`(가중치·네트워크 없이):

- `tokenizer.rs`의 스키마 텍스트 조립은 문자열 비교로 검사한다.
- `joint_head.rs`는 랜덤/더미 가중치로 forward가 깨지지 않는지, 텐서 shape이
  맞는지만 검사한다.

`cargo test --features parity`(가중치·네트워크 필요, 기본 스위트와 분리):

1. Python 스크립트가 `transformers` + HuggingFace `Cloudflare/clef-flash`
   (BF16 원본)로 골든 입력 각각에 대해 `joint_schema_model.py::systemone()`
   을 직접 호출해 로짓·확률을 기록한다 — 이것이 오라클이다.
2. Rust(GGUF Q4_K_M 백본 + 재구현한 joint_head)가 같은 입력에 같은 게시
   필드(`round(x, 4)`)를 내는지 비교한다. 4bit 양자화 손실을 감안한 허용
   오차를 둔다.
3. 골든 입력: 영문 noul, 옵션 2개 choice, 옵션 11개 이상 choice, 등급 3개
   score, 한글 state. 한글도 같은 체크포인트로 처리되는지만 확인한다 —
   별도 분기가 없으므로 라우팅 테스트는 없다.

로컬 게이트를 통과하기 전에는 `DECIDE_BACKEND=local`이 계속
`local::NOT_READY`를 반환한다. TypeSafe 백엔드와 MCP 진입점은 이 게이트를
기다리지 않는다.

## 구현 순서

1. `tokenizer.rs` — 스키마 텍스트 조립, 가중치 없는 단위 테스트.
2. PR #3461 벤더링 — `backbone.rs`에 하이브리드 레이어 forward, hidden_states
   추출이 되는지 더미 가중치로 shape만 확인.
3. `joint_head.rs` — EvidenceRoutingLayer, TransformerDecoderLayer, residual
   scorer 구현, shape 테스트.
4. `postprocess.rs` — 로짓 → 응답 매핑, TypeSafe 응답 포맷과 필드 비교
   테스트.
5. Python 오라클 스크립트로 골든 입력 로짓을 기록하고
   `cargo test --features parity`로 Rust 쪽과 비교, 허용 오차를 확정한다.
6. parity 통과 시 `local.rs`를 이 모듈로 교체하고 `backend.rs`의 "준비되지
   않았습니다" 분기를 제거한다.
7. Homebrew formula, `.mcp.json`, `CLAUDE.md`를 Laya 기준에서 Clef-flash
   기준으로 갱신한다. 이전 Laya 설계 문서(`2026-09-23`, `2026-09-29`)는
   이력으로 보존하고 지우지 않는다.

## 설치

`packaging/homebrew/decide.rb`는 바이너리만 설치하는 지금 방식을 유지한다
— 가중치는 formula에 넣지 않는다. `CLEF_WEIGHTS`가 있으면 그 디렉터리,
없으면 HuggingFace 캐시에서 로컬 백엔드 첫 호출 시 1회 받는다. 1차 범위는
CPU 추론만이다 — candle Metal 백엔드는 느리면 이후 별도로 검토한다.
