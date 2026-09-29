# 로컬 laya 판단 모델을 choice/score/noul MCP tool로 노출하는 stdio 서버
import threading
import time
from typing import Callable, Literal

import laya
from mcp.server import MCPServer
from mcp.server.mcpserver.exceptions import ToolError
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
        if not options or len(set(options)) < 2:
            raise ValueError("choice 타입은 서로 다른 옵션이 최소 2개 필요합니다")
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


_router_singleton = None
_router_lock = threading.Lock()


def build_router_predict_fn() -> PredictFn:
    def predict(state: str, questions: dict) -> dict:
        global _router_singleton
        if _router_singleton is None:
            with _router_lock:
                if _router_singleton is None:
                    _router_singleton = laya.Router(preload=True)
        return _router_singleton.predict(state, questions)

    return predict


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


mcp = MCPServer("decide")


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
