# decide Rust 런타임

> 2026-09-30: 로컬 런타임 절(ONNX/Candle, Laya 게시 필드 일치)은 `2026-09-30-decide-local-jev-style-design.md`가 대체한다. 이 문서의 나머지와 Python 삭제 절차는 그대로 적용됐다.

## 배경

`decide`는 판단 한 건을 MCP 도구 `decide`로 연다. 지금 구현은 Python 패키지
`decide`가 `laya.Router`를 프로세스 안에 둔다. 이 체크아웃의 `.venv`는
941MB이고 torch가 582MB다. 설치 절차는 가상환경과 `pip install -e`다.

가중치는 패키지 밖에 있다. 허브의 `convaiinnovations/laya`에서 영어
`model.safetensors`는 804MB, 다국어는 614MB, typed-decisions는 804MB다.
세 파일 모두 `model_type: modernbert`다. 이 서버의 호출은
`Router(preload=True).predict(state, {"q": question})`뿐이라 typed-decisions는
선택되지 않는다.

로컬 순전파는 재고 분류기가 아니다. `DecisionModel`은 ModernBERT 숨은 상태에
질문 종류 임베딩과 트랜스포머 2층, 마스크 위치 채점을 얹는다. 입력 형식은
`[CLS] <type> question: … [SEP] [MASK] opt … [SEP] state [SEP]`이다.

2026-09-29에 언어는 Rust로 정했다. 같은 날 설치는 Homebrew로 정했고, 실행
파일 하나가 두 백엔드를 가진다. Jev 키가 있으면 TypeSafe.ai의 Jev를 쓰고,
로컬 모델은 그 옆의 백엔드로 남긴다.

TypeSafe 공식 계약은 `POST https://api.typesafe.ai/v1/systemone`이다.
`Authorization: Bearer`에 키를 넣고, 공식 SDK가 읽는 환경 변수 이름은
`TYPESAFE_API_KEY`다. `model`은 `"jev-latest"`다. 질문 타입은 choice, score,
noul로 이 도구와 같다. choice 옵션은 최대 255개, score 등급은 최대 10개다.
문서상 401은 키 오류, 422는 본문 오류, 429와 529는 재시도 대상이다.

## 목표

Apple Silicon Mac에서 `brew install`로 깔리는 `decide` 실행 파일 하나가
TypeSafe Jev와 로컬 Laya를 모두 수행한다. Python과 libtorch는 그 설치에
없다. 도구 이름, 인자, 검증 문장, 데몬 소켓 경로는 유지한다.

두 백엔드의 확률은 서로 맞출 대상이 아니다. 성공 기준은 갈라진다.

- TypeSafe 백엔드는 공식 응답의 `answers.q`를 도구의 `answer`로 그대로
  싣는다. 키 없이 이 백엔드를 고르면 도구 오류다. 네트워크가 실패한 호출을
  로컬 추론으로 바꾸지 않는다.
- 로컬 백엔드는 Python Laya 0.3.6과 게시 필드가 같다. 게시 필드는 Python 3
  `round(x, 4)` 값이다. choice의 `choice`와 `probabilities`, score의
  `score`와 `probabilities`, noul의 `noul`, 각 답의 `confidence`와
  `action.act_probability`가 여기 들어간다.

## 백엔드 선택

환경 변수 `DECIDE_BACKEND`가 고른다.

| 값 | 동작 |
| --- | --- |
| `typesafe` | `TYPESAFE_API_KEY`로 Jev를 호출한다. 키가 비어 있으면 오류다. |
| `local` | 로컬 체크포인트만 쓴다. 키는 무시한다. |
| 없음 | 키가 있으면 `typesafe`, 없으면 `local`이다. |

한 번 고른 뒤 다른 백엔드로 넘어가지 않는다. TypeSafe가 429 또는 529를
주면 그 요청만 1초 뒤에 한 번 더 보낸다. 두 번째도 실패하면 오류다.
401, 422, 연결 실패는 다시 보내지 않고 오류다. score 등급이 11개 이상이거나
choice 옵션이 256개 이상이면 TypeSafe 백엔드는 호출 전에 오류를 반환한다.
그 요청을 로컬로 넘기지 않는다.

키는 환경 변수로만 읽는다. formula, `.mcp.json`, 저장소, 로그에 넣지 않는다.
제삼자 호스트는 쓰지 않고 `api.typesafe.ai`만 호출한다.

## 설치

사용자 설치는 Homebrew 탭 `sparktype/tap`의 formula `decide`다.
`brew install sparktype/tap/decide`로 깐다. 공개 formula는
`sparktype/homebrew-tap`의 `Formula/decide.rb`이고, 이 저장소의
`packaging/homebrew/decide.rb`는 같은 빌드 절차를 담는다. 버전은 0.0.1이다.
소스 태그는 `https://github.com/sparktype/decide`의 `v0.0.1`이다.
바이너리는 `/opt/homebrew/bin/decide`에 둔다. formula는 Rust 크레이트를 빌드해 실행
파일만 설치한다. Python, torch, 가중치, API 키는 formula에 없다.

이 저장소의 `.mcp.json` 명령은 그 절대 경로다. `args`는 비운다. `env`에
키를 넣지 않는다. `DECIDE_BACKEND`를 파일에 고정하지 않으면 키가 있는
환경에서는 TypeSafe, 없는 환경에서는 로컬이 된다.
경로 검사는 Rust 테스트로 옮긴 뒤 Python 테스트를 지운다.

로컬 가중치는 병목에 넣지 않는다. `LAYA_WEIGHTS`가 있으면 그 디렉터리를
쓰고, 없으면 `~/.cache/huggingface/hub`의 `convaiinnovations/laya`
스냅샷을 쓴다. 둘 다 없으면 로컬 백엔드를 처음 쓸 때 영어와 다국어
서브폴더만 받는다.

## 범위 밖

- 가중치를 실행 파일이나 Homebrew bottle 안에 넣기.
- GPU, 양자화, fp16 로컬 추론. 로컬 기준은 CPU fp32다.
- typed-decisions, `auto_task_detection`, 도구 인자 `model` / `task` / `lang`.
- TypeSafe와 로컬 사이를 호출마다 자동으로 오가기.
- 비공식 Jev 프록시, OpenRouter, Vercel, Cloudflare 경로.
- Intel Mac과 Linux bottle. 첫 formula는 이 머신과 같은 arm64 macOS다.
- 검증 전에 Python 구현을 지우는 것. 비교와 픽스처 생성이 끝난 뒤에만 지운다.

## 로컬 런타임

인코더와 헤드를 한 그래프로 고정할 수 있으면 헤드를 Rust로 다시 쓰지 않는다.
첫 후보는 ONNX다.

1. Python 스크립트가 영어 체크포인트의 `DecisionModel` 전체를 ONNX로 보낸다.
   동적 축은 배치와 시퀀스 길이다. 슬라이딩 주의 마스크가 그래프에 남는다.
2. Rust는 그 그래프를 실행하고 같은 토큰 열의 Python 로짓과 비교한다.
3. 게시 필드가 하나라도 다르면 ONNX를 버리고 Candle로 같은 비교를 한다.
   Candle은 ModernBERT 백본과 Laya 헤드, `layer_types`, 중첩
   `rope_parameters`를 읽는다.
4. 통과한 로컬 런타임만 크레이트에 남긴다.

ONNX가 통과한 뒤 순수 Rust 런타임이 같은 게시 필드를 내면 그쪽으로 링크를
줄인다. 아니면 ONNX Runtime 링크를 유지한다. Candle로 가면 libtorch와
ONNX Runtime은 없다.

TypeSafe 백엔드와 MCP 진입점은 이 게이트를 기다리지 않는다. 로컬 백엔드를
고른 호출만 게이트를 통과한 런타임이 처리한다. 게이트 전에 `DECIDE_BACKEND=local`
이면 준비되지 않았다는 오류를 반환한다.

## 구성

크레이트는 `crates/decide` 하나다. 바이너리 이름은 `decide`다.

```
crates/decide/
  src/
    main.rs           mcp | daemon. 인자 없으면 mcp
    protocol.rs       검증, 질문 dict, DecideResult
    backend.rs        DECIDE_BACKEND와 키로 백엔드를 고름
    typesafe.rs       api.typesafe.ai 호출과 응답 매핑
    sequence.rs       로컬 시퀀스
    postprocess.rs    로컬 온도, 확률, 게시 필드
    route.rs          로컬 analyse와 route
    local.rs          통과한 로컬 런타임
packaging/homebrew/decide.rb
```

`target/`, 내보낸 `*.onnx`, 가중치는 gitignore다.

프로세스:

- `decide`와 `decide mcp`는 stdio MCP다. TypeSafe만 쓰는 프로세스는 가중치를
  올리지 않는다. 로컬 백엔드이면 영어와 다국어를 기동 시 한 번 올린다.
- `decide daemon`은 `~/.cache/decide/decide.sock`에서 JSON 한 줄을 받는다.
  살아 있는 소켓이 있으면 거절하고, 죽은 파일이면 지우고 다시 묶는다.
  유휴 30분이면 종료한다. 데몬은 자기 환경의 `DECIDE_BACKEND`와
  `TYPESAFE_API_KEY`를 쓴다.
- `.claude/hooks/stop_verify.py`는 표준 라이브러리 그대로 둔다. 기동 명령만
  `decide daemon`으로 바꾼다. 환경은 훅 프로세스의 환경을 넘긴다. 소켓이
  없으면 그 호출은 통과시킨다.

## 프로토콜

도구 인자는 `state`, `type`, `instructions`, `options`, `criteria`다.

- choice는 서로 다른 옵션이 2개 미만이면
  `choice 타입은 서로 다른 옵션이 최소 2개 필요합니다`.
- score는 등급이 2개 미만이면
  `score 타입은 등급이 최소 2개 필요합니다`.
- noul에 `options` 또는 `criteria`가 있으면
  `noul 타입은 options/criteria를 받지 않습니다`.
- TypeSafe 백엔드에서 choice가 256개 이상이거나 score가 11개 이상이면
  호출 전에 그 한도를 말하는 오류를 반환한다.

질문 id는 `"q"` 하나다. TypeSafe 요청 본문은 다음과 같다.

```json
{
  "model": "jev-latest",
  "state": "<state 문자열>",
  "questions": {
    "q": { "type": "choice", "instructions": "...", "criteria": { "a": "a", "b": "b" } }
  }
}
```

choice의 `criteria` 값은 지금 `_decide_impl`처럼 옵션 문자열 자신이다.
score의 `criteria`는 등급 리스트다. noul은 `criteria` 없이 보낸다.

반환 껍데기는 같다.

```json
{
  "answer": {},
  "routing": { "backend": "typesafe", "model": "jev-1.13.0" },
  "latency_ms": 180.0
}
```

`answer`는 백엔드가 낸 `"q"` 답이다. TypeSafe noul은 `type`과 `noul`만
갖고 `confidence`와 `action`이 없다. choice와 score는 공식 필드의
`choice` 또는 `score`, `probabilities`, `confidence`, score의 `legend`를
가진다. 로컬 답은 Laya `system_one`의 키를 유지하고 `action`을 포함한다.
`routing.backend`는 `typesafe` 또는 `local`이다. 로컬의 `routing`은 그에
더해 `model`, `reason`, `detection`, `workflow`를 지금 `Router.route`와
같은 키로 싣는다. `detection` 키는 `script`, `script_profile`, `language`,
`is_english`, `language_undecided`, `diacritic_rate`, `non_latin_fraction`이다.
`latency_ms`는 고른 백엔드 호출만 재고, 다른 백엔드의 기동 시간은 넣지 않는다.

데몬은 같은 필드의 JSON 한 줄을 읽고 같은 껍데기 한 줄을 쓴다. 실패하면
`{"error": "..."}`만 돌려주고 프로세스는 유지한다.

로컬 라우팅은 `laya/lang.py`의 `analyse`와 `Router.route`의 검출 분기다.
글자가 없으면 영어, 비라틴이면 다국어, 라틴이라도 영어가 아니면 다국어다.
한글은 다국어 체크포인트다. 시퀀스와 후처리는 `build_sequence`,
`render_options`, `clamp_temperature`, `temp_bucket`, `system_one`을 따른다.
온도는 `[0.5, 5.0]` 밖이거나 숫자가 아니면 1.0이다. `max_len`은 512,
`head_max_len`은 192다.

## 테스트

기본 `cargo test`는 가중치, 네트워크, API 키 없이 돈다.

- TypeSafe 매핑은 기록해 둔 응답 JSON으로 검사한다. 테스트가 실제 키를
  읽거나 `api.typesafe.ai`에 접속하지 않는다.
- 백엔드 선택은 키 있음, 키 없음, `DECIDE_BACKEND` 강제의 네 경우를 본다.
  키가 있을 때 TypeSafe 오류가 로컬 호출로 바뀌지 않는지도 본다.
- 로컬 시퀀스, 후처리, 검증 문장, 스크립트 라우팅은 `laya` 0.3.6이 만든
  JSON 픽스처와 비교한다. 픽스처는 토크나이저만 쓰고 가중치는 쓰지 않는다.
- 로컬 게시 필드 비교는 `cargo test --features parity`다. 기본 테스트와
  `pytest`에 넣지 않는다.
- 로컬 골든 입력은 영어 noul, 옵션 2개인 choice, 옵션 11개 이상인 choice,
  등급 3개인 score, 한글 state다. 한글은 다국어 체크포인트여야 한다.
- `.mcp.json`이 `/opt/homebrew/bin/decide`를 가리키는 검사는 Rust 테스트에
  둔다. `stop_verify.py`의 기동 명령 검사도 정리 전에 그 명령 기준으로 고친다.

## 구현 순서

1. 검증, 백엔드 선택, TypeSafe 요청과 응답 매핑. 기록된 JSON으로 검사한다.
2. `decide mcp`, `decide daemon`, Homebrew formula. 이 시점의 로컬 백엔드는
   준비되지 않았다는 오류를 반환한다.
3. `.mcp.json`을 `/opt/homebrew/bin/decide`로 두고 `stop_verify.py`가
   `decide daemon`을 띄우게 한다. 이 변경과 함께 경로 검사는 Rust 테스트로
   옮긴다.
4. 로컬 시퀀스, 후처리, 라우팅 픽스처.
5. ONNX 비교. 게시 필드가 다르면 Candle로 같은 비교를 하고, 통과한 런타임만
   로컬 백엔드에 연결한다.
6. 아래 실행 검증을 통과하면 이전 Python 구현을 저장소에서 지운다.

1단계는 로컬 런타임 코드를 크레이트에 남기지 않는다. 5단계가 게시 필드
기준을 통과하기 전에는 로컬 백엔드가 추론을 수행하지 않는다. 6단계는
실행 검증 전에 하지 않는다.

## 실행 검증 뒤 정리

실행 검증은 Homebrew로 깔린 `/opt/homebrew/bin/decide`에 대해 다음이 모두
통과한 상태다.

- `cargo test`가 통과한다. 로컬 게시 필드 비교(`cargo test --features parity`)도
  통과한다.
- 키가 있는 환경에서 TypeSafe 백엔드로 `decide` 도구를 한 번 호출하면
  `routing.backend`가 `typesafe`인 답이 온다.
- `DECIDE_BACKEND=local`로 같은 도구를 한 번 호출하면 `routing.backend`가
  `local`인 답이 오고, 한글 state는 다국어 체크포인트로 간다.
- 데몬 소켓에 JSON 한 줄을 보내면 같은 껍데기의 답이 돌아온다.

이 네 가지가 끝나기 전에는 Python 구현을 지우지 않는다. 네 가지가 끝나면
같은 변경에서 이전 구현을 지운다.

지우는 것:

- `src/decide/`
- `tests/`의 Python 테스트
- `test_smoke.py`
- `pyproject.toml`, `.python-version`
- 로컬 게이트에서 ONNX를 만들 때 쓴 Python 스크립트
- README와 `CLAUDE.md`의 `pip`, `.venv`, `pytest` 설치와 실행 절차

남기는 것:

- `crates/decide`와 `packaging/homebrew/decide.rb`
- 로컬 회귀에 쓰는 JSON 픽스처. 이것은 옛 서버가 아니라 Rust 테스트 입력이다.
- `.claude/skills/decide/SKILL.md`. 두 백엔드 설명으로 고친다.
- `.claude/hooks/stop_verify.py`. 모델 서버가 아니라 훅이며, 기동 대상만
  `decide daemon`이다.
- `docs/superpowers/`의 지난 설계 문서.

체크아웃의 `.venv`는 저장소에 없으므로 커밋 대상이 아니다. 검증이 끝나면
그 디렉터리도 삭제한다. 이후 이 저장소의 테스트 명령은 `cargo test`다.
