# decide

![decide 배너. choice는 B, score는 3/5, noul은 0.82를 돌려준다.](docs/banner.svg)

판단 한 건을 MCP 도구 `decide`로 연다. 문장을 생성하지 않고, 선택(`choice`)·순서형 점수(`score`)·확률(`noul`)을 돌려준다. 출력 형식은 고정돼 있다. 판단이 맞는지와는 별개다.

실행 파일 하나가 백엔드를 둘 가진다.

| `DECIDE_BACKEND` | 동작 |
| --- | --- |
| `typesafe` | `TYPESAFE_API_KEY`로 [TypeSafe.ai](https://api.typesafe.ai) Jev를 호출한다. 키가 비어 있으면 오류다. |
| `local` | 내 Mac에서 Clef-flash를 직접 돌린다. 서버도, 키도 필요 없다. |
| 없음 | 키가 있으면 TypeSafe, 없으면 로컬이다. |

한 요청은 고른 백엔드에서만 끝난다. 429 또는 529를 받으면 그 요청만 1초 뒤에 한 번 더 보낸다. 401, 422, 연결 실패는 그 호출의 오류다. 두 백엔드의 확률을 서로 같은 값으로 맞추지는 않는다.

로컬 백엔드는 Cloudflare의 Clef-flash(Qwen3.5-9B 하이브리드 백본, GGUF Q6_K, candle-transformers PR #3396에서 벤더링)와 자체 구현한 joint schema head를 같은 프로세스 안에서 직접 돌린다. 따로 띄우는 서버가 없다. 설정은 아래 "로컬 백엔드"를 본다.

## 붙이기

Apple Silicon Mac에서는 탭으로 깐다.

```bash
brew install sparktype/tap/decide
```

버전은 0.0.6다. 바이너리는 `/opt/homebrew/bin/decide`다. formula는 GitHub Release에 올라간 사전 빌드 arm64 바이너리를 받아 그대로 설치한다 — 설치에 Rust 툴체인이 필요 없다. 가중치와 API 키는 병 밖에 둔다.

```bash
decide install
```

`brew install` 뒤에 위 명령을 실행하면 `claude mcp add -s user decide -- /opt/homebrew/bin/decide mcp`를 대신 실행해 Claude Code 사용자 스코프에 `decide`를 등록한다. `.mcp.json`을 손으로 고칠 필요는 없다. `decide install --claude`는 여기에 더해 표시 훅(`decide hook`, 아래 "decide가 고른 것을 눈으로 보기")도 `~/.claude/settings.json`에 넣는다(0.0.6부터). 이 저장소처럼 프로젝트 스코프로 등록하고 싶으면 `.mcp.json`을 직접 쓴다.

```json
{
  "mcpServers": {
    "decide": {
      "command": "/opt/homebrew/bin/decide",
      "args": ["mcp"]
    }
  }
}
```

`.mcp.json`을 고친 뒤에는 세션을 다시 연다. 이미 떠 있는 세션은 등록을 다시 읽지 않는다.

`decide mcp`가 stdio MCP다. 인자 없이 실행하면 도움말이다. `decide daemon`은 `~/.cache/decide/decide.sock`에서 JSON 한 줄을 받고, 30분 동안 요청이 없으면 끝난다. `--help`는 서버를 띄우지 않는다.

도구를 언제 부르고 언제 직접 추론할지는 아래 "에이전트가 쓸 때"를 따른다. 스킬 파일 `.claude/skills/decide/SKILL.md`는 `.gitignore` 대상이라 이 저장소에 포함되지 않는다.

## 로컬 백엔드

키가 없거나 `DECIDE_BACKEND=local`이면 `decide`는 Clef-flash를 그 프로세스 안에서 직접 돌린다. 띄워 둬야 할 서버가 없다.

가중치는 두 파일이다: 백본 GGUF(`clef-flash.Q6_K.gguf`, 약 7.4GB)와 joint head(`joint_head.safetensors`).

- `CLEF_WEIGHTS`에 디렉터리를 넣으면 그 안에서 두 파일을 찾는다. 하나라도 없으면 다운로드로 넘어가지 않고 그대로 오류다.
- `CLEF_WEIGHTS`가 없으면 처음 쓸 때 HuggingFace에서 받아 `~/.cache/huggingface/hub`에 둔다(백본은 `prithivMLmods/clef-flash-GGUF`, joint head와 토크나이저는 `Cloudflare/clef-flash`에서). 첫 호출은 그만큼 시간과 디스크를 쓴다.
- 로컬 답은 TypeSafe 답과 같은 모양이다(`choice`/`score`/`noul`, choice·score의 `probabilities`와 `confidence`, score의 `legend`). `routing.model`은 `"clef-flash"`다.
- 5개 골든 케이스로 BF16 Python 오라클(`scripts/clef_flash_oracle.py`, `cargo test --features parity`)과 대조한 결과 4/5가 질적으로 일치한다. 어긋난 1개는 선택지가 11개인 choice 질문으로, 오라클 자체도 1·2위 확률 차이가 0.02 안쪽인 거의 동률 사례다 — 양자화 버그가 아니라 9B 모델을 CPU에서 4~6비트로 돌리는 데 따르는, 측정되고 받아들인 한계다. 중요한 판단이면 `probabilities`를 보고 직접 확인한다.

## 사용법

도구 인자는 다섯 개다.

| 인자 | 역할 |
| --- | --- |
| `state` | 판단할 내용 |
| `instructions` | 고정된 질문 |
| `type` | `choice`, `score`, `noul` 중 하나 |
| `options` | `choice`일 때 서로 다른 선택지. 최소 2개 |
| `criteria` | `score`일 때 낮은 쪽부터 나열한 등급. 최소 2개 |

`noul`에는 `options`와 `criteria`를 넣지 않는다. 옵션 한도는 백엔드마다 다르다.

| 백엔드 | `choice` 옵션 | `score` 등급 |
| --- | --- | --- |
| TypeSafe | 255개까지. 넘으면 호출 전에 오류가 난다. | 10개까지. 넘으면 호출 전에 오류가 난다. |
| 로컬 | 코드에 별도 상한은 없다. 모델의 컨텍스트 길이 안에서만 제한된다. | 코드에 별도 상한은 없다. |

```text
decide(state="서버가 다운됐습니다", instructions="이 요청이 긴급한가?", type="noul")
decide(state="청구서가 중복 결제됐습니다", instructions="어느 팀이 처리해야 하는가?", type="choice", options=["billing", "technical", "sales"])
decide(state="이미 세 번째 문의입니다", instructions="고객의 불만 강도는?", type="score", criteria=["낮음", "보통", "높음"])
```

반환은 `answer`, `routing`, `latency_ms`다. `routing.backend`는 `typesafe` 또는 `local`이다. `decide daemon`을 거친 호출이 이전 요청과 `state`·`type`·`instructions`·`options`·`criteria`·백엔드가 모두 같으면 `routing.cached`가 `true`이고 `latency_ms`는 0이다 — 이 캐시는 데몬 프로세스 안에서만 유지되고, `decide mcp`(요청마다 새 프로세스)에는 없다.

- `choice`의 `answer.choice`가 고른 라벨이고 `answer.confidence`가 신뢰도다.
- `score`의 `answer.score`는 0부터 등급 개수−1 사이의 기대값이다.
- `noul`의 `answer.noul`은 참/거짓이 아니라 0.0에서 1.0 사이의 확률이다. 임계값은 부르는 쪽에서 정한다. 두 백엔드 모두 noul 답은 `type`과 `noul`만 가진다(`confidence` 없음).

`instructions`는 상태를 바로 묻는 문장으로 쓴다. "Does the customer express satisfaction?"처럼. "Is this NOT a positive review?" 같은 반문은 방향이 쉽게 뒤집힌다. 갈림이 분명해야 하면 `noul` 대신 `choice`에 `positive`와 `negative`를 넣는 편이 안정적이다.

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

반환은 `answers`(질문 id → 답, 입력 순서), `routing`, `latency_ms`다. 하나라도 검증에 실패하거나 백엔드가 실패하면 전체가 도구 오류이고 부분 결과는 없다. 검증 오류 앞에는 `질문 "<id>": `가 붙는다. 질문 개수 상한은 두지 않고 백엔드가 거절하는 대로 돌려준다. 로컬 백엔드는 질문마다 추론을 다시 돌리므로 질문 수에 비례해 시간이 든다. 이점은 모델 지연이 아니라 에이전트의 도구 호출 횟수가 줄어드는 것이다.

질문 하나하나는 yes/no나 단일 선택처럼 작게 쪼갠다. 복합 질문은 정확도가 떨어진다. 앞 답에 따라 뒤 질문을 정해야 하면 호출을 나눈다. `decide daemon` 소켓도 줄에 `questions`가 있으면 같은 모양으로 답한다(`type`과 함께 줄 수 없다).

### 에이전트가 쓸 때

- **언제 부르나.** 답이 선택지·등급·확률로 고정되는 판단 한 건일 때 부른다. 서술, 요약, 계획처럼 글을 만들어야 하는 일에는 쓰지 않는다.
- **실패하면.** 그 오류로 작업을 멈추지 않고 직접 추론해서 계속한다. 검증 오류와 백엔드 오류는 도구 오류로 돌아온다. 로컬 백엔드의 실패는 보통 가중치 다운로드·로드 오류이니 사용자에게 한 줄로 알린다.
- **어느 백엔드가 답했나.** `routing.backend`를 본다. 같은 질문이라도 TypeSafe와 로컬의 확률은 보정이 달라 섞어서 비교하지 않는다.
- **임계값.** `noul`을 조건으로 쓸 때 기준은 부르는 쪽이 정한다. 경계 근처(0.3~0.7)면 한 번 더 확인한다.

키는 환경 변수 `TYPESAFE_API_KEY`로만 읽는다. TypeSafe 호출 주소는 `https://api.typesafe.ai/v1/systemone`이고, 요청의 `model`은 `jev-latest`다. 로컬 백엔드는 네트워크 호출이 아니라 같은 프로세스 안의 추론이다.

## decide가 고른 것을 눈으로 보기

에이전트가 `decide`를 부르면 결과는 도구 결과 안에 JSON으로만 있다. `decide hook`은 Claude Code `PostToolUse` 훅으로 붙어 질문과 결과를 한 줄 요약으로 사용자에게 보여준다. 공식 문서에 따르면 이 요약(`systemMessage`)은 사용자에게만 보이고 모델은 보지 않는다.

```text
🔎 decide가 선택했습니다: "푸시하고 PR 생성" (96%)
   질문: 이 브랜치를 마무리하는 가장 적절한 방법은?
   나머지: 브랜치를 그대로 유지 3% · main에 로컬로 머지 1% · 작업을 폐기 0%
   typesafe · jev-1.13.0 · 203ms
```

가장 쉬운 방법은 다음 한 줄이다. MCP 등록과 훅 등록을 함께 하고, 여러 번 실행해도 안전하다.

```bash
decide install --claude
```

`~/.claude/settings.json`(`CLAUDE_CONFIG_DIR`이 있으면 그 아래)의 `hooks.PostToolUse`에 그룹 하나를 덧붙이고, 다른 설정은 순서까지 그대로 둔다. 같은 명령이 이미 있으면 아무것도 바꾸지 않는다. 설정이 깨진 JSON이거나 병합할 수 없는 모양이면 파일을 쓰지 않고 오류로 끝난다. 바꾸기 전에 `settings.json.bak-decide`로 백업을 남긴다. JSON은 2칸 들여쓰기로 다시 쓰기 때문에 공백 모양은 달라질 수 있다.

직접 넣으려면 `~/.claude/settings.json`(또는 프로젝트 설정)에 다음을 넣고 세션을 다시 연다.

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

**아직 실제 Claude Code 세션에서 확인하지 않았다.** MCP 도구 결과가 훅에 어떤 모양으로 오는지(문자열, 객체, 배열)와 요약이 화면에 어떻게 보이는지는 문서로만 확인했다. `decide hook`은 세 모양을 모두 읽도록 만들었지만, 등록한 뒤 한 줄이 실제로 보이는지 직접 확인해 달라.

끄려면 `settings.json`의 `mcp__decide__decide` 훅 그룹을 지운다(제거 명령은 아직 없다). 문제가 생기면 `settings.json.bak-decide`가 바꾸기 전 원본이다. 읽을 수 없는 입력에는 아무것도 출력하지 않고 종료 코드 0이라 훅이 에이전트 작업을 막지 않는다. `decide_many`의 표시는 아직 없다. 훅은 `decide` 바이너리에 들어 있어서 0.0.5 이상에서만 동작한다.

## 개발

런타임은 `crates/decide`다. 기본 테스트는 가중치와 네트워크 없이 돈다.

```bash
cargo test --manifest-path crates/decide/Cargo.toml
```

이전 Python 패키지(Jev-Style 서버 호출 방식의 비교용)는 그 백엔드의 실행 검증이 끝나 지웠다. Clef-flash의 비교 오라클은 `scripts/clef_flash_oracle.py`(Python `transformers`, 실행에 가중치가 필요해 기본 테스트에 넣지 않는다)이고, `cargo test --features parity`로 돈다. 기본 테스트 명령은 `cargo test`뿐이다. 저장소에 남은 Python은 그 오라클 스크립트와 표준 라이브러리만 쓰는 Stop 훅이다.

Stop 훅 `.claude/hooks/stop_verify.py`는 소켓이 없으면 `/opt/homebrew/bin/decide daemon`을 백그라운드로 띄우고, 그 호출은 통과시킨다. 로컬 백엔드가 가중치 다운로드·로드에 실패하면 데몬 호출이 오류가 되고, 훅은 그 오류를 통과로 처리한다. 두 백엔드의 확률은 보정이 달라서, 백엔드를 바꾸면 임계값이 여전히 맞는지 확인한다. 노울 신뢰도 임계값(기본 0.4)은 `DECIDE_STOP_THRESHOLD` 환경 변수로 바꿀 수 있다.

설계는 `docs/superpowers/specs/2026-09-29-decide-rust-runtime-design.md`에 있다. 로컬 백엔드는 `docs/superpowers/specs/2026-10-02-clef-flash-local-backend-design.md`를 따른다.
