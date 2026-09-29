# .mcp.json 등록과 decide SKILL.md 존재/내용 테스트
import json
import os
from pathlib import Path

ROOT = Path(__file__).parent.parent
MCP_CONFIG = ROOT / ".mcp.json"
SKILL_PATH = ROOT / ".claude" / "skills" / "decide" / "SKILL.md"


def test_mcp_json_command_resolves_to_this_repos_venv():
    pyproject_text = (ROOT / "pyproject.toml").read_text()
    assert 'decide-mcp = "decide.server:main"' in pyproject_text

    config = json.loads(MCP_CONFIG.read_text())
    command = config["mcpServers"]["decide"]["command"]

    command_path = Path(command)
    assert command_path.is_absolute(), "상대/PATH 의존 커맨드는 다른 설치를 잘못 가리킬 수 있다"
    assert str(command_path).startswith(str(ROOT / ".venv")), (
        "이 저장소의 .venv 밖을 가리키면 공유 환경의 오염된 설치를 부팅할 위험이 있다"
    )
    assert command_path.exists(), f"{command_path}가 존재하지 않는다"
    assert os.access(command_path, os.X_OK), f"{command_path}가 실행 가능하지 않다"


def test_skill_file_exists_with_frontmatter_and_mentions_tool():
    assert SKILL_PATH.exists()
    text = SKILL_PATH.read_text()
    assert text.startswith("---\n")
    frontmatter, _, body = text[4:].partition("\n---\n")
    assert "name:" in frontmatter
    assert "description:" in frontmatter
    assert "decide" in body
    assert "noul" in body
