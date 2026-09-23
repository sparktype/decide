# decide_daemon._handle의 소켓 프로토콜(JSON 요청 처리, 예외를 error 키로 감싸기) 테스트
import json
import socket

from guru.decide_daemon import _handle


def test_handle_returns_noul_answer_over_socket():
    client, server = socket.socketpair()

    def stub_predict(state, questions):
        return {"answers": {"q": {"noul": 0.8}}, "routing": {}}

    client.sendall(
        json.dumps({"state": "some diff", "type": "noul", "instructions": "검증됐는가"}).encode() + b"\n"
    )
    client.shutdown(socket.SHUT_WR)

    _handle(server, stub_predict)

    resp = json.loads(client.makefile("rb").readline())
    assert resp["answer"] == {"noul": 0.8}


def test_handle_wraps_exceptions_as_error():
    client, server = socket.socketpair()

    def failing_predict(state, questions):
        raise RuntimeError("boom")

    client.sendall(json.dumps({"state": "x", "type": "noul", "instructions": "?"}).encode() + b"\n")
    client.shutdown(socket.SHUT_WR)

    _handle(server, failing_predict)

    resp = json.loads(client.makefile("rb").readline())
    assert "error" in resp
