# Stop 훅(stop_verify.py)의 순수 로직(테스트 흔적 판별, 차단 임계값) 테스트
import importlib.util
from pathlib import Path

MODULE_PATH = Path(__file__).resolve().parents[1] / ".claude" / "hooks" / "stop_verify.py"
spec = importlib.util.spec_from_file_location("stop_verify", MODULE_PATH)
stop_verify = importlib.util.module_from_spec(spec)
spec.loader.exec_module(stop_verify)


def test_mentions_test_run_detects_known_markers():
    assert stop_verify.mentions_test_run("ran: pytest -q tests/")
    assert stop_verify.mentions_test_run("$ npm test")
    assert not stop_verify.mentions_test_run("just editing files, no test command")


def test_should_block_below_threshold_only():
    assert stop_verify.should_block(0.0)
    assert stop_verify.should_block(stop_verify.THRESHOLD - 0.01)
    assert not stop_verify.should_block(stop_verify.THRESHOLD)
    assert not stop_verify.should_block(1.0)
