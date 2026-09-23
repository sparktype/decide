import pytest

from guru.server import DecideResult, _decide_impl


def test_noul_builds_question_without_criteria():
    captured = {}

    def recording(state, questions):
        captured["questions"] = questions
        return {"answers": {"q": {"noul": 0.9}}, "routing": {}}

    result = _decide_impl("2+2=4", "noul", "참인가?", None, None, recording)

    assert captured["questions"] == {"q": {"type": "noul", "instructions": "참인가?"}}
    assert isinstance(result, DecideResult)
    assert result.answer == {"noul": 0.9}
    assert result.routing == {}
    assert result.latency_ms >= 0


def test_choice_converts_options_to_criteria_dict():
    captured = {}

    def recording(state, questions):
        captured["questions"] = questions
        return {"answers": {"q": {"choice": "a", "confidence": 0.5}}, "routing": {}}

    _decide_impl("text", "choice", "골라라", ["a", "b", "c"], None, recording)

    assert captured["questions"]["q"]["criteria"] == {"a": "a", "b": "b", "c": "c"}


def test_score_passes_criteria_list_in_order():
    captured = {}

    def recording(state, questions):
        captured["questions"] = questions
        return {"answers": {"q": {"score": 1.0, "confidence": 0.5}}, "routing": {}}

    _decide_impl("text", "score", "평가해라", None, ["low", "medium", "high"], recording)

    assert captured["questions"]["q"]["criteria"] == ["low", "medium", "high"]


def test_choice_requires_at_least_two_options():
    with pytest.raises(ValueError, match="옵션"):
        _decide_impl("text", "choice", "골라라", ["only-one"], None, lambda s, q: {})


def test_choice_requires_options_present():
    with pytest.raises(ValueError, match="옵션"):
        _decide_impl("text", "choice", "골라라", None, None, lambda s, q: {})


def test_score_requires_at_least_two_criteria():
    with pytest.raises(ValueError, match="등급"):
        _decide_impl("text", "score", "평가해라", None, ["only-one"], lambda s, q: {})


def test_noul_rejects_options_or_criteria():
    with pytest.raises(ValueError, match="noul"):
        _decide_impl("text", "noul", "참인가?", ["a", "b"], None, lambda s, q: {})
