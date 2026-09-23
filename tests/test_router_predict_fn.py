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
