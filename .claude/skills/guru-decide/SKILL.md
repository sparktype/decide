---
name: guru-decide
description: Use when you need a fast, calibrated categorical/ordinal/probability-style judgment (pick one of several options, rate on an ordered scale, or estimate how likely something is) instead of open-ended reasoning. Calls the guru MCP server's decide tool, backed by the local Laya decision model.
---

# guru decide

로컬에 상주하는 Laya 판단 모델을 `decide` MCP tool로 호출해, 개방형 추론 대신
빠른 판단을 받는다. 텍스트를 생성하지 않고 확률/점수/선택을 직접 반환하므로
JSON 파싱 실패나 형식 이탈은 없다 — 다만 판단 자체의 정확도는 질문 문구에 따라
크게 달라질 수 있다(아래 "주의" 참고). "환각이 없다"는 형식에 대한 것이지 정확도를
보장한다는 뜻이 아니다.

## 언제 쓰는가

- 여러 선택지 중 하나를 골라야 할 때 (`type="choice"`)
- 순서형 척도로 점수를 매겨야 할 때 (`type="score"`)
- 참/거짓에 가까운 확률을 알고 싶을 때 (`type="noul"`)

## 언제 쓰지 않는가

- 개방형 추론, 설명, 코드 생성, 자유 형식 응답이 필요한 작업에는 쓰지 않는다.
- 판단 대상이 애매하거나 선택지/기준을 명확히 정의할 수 없다면 쓰지 않는다.

## 사용법

`decide` tool 파라미터:
- `state`: 판단 대상 내용(문장/이메일/티켓 등)
- `instructions`: 고정 판단 기준(질문)
- `type`: `"choice"` | `"score"` | `"noul"`
- `options`: `type="choice"`일 때 선택지 목록(최소 2개)
- `criteria`: `type="score"`일 때 등급을 낮은 순서대로 나열한 목록(최소 2개)

예)
- `decide(state="서버가 다운됐습니다", instructions="이 요청이 긴급한가?", type="noul")`
- `decide(state="청구서가 중복 결제됐습니다", instructions="어느 팀이 처리해야 하는가?", type="choice", options=["billing","technical","sales"])`
- `decide(state="이미 세 번째 문의입니다", instructions="고객의 불만 강도는?", type="score", criteria=["낮음","보통","높음"])`

결과는 `{"answer": ..., "routing": ..., "latency_ms": ...}` 형태다.

- `noul`의 `answer.noul`은 boolean이 아니라 0.0~1.0 확률이다. 필요하면 대화
  맥락에서 직접 임계값(예: 0.5)을 적용해라.
- `choice`의 `answer.choice`가 선택된 라벨, `answer.confidence`가 신뢰도다.
- `score`의 `answer.score`가 기대값(0부터 등급 개수-1 사이)이다.

## 주의: noul은 질문 문구에 민감하다

`instructions`를 "Is this a positive review?"처럼 반문형으로 쓰면 판단 방향이
무너지는 것을 확인했다. "Does the customer express satisfaction?"처럼 상태를
직접 서술적으로 묻는 문구가 더 안정적이다. `noul` 결과를 신뢰해 자동으로 행동을
분기하기 전에, 애매한 케이스라면 대조되는 예시로 방향이 실제로 뒤집히는지
한 번 확인해라. 참/거짓이 명확히 갈리는 판단이 중요하다면 `noul` 대신 명시적인
`choice`(예: `options=["positive","negative"]`)가 더 안정적이다.

## 실패 시

tool 호출이 에러로 돌아오면(모델 로드 실패, 입력 검증 실패 등) 작업을 막지 말고
평소처럼 직접 추론해서 계속 진행해라 — guru는 있으면 좋은 가속 수단이지, 필수
의존성이 아니다.
