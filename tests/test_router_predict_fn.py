# laya Router 지연 싱글톤(1회 로드, 재사용, 스레드 안전성) 테스트
import threading
import time

import decide.server as server_module


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


def test_build_router_predict_fn_is_thread_safe(monkeypatch):
    created = []

    class SlowFakeRouter:
        def __init__(self, preload):
            time.sleep(0.05)
            created.append(preload)

        def predict(self, state, questions):
            return {"answers": {"q": {"noul": 0.5}}, "routing": {}}

    monkeypatch.setattr(server_module, "_router_singleton", None)
    monkeypatch.setattr(server_module.laya, "Router", SlowFakeRouter)

    predict_fn = server_module.build_router_predict_fn()
    threads = [
        threading.Thread(
            target=predict_fn, args=(f"state {i}", {"q": {"type": "noul", "instructions": "?"}})
        )
        for i in range(5)
    ]
    for t in threads:
        t.start()
    for t in threads:
        t.join()

    assert created == [True]
