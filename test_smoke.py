"""guru decide MCP 서버 엔드투엔드 스모크 테스트. 실제 Laya 가중치를
다운로드/로드하므로 네트워크가 필요하고 최초 실행은 수 분이 걸릴 수 있다.
수동 실행:
    python test_smoke.py
"""
import sys

import anyio
from mcp import Client, StdioServerParameters


async def run() -> None:
    # noul은 질문 문구에 민감하다 — "Is this a positive review?"처럼 반문형으로
    # 물으면 방향이 무너지는 것을 확인했다. "Does the customer express
    # satisfaction?"처럼 상태를 직접 묻는 문구는 안정적으로 방향을 구분한다.
    # (체크포인트 confidence 보정 문제가 아니라 문구 민감성이 원인이었다.)
    params = StdioServerParameters(command=sys.executable, args=["-m", "guru.server"])
    async with Client(params) as client:
        positive = await client.call_tool(
            "decide",
            {
                "state": "This was absolutely the best service I've ever had. Highly recommend!",
                "type": "noul",
                "instructions": "Does the customer express satisfaction?",
            },
        )
        negative = await client.call_tool(
            "decide",
            {
                "state": "This was the worst experience. Never coming back.",
                "type": "noul",
                "instructions": "Does the customer express satisfaction?",
            },
        )
        assert not positive.is_error, positive.content
        assert not negative.is_error, negative.content

        pos_prob = positive.structured_content["answer"]["noul"]
        neg_prob = negative.structured_content["answer"]["noul"]
        assert 0.0 <= pos_prob <= 1.0, f"확률 범위 밖: {pos_prob}"
        assert 0.0 <= neg_prob <= 1.0, f"확률 범위 밖: {neg_prob}"
        assert pos_prob > neg_prob, f"긍정/부정 구분 실패: pos={pos_prob}, neg={neg_prob}"
        print(f"OK: positive={pos_prob:.3f}, negative={neg_prob:.3f}")


if __name__ == "__main__":
    anyio.run(run)
