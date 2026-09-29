# decide

![decide 배너. 저울 한쪽에 호박색 돌이 놓여 있다.](docs/banner.jpg)

판단 한 건을 MCP 도구 `decide`로 연다. 문장을 생성하지 않고, 선택(`choice`)·순서형 점수(`score`)·확률(`noul`)을 돌려준다. 출력 형식은 고정돼 있다. 판단이 맞는지와는 별개다.

실행 파일 하나가 백엔드를 둘 가진다.

| `DECIDE_BACKEND` | 동작 |
| --- | --- |
| `typesafe` | `TYPESAFE_API_KEY`로 [TypeSafe.ai](https://api.typesafe.ai) Jev를 호출한다. 키가 비어 있으면 오류다. |
| `local` | 로컬 Laya 체크포인트만 쓴다. 키는 읽지 않는다. |
| 없음 | 키가 있으면 TypeSafe, 없으면 로컬이다. |

한 요청은 고른 백엔드에서만 끝난다. TypeSafe가 429 또는 529를 주면 그 요청만 1초 뒤에 한 번 더 보낸다. 401, 422, 연결 실패는 그 호출의 오류다. 두 백엔드의 확률을 서로 같은 값으로 맞추지는 않는다.

로컬 게시 필드 비교가 끝나기 전에는 `DECIDE_BACKEND=local`이 `로컬 백엔드가 아직 준비되지 않았습니다`를 반환한다.

## 붙이기

Apple Silicon Mac에서는 탭으로 깐다.

```bash
brew install sparktype/tap/decide
```

버전은 0.0.1이다. 바이너리는 `/opt/homebrew/bin/decide`다. formula가 설치하는 것은 그 실행 파일이다. 가중치와 API 키는 병 밖에 둔다.

`.mcp.json`의 `decide` 명령은 그 절대 경로다. `args`는 비운다. 키와 `DECIDE_BACKEND`는 이 파일에 적지 않는다. 프로세스 환경에 키가 있으면 TypeSafe를 쓰고, 없으면 로컬을 쓴다.

```json
{
  "mcpServers": {
    "decide": {
      "command": "/opt/homebrew/bin/decide",
      "args": []
    }
  }
}
```

`.mcp.json`을 고친 뒤에는 세션을 다시 연다. 이미 떠 있는 세션은 등록을 다시 읽지 않는다.

`decide`와 `decide mcp`는 stdio MCP다. `decide daemon`은 `~/.cache/decide/decide.sock`에서 JSON 한 줄을 받고, 30분 동안 요청이 없으면 끝난다. `--help`는 서버를 띄우지 않는다.

이 폴더를 열면 `.claude/skills/decide/SKILL.md`도 같이 읽힌다. 도구를 언제 부르고 언제 직접 추론할지는 그 스킬이 안내한다.

## 사용법

도구 인자는 다섯 개다.

| 인자 | 역할 |
| --- | --- |
| `state` | 판단할 내용 |
| `instructions` | 고정된 질문 |
| `type` | `choice`, `score`, `noul` 중 하나 |
| `options` | `choice`일 때 서로 다른 선택지. 최소 2개 |
| `criteria` | `score`일 때 낮은 쪽부터 나열한 등급. 최소 2개 |

`noul`에는 `options`와 `criteria`를 넣지 않는다. TypeSafe에서 `choice` 옵션은 255개까지, `score` 등급은 10개까지다. 그 한도를 넘으면 호출 전에 오류가 난다.

```text
decide(state="서버가 다운됐습니다", instructions="이 요청이 긴급한가?", type="noul")
decide(state="청구서가 중복 결제됐습니다", instructions="어느 팀이 처리해야 하는가?", type="choice", options=["billing", "technical", "sales"])
decide(state="이미 세 번째 문의입니다", instructions="고객의 불만 강도는?", type="score", criteria=["낮음", "보통", "높음"])
```

반환은 `answer`, `routing`, `latency_ms`다. `routing.backend`는 `typesafe` 또는 `local`이다.

- `choice`의 `answer.choice`가 고른 라벨이고 `answer.confidence`가 신뢰도다.
- `score`의 `answer.score`는 0부터 등급 개수−1 사이의 기대값이다.
- `noul`의 `answer.noul`은 참/거짓이 아니라 0.0에서 1.0 사이의 확률이다. 임계값은 부르는 쪽에서 정한다. TypeSafe의 noul 답은 `type`과 `noul`만 가진다.

`instructions`는 상태를 바로 묻는 문장으로 쓴다. "Does the customer express satisfaction?"처럼. "Is this NOT a positive review?" 같은 반문은 방향이 쉽게 뒤집힌다. 갈림이 분명해야 하면 `noul` 대신 `choice`에 `positive`와 `negative`를 넣는 편이 안정적이다.

호출이 실패하면 그 오류로 작업을 멈추지 않고 직접 추론해서 계속한다. 검증 오류와 백엔드 오류는 도구 오류로 돌아오고, 서버 세션은 유지된다.

키는 환경 변수 `TYPESAFE_API_KEY`로만 읽는다. 호출 주소는 `https://api.typesafe.ai/v1/systemone`이고, 요청의 `model`은 `jev-latest`다.

## 개발

런타임은 `crates/decide`다. 기본 테스트는 가중치와 네트워크 없이 돈다.

```bash
cargo test --manifest-path crates/decide/Cargo.toml
```

Python 패키지 `src/decide`와 `pytest`는 로컬 게시 필드 비교가 끝날 때까지 이 저장소에 있다. 비교에 쓰는 수동 확인은 `test_smoke.py`다.

```bash
.venv/bin/python -m pytest
.venv/bin/python test_smoke.py
```

Stop 훅 `.claude/hooks/stop_verify.py`는 소켓이 없으면 `/opt/homebrew/bin/decide daemon`을 백그라운드로 띄우고, 그 호출은 통과시킨다.

설계는 `docs/superpowers/specs/2026-09-29-decide-rust-runtime-design.md`에 있다.
