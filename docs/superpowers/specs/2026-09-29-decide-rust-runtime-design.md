# decide Rust 런타임

## 배경

`decide`는 로컬 Laya 판단 모델을 MCP 도구 `decide`로 연다. 판단 코드는
`src/decide/` 세 파일이고, 프로세스 크기를 만드는 것은 CPython, torch,
transformers다. 이 체크아웃의 `.venv`는 941MB이고 torch가 582MB다.

가중치는 패키지 밖에 있다. `Router(preload=True)`가
`convaiinnovations/laya`에서 체크포인트를 받는다. 허브 파일 크기는 영어
`model.safetensors` 804MB, `typed-decisions/model.safetensors` 804MB,
`multilingual/model.safetensors` 614MB, 토크나이저 약 41MB다. 세 파일 모두
`model_type: modernbert`다. 영어와 typed-decisions는 hidden 1024, 28층이고
다국어는 hidden 768이다.

순전파는 재고 ModernBERT 분류기가 아니다. `DecisionModel.forward`는 인코더
숨은 상태에 질문 종류 임베딩을 더하고, 사전 정규화 트랜스포머 2층을 올린 뒤,
옵션마다 심은 마스크 위치만 모아 로짓을 만든다. 엔트로피와 상위 2개 확률로
`act_head`를 따로 계산한다. 온도 보정과 choice, score, noul 조립은 순전파
밖이다. 입력 형식은 `[CLS] <type> question: … [SEP] [MASK] opt … [SEP] state
[SEP]`이다.

2026-09-29 검토에서 배포 형태는 두 가지였다. Python을 onedir로 묶으면 확률을
다시 재지 않아도 되고, 산출물은 약 1GB 런타임에 가중치 2.3GB가 더해진다.
Rust로 옮기면 libtorch 없이 실행 파일 하나가 되고, 가중치는 파일 밖에 둔다.
같은 날 언어는 Rust로 정했다.

현재 MCP와 데몬은 `Router(preload=True).predict(state, {"q": question})`만
부른다. `model`, `task`, `lang`을 넘기지 않고 `auto_task_detection`은 기본값
false다. 질문 id는 항상 `"q"`라 typed-decisions 워크플로 일치로 그 체크포인트가
선택되지 않는다. Python이 세 모델을 미리 올리는 것은 `preload=True`의 기본
동작이지, 이 서버의 호출 경로는 아니다.

## 목표

macOS arm64에서 Python과 libtorch 없이 동작하는 `decide` 실행 파일 하나를
만든다. 도구 인자, 검증 문장, 답 필드, 데몬 소켓 경로는 지금 호출자가 보는
것과 같다.

성공 기준은 바이너리가 뜨는 것이 아니다. 고정된 입력 묶음에서 Python Laya
0.3.6이 내는 게시 필드와 Rust가 내는 게시 필드가 같다. 게시 필드는 Laya가
Python 3 `round(x, 4)`로 만든 값이다. 짝수 쪽으로 반올림하는 그 규칙을
따른다. choice의 `choice`와 `probabilities`, score의 `score`와
`probabilities`, noul의 `noul`, 그리고 각 답의 `confidence`와
`action.act_probability`가 여기 들어간다.

## 범위 밖

- 가중치를 실행 파일 안에 넣기. 코드 바이너리는 작고, 가중치는 옆에 둔다.
- GPU, 양자화, fp16 추론. 기준 구현은 CPU fp32다. 가중치 파일은 fp16 크기에
  가깝지만 `Agent`는 CPU에서 fp32로 올린다.
- typed-decisions 체크포인트, `auto_task_detection`, `model` / `task` /
  `lang` 인자. 지금 도구는 이것을 노출하지 않는다.
- Go, TypeScript, Python 프리즈. Python 패키지는 기준 구현으로 남는다.
- Intel Mac과 Linux 빌드. 첫 바이너리는 이 개발 머신과 같은 OS와
  아키텍처다.
- `.mcp.json`을 개발용 `.venv` 밖으로 바꾸는 것. 그 경로는
  `tests/test_project_config.py`가 고정한다. 배포 바이너리는 그 검사를
  바꾸지 않는 별도 산출물이다.

## 런타임 선택

인코더와 헤드를 한 그래프로 고정할 수 있으면 헤드를 Rust로 다시 쓰지 않는다.
첫 후보는 ONNX다.

1. Python 스크립트가 영어 체크포인트의 `DecisionModel` 전체를 ONNX로 보낸다.
   동적 축은 배치와 시퀀스 길이다. 슬라이딩 주의 마스크가 그래프에 남아
   있어야 한다.
2. Rust는 그 그래프를 실행하고, 같은 토큰 열에 대해 Python 로짓과 비교한다.
3. 게시 필드가 하나라도 다르면 ONNX를 버리고 Candle로 바꾼다. Candle은
   ModernBERT 백본을 쓰고, Laya 헤드(`type_emb`, 트랜스포머 2층, `scorer`,
   `act_head`)와 `layer_types`, 중첩 `rope_parameters`를 직접 읽는다.
4. 한 런타임이 기준을 통과하면 크레이트에는 그 런타임만 남긴다. 두 백엔드를
   기능 플래그로 같이 싣지 않는다.

ONNX를 쓰는 동안 의존성은 ONNX 그래프를 CPU에서 실행할 수 있는 것이다.
그래프가 통과하면 순수 Rust 런타임으로 링크를 줄일 수 있는지 한 번 더 본다.
그 대체가 같은 게시 필드를 내지 못하면 ONNX Runtime 링크를 유지한다.
Candle로 넘어가면 libtorch와 ONNX Runtime은 없다.

이 게이트를 통과하기 전에는 MCP 서버와 데몬을 붙이지 않는다.

## 구성

크레이트는 `crates/decide` 하나다. 바이너리 이름도 `decide`다.

```
crates/decide/
  src/
    main.rs           mcp | daemon 분기. 인자 없으면 mcp
    protocol.rs       검증, 질문 dict, DecideResult
    sequence.rs       render_options, build_sequence
    postprocess.rs    온도, 확률, 게시 필드
    route.rs          analyse + 기본 route
    runtime.rs        통과한 런타임 하나의 로드와 순전파
```

가중치 디렉터리는 환경 변수 `LAYA_WEIGHTS`가 가리키는 경로다. 없으면
`~/.cache/huggingface/hub`의 `convaiinnovations/laya` 스냅샷을 쓴다. 둘 다
없으면 허브에서 영어와 다국어 서브폴더만 받는다. typed-decisions는 받지
않는다. ONNX 파일과 Candle이 읽는 safetensors는 저장소에 커밋하지 않는다.
`target/`과 내보낸 `*.onnx`는 gitignore에 넣는다.

프로세스 모델은 지금과 같다.

- `decide`와 `decide mcp`는 stdio MCP 서버다. 세션당 한 프로세스이고, 영어와
  다국어 체크포인트를 기동 시 한 번 올린다.
- `decide daemon`은 `~/.cache/decide/decide.sock`에서 한 줄 JSON을 받는다.
  소켓이 이미 살아 있으면 기동을 거절한다. 죽은 소켓 파일은 지우고 다시
  묶는다. 유휴 30분이면 종료한다.
- `.claude/hooks/stop_verify.py`는 표준 라이브러리 그대로 둔다. 데몬을 띄우는
  명령만 `decide daemon`으로 바꾼다. 소켓이 없으면 이번 호출은 통과시키는
  fail-open도 유지한다. 이 변경은 런타임 게이트 다음이다.

## 프로토콜

도구 이름은 `decide`다. 인자는 `state`, `type`, `instructions`, `options`,
`criteria`다. `type`은 `choice`, `score`, `noul`이다.

질문 dict는 `src/decide/server.py`의 `_decide_impl`과 같다.

- choice는 `criteria`가 `{옵션: 옵션}`인 맵이다. 서로 다른 옵션이 2개
  미만이면 `choice 타입은 서로 다른 옵션이 최소 2개 필요합니다`.
- score는 `criteria`가 등급 문자열의 리스트다. 2개 미만이면
  `score 타입은 등급이 최소 2개 필요합니다`.
- noul은 `options`와 `criteria`가 둘 다 비어 있어야 한다. 아니면
  `noul 타입은 options/criteria를 받지 않습니다`.

모델에는 `{"q": question}` 하나만 넘긴다. 반환은 지금 `DecideResult`와 같다.

```json
{
  "answer": { "type": "noul", "noul": 0.62, "confidence": 0.62, "action": { "act_probability": 0.41 } },
  "routing": { "model": "multilingual", "reason": "...", "detection": {}, "workflow": null },
  "latency_ms": 180.0
}
```

`answer` 안의 키는 Laya `system_one`이 `"q"`에 넣는 키 전부다. choice는
`choice`, `probabilities`, `confidence`, `action`이다. score는 `score`,
`legend`, `probabilities`, `confidence`, `action`이다. noul의 `noul`은
온도 보정 뒤 확률의 index 1이고, `confidence`는 `max(p[1], 1-p[1])`를
넷째 자리에서 반올림한 값이다. choice와 score의 `confidence`는
`confidence_from_probs`다.

데몬은 연결당 JSON 한 줄을 읽고 JSON 한 줄을 쓴다. 요청 필드는 `state`,
`type`, `instructions`, `options`, `criteria`다. 실패하면 `{"error": "..."}`만
돌려주고 프로세스는 유지한다.

라우팅은 `laya/lang.py`의 `analyse`와 `Router.route`의 검출 분기다.
`analyse`는 유니코드 스크립트 범위와 라틴 불용어, 발음 구별 기호 비율을
함께 본다. 글자가 없으면 영어, 비라틴 스크립트면 다국어, 라틴이라도 영어가
아니면 다국어, 영어 라틴이면 영어다. 한글은 비라틴이므로 다국어
체크포인트다. `detection`은 `script`, `script_profile`, `language`,
`is_english`, `language_undecided`, `diacritic_rate`, `non_latin_fraction`
키를 Python과 같이 낸다. `reason` 문장도 같은 형식이다.

시퀀스와 후처리는 `laya/common.py`와 `laya/agent.py`의 `build_sequence`,
`render_options`, `clamp_temperature`, `temp_bucket`, `system_one`의 확률
조립을 따른다. 온도는 `[0.5, 5.0]` 밖이거나 숫자가 아니면 1.0이다. 기본
`max_len`은 512, `head_max_len`은 192다. 토크나이저는 각 체크포인트의
`tokenizer.json`이다.

## 테스트

기본 `cargo test`는 가중치와 네트워크 없이 돈다.

- 시퀀스와 후처리, 검증 문장, 스크립트 라우팅은 Python이 만든 JSON 픽스처와
  비교한다. 픽스처 생성은 설치된 `laya` 0.3.6과 로컬 토크나이저만 쓰고,
  모델 가중치는 쓰지 않는다.
- 게시 필드 비교는 `cargo test --features parity`다. 가중치가 있을 때만
  돌리고, 기본 테스트와 `pytest`에는 넣지 않는다. 수동 스모크
  `test_smoke.py`와 같은 층이다.
- 골든 입력은 최소 다섯 개다. 영어 noul, 옵션 2개인 choice, 옵션 11개 이상인
  choice, 등급 3개인 score, 한글이 포함된 state다. 한글 입력은 다국어
  체크포인트로 가야 한다.
- 기존 `pytest`는 그대로 통과해야 한다. Python 서버 동작은 이 스펙에서
  바꾸지 않는다. `stop_verify.py`의 기동 명령 변경은 예외이며, 그 테스트가
  기대하는 명령 문자열을 같이 고친다.

## 구현 순서

1. 픽스처와, 가중치 없이 검증할 수 있는 시퀀스, 후처리, 검증, 라우팅.
2. ONNX보내기와 영어 체크포인트 로짓 비교. 실패하면 Candle로 같은 비교.
3. 통과한 런타임만 남겨 영어와 다국어를 기동 시 로드.
4. `decide mcp`와 `decide daemon`.
5. `stop_verify.py`가 `decide daemon`을 띄우게 변경.

1단계가 끝나기 전에는 2단계 코드를 크레이트에 남기지 않는다. 2단계가
게시 필드 기준을 통과하기 전에는 4단계를 시작하지 않는다.
