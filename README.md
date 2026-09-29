# decide

![decide 배너. 저울 한쪽에 호박색 돌이 놓여 있다.](docs/banner.jpg)

로컬 [Laya](https://pypi.org/project/laya/) 판단 모델을 MCP 도구 `decide`로 연다. 문장을 생성하지 않고, 선택(`choice`)·순서형 점수(`score`)·확률(`noul`)을 한 번의 순전파로 돌려준다. 출력 형식은 고정돼 있다. 판단이 맞는지와는 별개다.

## 붙이기

Python 3.12. 이 저장소의 `.python-version`은 `3.12.13`이다.

```bash
python3.12 -m venv .venv
.venv/bin/pip install -e ".[dev]"
```

`laya`가 torch와 transformers를 끌어온다. 첫 설치는 몇 분이 걸릴 수 있다.

Claude Code는 이 폴더의 `.mcp.json`으로 세션당 서버를 한 번 띄운다. 명령은 이 체크아웃의 가상환경을 가리킨다.

```json
{
  "mcpServers": {
    "decide": {
      "command": "/Users/spark/Develop/Workspaces/decide-mcp/.venv/bin/decide-mcp",
      "args": []
    }
  }
}
```

다른 경로에 받으면 `command`만 그 체크아웃의 `.venv/bin/decide-mcp`로 바꾼다. `.mcp.json`을 고친 뒤에는 Claude Code를 다시 연다. 이미 떠 있는 세션은 등록을 다시 읽지 않는다.

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

`noul`에는 `options`와 `criteria`를 넣지 않는다.

```text
decide(state="서버가 다운됐습니다", instructions="이 요청이 긴급한가?", type="noul")
decide(state="청구서가 중복 결제됐습니다", instructions="어느 팀이 처리해야 하는가?", type="choice", options=["billing", "technical", "sales"])
decide(state="이미 세 번째 문의입니다", instructions="고객의 불만 강도는?", type="score", criteria=["낮음", "보통", "높음"])
```

반환은 `answer`, `routing`, `latency_ms`다.

- `choice`의 `answer.choice`가 고른 라벨이고 `answer.confidence`가 신뢰도다.
- `score`의 `answer.score`는 0부터 등급 개수−1 사이의 기대값이다.
- `noul`의 `answer.noul`은 참/거짓이 아니라 0.0에서 1.0 사이의 확률이다. 임계값은 부르는 쪽에서 정한다.

`instructions`는 상태를 바로 묻는 문장으로 쓴다. "Does the customer express satisfaction?"처럼. "Is this NOT a positive review?" 같은 반문은 방향이 쉽게 뒤집힌다. 갈림이 분명해야 하면 `noul` 대신 `choice`에 `positive`와 `negative`를 넣는 편이 안정적이다.

호출이 실패하면 그 오류로 작업을 멈추지 않고 직접 추론해서 계속한다. 검증 오류와 모델 오류는 `ToolError`로 돌아오고, 서버 세션은 유지된다.

첫 호출은 체크포인트 세 개를 올린다. CPU에서 콜드 스타트는 약 90초까지 갈 수 있고, 그 프로세스 안의 다음 호출은 수백 밀리초다. 가중치는 처음 받을 때 네트워크가 필요하다.

## 개발 참고

서버는 `src/decide/server.py`다. 전송은 stdio이고, 클래스는 `mcp.server.MCPServer`다. `FastMCP`가 아니다.

판단 로직은 `_decide_impl`에 있고 `predict_fn`을 주입받는다. 단위 테스트는 가짜 함수로 이 층을 본다. Laya를 실제로 로드하는 단언은 `tests/`에 넣지 않는다. 실제 가중치 확인은 수동 스모크다.

```bash
.venv/bin/python -m pytest
.venv/bin/python test_smoke.py
```

MCP 세션은 프로세스를 살려 두므로 `laya.Router(preload=True)`가 세션당 한 번만 올라간다. Stop 훅처럼 호출마다 프로세스가 새로 생기는 곳은 `src/decide/decide_daemon.py`를 쓴다. 소켓은 `~/.cache/decide/decide.sock`이고, 30분 동안 요청이 없으면 데몬이 끝난다. `.claude/hooks/stop_verify.py`는 소켓이 없으면 데몬을 백그라운드로 띄우고 그 호출은 막지 않고 통과시킨다. 훅에서 `build_router_predict_fn()`을 직접 부르면 호출마다 콜드 스타트 비용을 낸다.

설계 배경은 `docs/superpowers/specs/2026-09-23-guru-local-decision-tooling-design.md`에 있다.
