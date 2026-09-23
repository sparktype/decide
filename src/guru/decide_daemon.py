# Router를 상주 프로세스에 1회 로드하고 유닉스 소켓으로 decide 요청을 처리하는 데몬
import json
import socket
import sys
from pathlib import Path

from guru.server import DecideResult, _decide_impl, build_router_predict_fn

SOCKET_PATH = Path.home() / ".cache" / "guru" / "decide.sock"
IDLE_TIMEOUT_S = 30 * 60  # ponytail: 30분 무요청 시 자동 종료, 필요하면 조정


def _handle(conn: socket.socket, predict_fn) -> None:
    with conn:
        line = conn.makefile("rb").readline()
        if not line:
            return
        try:
            req = json.loads(line)
            result: DecideResult = _decide_impl(
                state=req["state"],
                type=req["type"],
                instructions=req["instructions"],
                options=req.get("options"),
                criteria=req.get("criteria"),
                predict_fn=predict_fn,
            )
            resp = result.model_dump()
        except Exception as exc:
            resp = {"error": str(exc)}
        conn.sendall((json.dumps(resp) + "\n").encode())


def _claim_socket() -> bool:
    """이미 떠 있는 데몬이 있으면 False, 없으면 소켓을 선점하고 True."""
    SOCKET_PATH.parent.mkdir(parents=True, exist_ok=True)
    if not SOCKET_PATH.exists():
        return True
    probe = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
    probe.settimeout(0.5)
    try:
        probe.connect(str(SOCKET_PATH))
        probe.close()
        return False  # 응답하는 데몬이 이미 있다
    except OSError:
        SOCKET_PATH.unlink()  # 죽은 소켓 파일만 남은 상태
        return True


def main() -> None:
    if not _claim_socket():
        print(f"daemon already running at {SOCKET_PATH}", file=sys.stderr)
        return

    predict_fn = build_router_predict_fn()
    predict_fn("워밍업", {"q": {"type": "noul", "instructions": "참인가?"}})  # 최초 로드를 여기서 끝낸다

    server = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
    server.bind(str(SOCKET_PATH))
    server.listen(5)
    server.settimeout(IDLE_TIMEOUT_S)

    try:
        while True:
            try:
                conn, _ = server.accept()
            except socket.timeout:
                break  # 유휴 시간 초과 -> 종료
            _handle(conn, predict_fn)
    finally:
        server.close()
        SOCKET_PATH.unlink(missing_ok=True)


if __name__ == "__main__":
    main()
