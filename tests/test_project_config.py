import json
from pathlib import Path

ROOT = Path(__file__).parent.parent
MCP_CONFIG = ROOT / ".mcp.json"
SKILL_PATH = ROOT / ".claude" / "skills" / "guru-decide" / "SKILL.md"


def test_mcp_json_command_matches_pyproject_script_name():
    pyproject_text = (ROOT / "pyproject.toml").read_text()
    assert 'guru-mcp = "guru.server:main"' in pyproject_text

    config = json.loads(MCP_CONFIG.read_text())
    assert config["mcpServers"]["guru"]["command"] == "guru-mcp"


def test_skill_file_exists_with_frontmatter_and_mentions_tool():
    assert SKILL_PATH.exists()
    text = SKILL_PATH.read_text()
    assert text.startswith("---\n")
    frontmatter, _, body = text[4:].partition("\n---\n")
    assert "name:" in frontmatter
    assert "description:" in frontmatter
    assert "decide" in body
    assert "noul" in body
