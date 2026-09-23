# guru MCP 서버(decide tool) + Skill Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Claude Code가 세션당 1회 stdio로 스폰하는 `guru` MCP 서버에서, 로컬 Laya 판단 모델(choice/score/noul)을 `decide` tool로 호출할 수 있게 한다.

**Architecture:** `mcp` SDK의 `MCPServer`로 만든 단일 파일 stdio 서버가 `decide` tool 하나를 노출한다. 검증/변환/응답 조립 로직(`_decide_impl`)은 `predict_fn`을 주입받는 순수 함수로 분리해 실제 모델 없이 단위 테스트한다. 실제 `laya.Router(preload=True)`는 모듈 전역 싱글톤으로 첫 호출 시 지연 로딩한다. 데몬 자동 스폰·포트 관리·헬스체크는 필요 없다 — Claude Code가 `.mcp.json`을 보고 서버 프로세스 생명주기를 직접 관리한다.

**Tech Stack:** Python 3.10+, `mcp`(PyPI, 2.x, `mcp.server.MCPServer`, stdio 기본 transport), `laya`(PyPI, `Router` API), pytest + anyio(테스트 전용).

**Spec:** `docs/superpowers/specs/2026-09-23-guru-local-decision-tooling-design.md`

> 이 플랜은 이전의 HTTP 데몬 + CLI 방식 플랜을 대체한다. "stdio로 통신하면
> 어떨지"라는 요청을 검토한 결과, 순수 stdio 파이프는 데몬 하나를 여러 독립
> 프로세스가 공유하는 구조와 근본적으로 안 맞아 MCP 서버(Claude Code가 프로세스
> 생명주기를 관리)로 전환했다. 스펙 문서의 "검토한 대안과 선택" 참고.

## Global Constraints

- MCP 서버는 stdio 기본 transport를 그대로 쓴다 — HTTP/SSE 등 다른 transport를 추가하지 않는다.
- 제품명은 전부 `guru`로 통일한다 — "Jev"를 코드/문서/tool 설명에 노출하지 않는다.
- Python 3.10 이상을 요구한다 (`laya`/`mcp` 공통 요구사항).
- `decide` tool의 `noul` 결과는 boolean이 아니라 0.0~1.0 확률이다 — 어디에서도 boolean으로 강제 변환하지 않는다.
- tool 함수에서 발생하는 모든 예외(입력 검증 실패든 모델 로드 실패든)는 `ToolError`로 변환해 반환한다 — 원본 예외를 그대로 던져 MCP 세션을 죽이지 않는다.
- 2단계(PreToolUse Hook 자동 게이팅)는 이번 플랜 범위 밖이다 — 코드를 만들지 않는다.

## Review Focus

- **choice 타입에 옵션이 1개뿐이거나 없음** — 판단이 무의미하므로 최소 2개를 요구하는 검증이 있어야 한다. (Task 1)
- **score 타입에 등급이 1개뿐이거나 없음** — 마찬가지로 최소 2개를 요구해야 한다. (Task 1)
- **noul 타입에 options/criteria가 잘못 주어짐** — noul은 순수 확률 추정이라 둘 다 받지 않아야 한다. (Task 1)
- **모델 로드/추론 중 예상 못한 예외**(네트워크 없어 가중치 다운로드 실패 등) — `ValueError`가 아닌 임의의 예외가 나도 MCP 세션이 죽지 않고 `ToolError`로 Claude에게 명확히 전달돼야 한다. (Task 3)
- **`.mcp.json`의 `command`가 실제 설치된 콘솔 스크립트 이름과 다름** — Claude Code가 서버를 아예 못 띄우는 조용한 실패로 이어지므로, 두 값이 항상 일치하는지 테스트로 고정한다. (Task 4)

---

### Task 1: 프로젝트 스캐폴딩 + decide 로직(순수 함수, predict_fn 주입)

**Files:**
- Create: `pyproject.toml`
- Create: `src/guru/__init__.py`
- Create: `src/guru/server.py` (이 태스크에서는 `DecideResult`, `_decide_impl`만)
- Test: `tests/test_decide_impl.py`

**Interfaces:**
- Produces: `guru.server.DecideResult`(pydantic `BaseModel`: `answer: dict`, `routing: dict`, `latency_ms: float`), `guru.server.PredictFn`(타입 별칭, `Callable[[str, dict], dict]`), `guru.server._decide_impl(state: str, type: Literal["choice","score","noul"], instructions: str, options: list[str] | None, criteria: list[str] | None, predict_fn: PredictFn) -> DecideResult`(검증 실패 시 `ValueError`)

- [ ] **Step 1: pyproject.toml 작성**

```toml
[build-system]
requires = ["hatchling"]
build-backend = "hatchling.build"

[project]
name = "guru"
version = "0.1.0"
requires-python = ">=3.10"
dependencies = [
    "laya>=0.3.6",
    "mcp>=2.2.0,<3",
]

[project.optional-dependencies]
dev = ["pytest>=8.0", "anyio>=4.0"]

[project.scripts]
guru-mcp = "guru.server:main"

[tool.hatch.build.targets.wheel]
packages = ["src/guru"]
```

- [ ] **Step 2: 패키지 설치**

Run: `~/.pyenv/shims/pip3 install -e ".[dev]"`
Expected: 성공적으로 설치됨 (laya/torch/transformers 다운로드로 수 분 소요 가능 — 정상)

- [ ] **Step 3: 빈 패키지 초기화**

`src/guru/__init__.py`: (빈 파일)

- [ ] **Step 4: 실패하는 테스트 작성**

`tests/test_decide_impl.py`:
```python
import pytest

from guru.server import DecideResult, _decide_impl


def test_noul_builds_question_without_criteria():
    captured = {}

    def recording(state, questions):
        captured["questions"] = questions
        return {"answers": {"q": {"noul": 0.9}}, "routing": {}}

    result = _decide_impl("2+2=4", "noul", "참인가?", None, None, recording)

    assert captured["questions"] == {"q": {"type": "noul", "instructions": "참인가?"}}
    assert isinstance(result, DecideResult)
    assert result.answer == {"noul": 0.9}
    assert result.routing == {}
    assert result.latency_ms >= 0


def test_choice_converts_options_to_criteria_dict():
    captured = {}

    def recording(state, questions):
        captured["questions"] = questions
        return {"answers": {"q": {"choice": "a", "confidence": 0.5}}, "routing": {}}

    _decide_impl("text", "choice", "골라라", ["a", "b", "c"], None, recording)

    assert captured["questions"]["q"]["criteria"] == {"a": "a", "b": "b", "c": "c"}


def test_score_passes_criteria_list_in_order():
    captured = {}

    def recording(state, questions):
        captured["questions"] = questions
        return {"answers": {"q": {"score": 1.0, "confidence": 0.5}}, "routing": {}}

    _decide_impl("text", "score", "평가해라", None, ["low", "medium", "high"], recording)

    assert captured["questions"]["q"]["criteria"] == ["low", "medium", "high"]


def test_choice_requires_at_least_two_options():
    with pytest.raises(ValueError, match="옵션"):
        _decide_impl("text", "choice", "골라라", ["only-one"], None, lambda s, q: {})


def test_choice_requires_options_present():
    with pytest.raises(ValueError, match="옵션"):
        _decide_impl("text", "choice", "골라라", None, None, lambda s, q: {})


def test_score_requires_at_least_two_criteria():
    with pytest.raises(ValueError, match="등급"):
        _decide_impl("text", "score", "평가해라", None, ["only-one"], lambda s, q: {})


def test_noul_rejects_options_or_criteria():
    with pytest.raises(ValueError, match="noul"):
        _decide_impl("text", "noul", "참인가?", ["a", "b"], None, lambda s, q: {})
```

- [ ] **Step 5: 테스트 실행해서 실패 확인**

Run: `~/.pyenv/shims/python3 -m pytest tests/test_decide_impl.py -v`
Expected: FAIL — `ModuleNotFoundError: No module named 'guru.server'`

- [ ] **Step 6: 최소 구현 작성**

`src/guru/server.py`:
```python
import time
from typing import Callable, Literal

from pydantic import BaseModel


class DecideResult(BaseModel):
    answer: dict
    routing: dict
    latency_ms: float


PredictFn = Callable[[str, dict], dict]


def _decide_impl(
    state: str,
    type: Literal["choice", "score", "noul"],
    instructions: str,
    options: list[str] | None,
    criteria: list[str] | None,
    predict_fn: PredictFn,
) -> DecideResult:
    question: dict = {"type": type, "instructions": instructions}

    if type == "choice":
        if not options or len(options) < 2:
            raise ValueError("choice 타입은 옵션이 최소 2개 필요합니다")
        question["criteria"] = {o: o for o in options}
    elif type == "score":
        if not criteria or len(criteria) < 2:
            raise ValueError("score 타입은 등급이 최소 2개 필요합니다")
        question["criteria"] = criteria
    else:  # noul
        if options or criteria:
            raise ValueError("noul 타입은 options/criteria를 받지 않습니다")

    started = time.perf_counter()
    result = predict_fn(state, {"q": question})
    latency_ms = (time.perf_counter() - started) * 1000

    return DecideResult(
        answer=result["answers"]["q"],
        routing=result.get("routing", {}),
        latency_ms=latency_ms,
    )
```

- [ ] **Step 7: 테스트 실행해서 통과 확인**

Run: `~/.pyenv/shims/python3 -m pytest tests/test_decide_impl.py -v`
Expected: 7 passed

- [ ] **Step 8: 커밋**

```bash
git add pyproject.toml src/guru/__init__.py src/guru/server.py tests/test_decide_impl.py
git commit -m "feat: guru 패키지 스캐폴딩과 predict_fn 주입형 decide 로직 추가"
```

---

### Task 2: 실제 Laya Router 연결 (지연 싱글톤)

**Files:**
- Modify: `src/guru/server.py`
- Test: `tests/test_router_predict_fn.py`

**Interfaces:**
- Consumes: Task 1의 `PredictFn`
- Produces: `guru.server.build_router_predict_fn() -> PredictFn` — 최초 호출 시 `laya.Router(preload=True)`를 1회 생성해 모듈 전역에 캐싱하고, 이후 호출은 캐싱된 인스턴스의 `.predict`를 재사용한다.

- [ ] **Step 1: 실패하는 테스트 작성 (laya.Router를 스텁으로 대체)**

`tests/test_router_predict_fn.py`:
```python
import guru.server as server_module


def test_build_router_predict_fn_preloads_once_and_reuses(monkeypatch):
    created = []

    class FakeRouter:
        def __init__(self, preload):
            created.append(preload)

        def predict(self, state, questions):
            return {"answers": {"q": {"noul": 0.5}}, "routing": {}}

    monkeypatch.setattr(server_module, "_router_singleton", None)
    monkeypatch.setattr(server_module.laya, "Router", FakeRouter)

    predict_fn = server_module.build_router_predict_fn()
    predict_fn("state a", {"q": {"type": "noul", "instructions": "?"}})
    predict_fn2 = server_module.build_router_predict_fn()
    predict_fn2("state b", {"q": {"type": "noul", "instructions": "?"}})

    assert created == [True]
```

- [ ] **Step 2: 테스트 실행해서 실패 확인**

Run: `~/.pyenv/shims/python3 -m pytest tests/test_router_predict_fn.py -v`
Expected: FAIL — `AttributeError: module 'guru.server' has no attribute 'laya'` (또는 `build_router_predict_fn` 없음)

- [ ] **Step 3: 구현 추가**

`src/guru/server.py` 상단에 추가:
```python
import laya
```

파일 하단에 추가:
```python
_router_singleton = None


def build_router_predict_fn() -> PredictFn:
    def predict(state: str, questions: dict) -> dict:
        global _router_singleton
        if _router_singleton is None:
            _router_singleton = laya.Router(preload=True)
        return _router_singleton.predict(state, questions)

    return predict
```

- [ ] **Step 4: 테스트 실행해서 통과 확인**

Run: `~/.pyenv/shims/python3 -m pytest tests/test_router_predict_fn.py -v`
Expected: 1 passed (laya는 import되지만 테스트가 `laya.Router`를 스텁으로 교체했으므로 실제 모델은 로드하지 않음)

- [ ] **Step 5: 커밋**

```bash
git add src/guru/server.py tests/test_router_predict_fn.py
git commit -m "feat: laya Router를 preload로 1회 로드해 재사용하는 predict_fn 추가"
```

---

### Task 3: MCP 서버 wiring (`decide` tool, ToolError 변환, entrypoint)

**Files:**
- Modify: `src/guru/server.py`
- Test: `tests/test_tool_error_handling.py`

**Interfaces:**
- Consumes: Task 1의 `_decide_impl`, Task 2의 `build_router_predict_fn`
- Produces: `guru.server._decide_tool_body(state, type, instructions, options, criteria) -> DecideResult`(모든 예외를 `mcp.server.mcpserver.exceptions.ToolError`로 변환), `guru.server.mcp`(`MCPServer` 인스턴스, `decide` tool 등록됨), `guru.server.main() -> None`

- [ ] **Step 1: 실패하는 테스트 작성**

`tests/test_tool_error_handling.py`:
```python
import pytest
from mcp.server.mcpserver.exceptions import ToolError

import guru.server as server_module
from guru.server import _decide_tool_body


def test_invalid_input_raises_tool_error_not_value_error():
    with pytest.raises(ToolError, match="옵션"):
        _decide_tool_body("text", "choice", "골라라", ["only-one"], None)


def test_unexpected_predict_failure_raises_tool_error(monkeypatch):
    def boom_predict_fn_factory():
        def predict(state, questions):
            raise RuntimeError("model load failed")
        return predict

    monkeypatch.setattr(server_module, "build_router_predict_fn", boom_predict_fn_factory)

    with pytest.raises(ToolError, match="model load failed"):
        _decide_tool_body("2+2=4", "noul", "참인가?", None, None)
```

주의: 첫 번째 테스트는 검증 단계에서 `ValueError`가 발생해 `predict_fn`을 호출하기
전에 끝나므로 실제 `laya.Router`를 건드리지 않는다. 두 번째 테스트는
`build_router_predict_fn` 자체를 스텁으로 교체하므로 마찬가지로 실제 모델을
로드하지 않는다.

- [ ] **Step 2: 테스트 실행해서 실패 확인**

Run: `~/.pyenv/shims/python3 -m pytest tests/test_tool_error_handling.py -v`
Expected: FAIL — `ImportError: cannot import name '_decide_tool_body'`

- [ ] **Step 3: 구현 작성**

`src/guru/server.py` 상단에 추가:
```python
from typing import Literal

from mcp.server import MCPServer
from mcp.server.mcpserver.exceptions import ToolError
```

파일 하단에 추가:
```python
def _decide_tool_body(
    state: str,
    type: Literal["choice", "score", "noul"],
    instructions: str,
    options: list[str] | None,
    criteria: list[str] | None,
) -> DecideResult:
    try:
        return _decide_impl(state, type, instructions, options, criteria, build_router_predict_fn())
    except Exception as exc:
        raise ToolError(str(exc)) from exc


mcp = MCPServer("guru")


@mcp.tool()
def decide(
    state: str,
    type: Literal["choice", "score", "noul"],
    instructions: str,
    options: list[str] | None = None,
    criteria: list[str] | None = None,
) -> DecideResult:
    """여러 선택지 중 고르기(choice), 순서형 점수 매기기(score), 또는 참에 가까운
    확률 추정(noul)이 필요할 때 로컬 Laya 판단 모델을 단일 순전파로 호출한다.
    개방형 추론/생성 작업에는 사용하지 않는다. noul의 결과는 boolean이 아니라
    0.0~1.0 확률이다."""
    return _decide_tool_body(state, type, instructions, options, criteria)


def main() -> None:
    mcp.run()


if __name__ == "__main__":
    main()
```

- [ ] **Step 4: 테스트 실행해서 통과 확인**

Run: `~/.pyenv/shims/python3 -m pytest tests/test_tool_error_handling.py -v`
Expected: 2 passed

- [ ] **Step 5: 전체 테스트 스위트 재확인**

Run: `~/.pyenv/shims/python3 -m pytest -v`
Expected: 지금까지의 모든 테스트 통과 (Task 1+2+3 합계 10 passed)

- [ ] **Step 6: 커밋**

```bash
git add src/guru/server.py tests/test_tool_error_handling.py
git commit -m "feat: decide MCP tool 등록과 모든 예외의 ToolError 변환 추가"
```

---

### Task 4: `.mcp.json` 등록 + `guru-decide` Skill

**Files:**
- Create: `.mcp.json`
- Create: `.claude/skills/guru-decide/SKILL.md`
- Test: `tests/test_project_config.py`

**Interfaces:**
- Consumes: Task 1의 `[project.scripts] guru-mcp` 항목, Task 3의 `decide` tool 파라미터
- Produces: 없음 (Claude Code가 읽는 설정/문서)

- [ ] **Step 1: 실패하는 테스트 작성**

`tests/test_project_config.py`:
```python
import json
from pathlib import Path

ROOT = Path(__file__).parent.parent
MCP_CONFIG = ROOT / ".mcp.json"
SKILL_PATH = ROOT / ".claude" / "skills" / "guru-decide" / "SKILL.md"


def test_mcp_json_command_matches_pyproject_script_name():
    pyproject_text = (ROOT / "pyproject.toml").read_text()
    assert 'guru-mcp = "guru.server:main"' in pyproject_text

    config = json.loads(MCP_CONFIG.read_text())
    assert config["mcpServers"]["guru"]["command"] == "guru-mcp"


def test_skill_file_exists_with_frontmatter_and_mentions_tool():
    assert SKILL_PATH.exists()
    text = SKILL_PATH.read_text()
    assert text.startswith("---\n")
    frontmatter, _, body = text[4:].partition("\n---\n")
    assert "name:" in frontmatter
    assert "description:" in frontmatter
    assert "decide" in body
    assert "noul" in body
```

- [ ] **Step 2: 테스트 실행해서 실패 확인**

Run: `~/.pyenv/shims/python3 -m pytest tests/test_project_config.py -v`
Expected: FAIL — `.mcp.json` 없음

- [ ] **Step 3: `.mcp.json` 작성**

`.mcp.json`:
```json
{
  "mcpServers": {
    "guru": {
      "command": "guru-mcp",
      "args": []
    }
  }
}
```

- [ ] **Step 4: SKILL.md 작성**

`.claude/skills/guru-decide/SKILL.md`:
```markdown
---
name: guru-decide
description: Use when you need a fast, calibrated categorical/ordinal/probability-style judgment (pick one of several options, rate on an ordered scale, or estimate how likely something is) instead of open-ended reasoning. Calls the guru MCP server's decide tool, backed by the local Laya decision model.
---

# guru decide

로컬에 상주하는 Laya 판단 모델을 `decide` MCP tool로 호출해, 개방형 추론 대신
빠르고 보정된(calibrated) 판단을 받는다. 텍스트를 생성하지 않고 확률/점수/선택을
직접 반환하므로 환각이 없다.

## 언제 쓰는가

- 여러 선택지 중 하나를 골라야 할 때 (`type="choice"`)
- 순서형 척도로 점수를 매겨야 할 때 (`type="score"`)
- 참/거짓에 가까운 확률을 알고 싶을 때 (`type="noul"`)

## 언제 쓰지 않는가

- 개방형 추론, 설명, 코드 생성, 자유 형식 응답이 필요한 작업에는 쓰지 않는다.
- 판단 대상이 애매하거나 선택지/기준을 명확히 정의할 수 없다면 쓰지 않는다.

## 사용법

`decide` tool 파라미터:
- `state`: 판단 대상 내용(문장/이메일/티켓 등)
- `instructions`: 고정 판단 기준(질문)
- `type`: `"choice"` | `"score"` | `"noul"`
- `options`: `type="choice"`일 때 선택지 목록(최소 2개)
- `criteria`: `type="score"`일 때 등급을 낮은 순서대로 나열한 목록(최소 2개)

예)
- `decide(state="서버가 다운됐습니다", instructions="이 요청이 긴급한가?", type="noul")`
- `decide(state="청구서가 중복 결제됐습니다", instructions="어느 팀이 처리해야 하는가?", type="choice", options=["billing","technical","sales"])`
- `decide(state="이미 세 번째 문의입니다", instructions="고객의 불만 강도는?", type="score", criteria=["낮음","보통","높음"])`

결과는 `{"answer": ..., "routing": ..., "latency_ms": ...}` 형태다.

- `noul`의 `answer.noul`은 boolean이 아니라 0.0~1.0 확률이다. 필요하면 대화
  맥락에서 직접 임계값(예: 0.5)을 적용해라.
- `choice`의 `answer.choice`가 선택된 라벨, `answer.confidence`가 신뢰도다.
- `score`의 `answer.score`가 기대값(0부터 등급 개수-1 사이)이다.

## 실패 시

tool 호출이 에러로 돌아오면(모델 로드 실패, 입력 검증 실패 등) 작업을 막지 말고
평소처럼 직접 추론해서 계속 진행해라 — guru는 있으면 좋은 가속 수단이지, 필수
의존성이 아니다.
```

- [ ] **Step 5: 테스트 실행해서 통과 확인**

Run: `~/.pyenv/shims/python3 -m pytest tests/test_project_config.py -v`
Expected: 2 passed

- [ ] **Step 6: 커밋**

```bash
git add .mcp.json .claude/skills/guru-decide/SKILL.md tests/test_project_config.py
git commit -m "feat: guru MCP 서버 등록과 guru-decide Skill 추가"
```

---

### Task 5: 엔드투엔드 스모크 테스트 (실제 모델, 실제 stdio subprocess)

**Files:**
- Create: `test_smoke.py`
- Test: (이 태스크 자체가 테스트)

**Interfaces:**
- Consumes: `guru.server`를 `python -m guru.server`로 실행한 실제 stdio MCP 서버
- Produces: 없음 (검증 스크립트)

이 태스크는 스펙의 성공 기준을 실제 가중치·실제 MCP 프로토콜로 검증하는 마지막
단계다. 네트워크가 필요하고(최초 실행 시 가중치 다운로드) 최초 로드에 몇 분이
걸릴 수 있으므로, 평소 `pytest` 스위트에는 포함하지 않고 별도로 수동 실행한다.

- [ ] **Step 1: 스모크 테스트 스크립트 작성**

`test_smoke.py`:
```python
"""guru decide MCP 서버 엔드투엔드 스모크 테스트. 실제 Laya 가중치를
다운로드/로드하므로 네트워크가 필요하고 최초 실행은 수 분이 걸릴 수 있다.
수동 실행:
    python test_smoke.py
"""
import sys

import anyio
from mcp import Client
from mcp.server import StdioServerParameters


async def run() -> None:
    params = StdioServerParameters(command=sys.executable, args=["-m", "guru.server"])
    async with Client(params) as client:
        result = await client.call_tool(
            "decide",
            {
                "state": "2+2=4",
                "type": "noul",
                "instructions": "이 문장이 수학적으로 참인가?",
            },
        )
        assert not result.is_error, result.content
        body = result.structured_content
        noul_prob = body["answer"]["noul"]
        assert 0.0 <= noul_prob <= 1.0, f"확률 범위 밖: {noul_prob}"
        assert noul_prob > 0.5, f"참인 문장인데 확률이 낮음: {noul_prob}"
        print(f"OK: noul={noul_prob:.3f}, latency_ms={body['latency_ms']:.1f}")


if __name__ == "__main__":
    anyio.run(run)
```

- [ ] **Step 2: 실행해서 확인**

Run: `~/.pyenv/shims/python3 test_smoke.py`
Expected: `OK: noul=0.9xx, latency_ms=...` 출력 후 exit 0
(최초 실행은 가중치 다운로드로 수 분 소요 가능. 네트워크 없으면 subprocess가
죽거나 타임아웃되며 stderr에 원인이 찍힌다)

**주의**: `result.structured_content`가 `DecideResult`의 필드를 최상위에 그대로
담는지, 아니면 다른 키로 감싸는지는 `mcp` SDK 문서에서 스칼라 반환 예시
(`{'result': 3}`)만 확인했고 BaseModel 반환의 실제 모양은 확인하지 못했다.
`body["answer"]["noul"]`에서 `KeyError`가 나면, 먼저
`print(result.structured_content)`로 실제 모양을 찍어보고 그에 맞게 접근 경로를
고쳐라 — 값을 추측해서 코드를 더 쌓지 말 것.

- [ ] **Step 3: 커밋**

```bash
git add test_smoke.py
git commit -m "test: guru decide MCP 서버 엔드투엔드 스모크 테스트 추가"
```

---

## 이 플랜 완료 후 수동 확인 사항

`.mcp.json`을 이 세션이 인식하려면 **Claude Code를 재시작(또는 새 세션 시작)**해야
한다 — 실행 중인 세션은 이미 스폰된 MCP 서버 목록을 다시 읽지 않는다. 재시작 후
`decide` tool이 도구 목록에 나타나는지 확인해라.
