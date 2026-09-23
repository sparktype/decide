"""guru decide MCP 서버 엔드투엔드 스모크 테스트. 실제 Laya 가중치를
다운로드/로드하므로 네트워크가 필요하고 최초 실행은 수 분이 걸릴 수 있다.
수동 실행:
    python test_smoke.py
"""
import sys

import anyio
from mcp import Client, StdioServerParameters


async def run() -> None:
    params = StdioServerParameters(command=sys.executable, args=["-m", "guru.server"])
    async with Client(params) as client:
        result = await client.call_tool(
            "decide",
            {
                "state": "This was absolutely the best service I've ever had. Highly recommend!",
                "type": "noul",
                "instructions": "Is this a positive review?",
            },
        )
        assert not result.is_error, result.content
        body = result.structured_content
        noul_prob = body["answer"]["noul"]
        assert 0.0 <= noul_prob <= 1.0, f"확률 범위 밖: {noul_prob}"
        assert isinstance(body["latency_ms"], (int, float))
        # 이 테스트는 배관(MCP stdio 왕복, 응답 shape)만 검증한다. 모델의 판단
        # 방향(맞았는지)은 검증하지 않는다 — 다운로드된 체크포인트가 confidence
        # 보정 문제를 안고 있다고 laya 자체가 런타임 경고로 밝히기 때문이다.
        print(f"OK: noul={noul_prob:.3f}, latency_ms={body['latency_ms']:.1f}")


if __name__ == "__main__":
    anyio.run(run)
