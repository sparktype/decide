# 입력 검증 실패와 예상 못한 예외가 모두 ToolError로 변환되는지 테스트
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
