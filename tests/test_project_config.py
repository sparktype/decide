# pyproject 스크립트와 decide SKILL.md 존재/내용 테스트
from pathlib import Path

ROOT = Path(__file__).parent.parent
SKILL_PATH = ROOT / ".claude" / "skills" / "decide" / "SKILL.md"


def test_pyproject_exposes_decide_mcp_script():
    pyproject_text = (ROOT / "pyproject.toml").read_text()
    assert 'decide-mcp = "decide.server:main"' in pyproject_text


def test_skill_file_exists_with_frontmatter_and_mentions_tool():
    assert SKILL_PATH.exists()
    text = SKILL_PATH.read_text()
    assert text.startswith("---\n")
    frontmatter, _, body = text[4:].partition("\n---\n")
    assert "name:" in frontmatter
    assert "description:" in frontmatter
    assert "decide" in body
    assert "noul" in body
