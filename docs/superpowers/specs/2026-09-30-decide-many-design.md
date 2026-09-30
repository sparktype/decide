# decide_many: 한 state에 질문 여러 개를 한 번에

에이전트 하니스에 Jev를 넣는 영상(Sam Witteveen, Using Jev In Your Agent Harness)의
정리 노트(Obsidian `Personal/자기개발/Jev 에이전트 하니스 활용 영상 요약`. 같은 내용의
`docs/jev-agent-harness-video.md`는 아직 `main`에 없고 로컬 브랜치 커밋 `c69f53f`에만 있다)에서
"한 state에 질문 여러 개를 던져도 응답 시간은 거의 늘지 않는다"를 `decide`에 넣는다.
이 문서는 그 첫 번째 기능이다. 나머지 기능과 순서는 아래 로드맵에 있다.

## 로드맵

영상에서 고른 기능은 독립된 부분이 섞여 있어 기능마다 설계, 계획, 구현을 따로 돈다.

| 순서 | 기능 | 의존 |
| --- | --- | --- |
| 1 | 다중 질문 `decide_many` (이 문서) | 없음 |
| 2 | 인젝션 선검사 | 1 |
| 3 | PreToolUse 위험 게이트 훅 | 없음. 1이 있으면 "위험한가"와 "인젝션인가"를 한 번에 물을 수 있다 |
| 4 | 스킬 캐스케이드 선택. 힌트 모드부터 | 없음. 기존 `decide` 데몬만 쓴다 |
| 5 | RAG 리랭킹 | 1. 추가 가치가 불분명해 보류 |

## 결정

- 새 MCP 도구 `decide_many`를 하나 더 둔다. 기존 `decide`는 바뀌지 않는다. 질문 id `"q"`와
  응답 `answer`를 그대로 쓰고, Stop 훅과 기존 호출자는 영향이 없다.
- 전부 성공하거나 전부 실패한다. 부분 결과는 없다.
- 질문 개수 상한은 정하지 않고 백엔드가 거절하는 대로 돌려준다.

## 인터페이스

인자는 `state`(문자열)와 `questions`(id → 질문 객체의 맵)다. 질문 객체는 기존 `decide`의
`type`, `instructions`, `options`, `criteria`와 같은 뜻이다. 이 맵 모양은 TypeSafe와 로컬
서버 요청 본문의 `questions`와 같아서 그대로 전달한다.

```text
decide_many(
  state="서버가 다운됐습니다. 결제 API가 500을 반환합니다.",
  questions={
    "urgent":  {type: "noul",   instructions: "이 요청이 긴급한가?"},
    "team":    {type: "choice", instructions: "어느 팀이 처리해야 하는가?", options: ["billing", "infra", "sales"]},
    "anger":   {type: "score",  instructions: "고객의 불만 강도는?", criteria: ["낮음", "보통", "높음"]}
  }
)
```

응답은 `{answers: {id: 답}, routing, latency_ms}`다. `answers`는 입력 순서를 유지한다(이
저장소는 `serde_json`의 `preserve_order`를 켜 두었다). 각 답의 모양은 백엔드가 낸 그대로다.
`routing.backend`는 `typesafe` 또는 `local`이고 `latency_ms`는 그 한 번의 호출만 잰다.

## 검증과 오류

- `questions`는 비어 있지 않은 객체여야 한다. 비어 있으면 `questions는 비어 있을 수 없습니다`.
- id는 JSON 객체의 키라 중복이 없고, 빈 문자열은 거절한다.
- 각 질문의 검증은 기존 `protocol::validate`를 재사용한다. 기존 한글 문장을 그대로 쓰고
  앞에 질문 id를 붙인다. 예: `질문 "urgent": noul 타입은 options/criteria를 받지 않습니다`.
- 검증은 호출 전에 모든 질문에 대해 끝낸다. 하나라도 틀리면 첫 오류 하나로 호출 전체를
  거절하고 백엔드를 부르지 않는다.
- TypeSafe 한도(choice 255, score 10)는 질문마다 검사한다. 넘으면 질문 id를 붙여 호출 전에
  거절한다. 로컬은 한도를 검사하지 않고 서버가 거절하는 대로 돌려준다.
- 백엔드 오류, 연결 실패, 재시도 규칙은 `decide`와 같다. 429와 529는 1초 뒤 한 번 재시도한다.
- 응답에서 요청한 id가 하나라도 빠져 있으면 `{label} 응답에 answers.<id>가 없습니다` 오류로
  호출 전체를 실패시킨다. 응답에 요청하지 않은 id가 더 있으면 무시한다.

## 구성 변경

- `protocol.rs`: `parse_many`가 `state`와 `questions` 맵을 읽어 `(id, Question)` 목록을
  입력 순서대로 만든다. 질문 하나의 검증은 기존 `validate`를 부른다.
- `typesafe.rs`: `request_body`가 질문 하나를 만드는 부분을 `question_json`으로 분리한다.
  `request_body_many`가 `questions` 맵 전체를 만든다. 기존 `request_body`는 id `"q"` 하나로
  이를 호출해 결과가 바이트 단위로 같다. `map_answers(body, ids, label)`는 요청한 id의 답을
  모아 `(answers, model)`을 돌려준다.
- `backend.rs`: `decide_many`가 `decide`와 같은 흐름(백엔드 선택, 한도, 실행, 매핑, 지연)을
  탄다. 공통 부분은 겹치는 만큼만 묶고, `decide`의 동작은 바꾸지 않는다.
- `mcp.rs`: `tools/list`에 `decide_many`를 더하고 `tools/call`이 이름으로 분기한다. 도구
  설명에 "질문 하나하나는 yes/no나 단일 선택처럼 작게 쪼갠다. 복합 질문은 정확도가 떨어진다"를
  적는다.
- `daemon.rs`: 소켓 줄에 `questions` 키가 있으면 `decide_many`와 같은 모양으로 답한다.
  `type`과 `questions`를 함께 주면 `type과 questions는 함께 쓸 수 없습니다` 오류다. 기존
  줄 형식은 바뀌지 않는다. 캐시 키는 `(백엔드, state, questions 전체)`이고 기존 키와
  섞이지 않게 별도 변형으로 둔다. 캐시 적중 시 `routing.cached: true`, `latency_ms: 0.0`이다.
- README: `decide_many` 사용법과 "질문을 작게 쪼갠다" 지침, CHANGELOG `[Unreleased]`.

## 테스트

기본 `cargo test`는 가중치, 네트워크, API 키 없이 돈다. 스크립트 전송으로 검사한다.

- 본문: 여러 질문이 `questions` 맵에 입력 순서대로 들어가고, 질문 하나일 때 `request_body`가
  기존과 바이트 단위로 같다.
- 검증: 잘못된 질문 하나가 id를 붙인 기존 한글 문장으로 호출 전체를 거절하고 전송이 0회다.
  빈 `questions`, 빈 id, 모든 타입의 조합을 검사한다.
- 한도: TypeSafe에서 질문 id를 붙여 호출 전에 막고, 로컬에서는 호출까지 간다.
- 응답: 모든 id가 있으면 입력 순서로 매핑하고, 하나라도 빠지면 전체 실패, 여분 id는 무시한다.
- 전부 성공/전부 실패: 백엔드 오류가 부분 결과 없이 도구 오류로 끝난다.
- MCP: `tools/list`에 두 도구가 있고 `decide`의 스키마와 응답이 그대로다. `decide_many`의
  `structuredContent`가 `answers`를 가진다.
- 데몬: `questions` 줄이 같은 모양으로 답하고, `type`과 `questions` 동시 지정이 오류이며,
  동일 요청이 캐시에 적중한다. 기존 줄 형식 테스트는 수정 없이 통과한다.
- 실서버: 로컬 서버로 한글 다중 질문을 한 번 돌려 `answers`가 모두 오는지 본다.

## 범위 밖

- 질문 사이 의존(앞 답에 따라 뒤 질문을 정하는 것). 그런 흐름은 호출을 나눠서 한다.
- 질문 개수 상한과 부분 성공.
- `decide`의 인자나 응답 모양 변경.
- 인젝션 선검사, 위험 게이트, 스킬 캐스케이드, 리랭킹은 로드맵의 다른 항목이다.

## 알려진 위험

- TypeSafe가 한 요청에서 받는 질문 개수 상한과 다중 질문의 지연 증가는 확인하지 못했다.
  TypeSafe 실호출은 API 키가 필요해 사용자가 직접 확인해야 한다.
- 로컬 서버의 다중 질문 동작은 이 문서를 쓰는 시점에 실호출로 확인하지 않았다. 카드는
  `score_many`를 설명하고, API는 `questions` 맵을 받는다는 점만 확인했다. 구현 중 실서버
  테스트로 확인한다.
- 다중 질문의 정확도는 질문 하나씩 던질 때와 같다고 보장할 수 없다. 같은 state를 공유하는
  질문이 서로 영향을 주는지는 재 보지 않았다.
