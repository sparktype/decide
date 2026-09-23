import time
from typing import Callable, Literal

import laya
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


_router_singleton = None


def build_router_predict_fn() -> PredictFn:
    def predict(state: str, questions: dict) -> dict:
        global _router_singleton
        if _router_singleton is None:
            _router_singleton = laya.Router(preload=True)
        return _router_singleton.predict(state, questions)

    return predict
