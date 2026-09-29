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


def _commands(node):
    found = []
    if isinstance(node, dict):
        if isinstance(node.get("command"), str):
            found.append(node["command"])
        for value in node.values():
            found.extend(_commands(value))
    elif isinstance(node, list):
        for value in node:
            found.extend(_commands(value))
    return found


def test_claude_stop_hook_runs_stop_verify_and_commands_exist():
    import json

    root = MODULE_PATH.parents[2]
    settings = json.loads((root / ".claude" / "settings.json").read_text())
    commands = _commands(settings["hooks"]["Stop"])
    assert any(command.endswith("stop_verify.py\"") or "stop_verify.py" in command for command in commands)
    for command in _commands(settings):
        for token in command.split():
            if token.startswith("\"") and token.endswith("\"") and "/" in token:
                path = Path(token.strip("\"").replace("${CLAUDE_PROJECT_DIR:-.}", str(root)))
                if path.suffix in {".py", ".cjs"}:
                    assert path.is_file(), path


def test_cursor_hooks_point_at_this_repo():
    import json

    root = MODULE_PATH.parents[2]
    config = json.loads((root / ".cursor" / "hooks.json").read_text())
    commands = _commands(config)
    assert commands
    for command in commands:
        assert "/Users/spark/Develop/Workspaces/decide-mcp/.cursor/hooks/graft-hooks.cjs" in command
        assert Path("/Users/spark/Develop/Workspaces/decide-mcp/.cursor/hooks/graft-hooks.cjs").is_file()
