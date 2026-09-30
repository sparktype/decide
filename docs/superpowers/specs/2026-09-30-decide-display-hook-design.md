# decide가 고른 것을 사용자에게 보여주기: `decide hook`

에이전트가 `decide` 도구를 부르면 지금은 결과가 JSON 그대로 도구 결과 안에만 있어, 사용자가
"decide가 무엇을 골랐는지"를 눈으로 보기 어렵다. Claude Code `PostToolUse` 훅으로 한 줄 요약을
`systemMessage`로 띄운다. `systemMessage`는 사용자에게만 보이고 모델은 보지 않는다.

## 결정

- 훅 명령은 `decide` 바이너리의 새 서브커맨드 `decide hook`이다(Rust). stdin으로 훅 입력 JSON을
  읽고 stdout에 `{"systemMessage": "..."}` 한 줄을 쓴다.
- 이유: `claude mcp add -s user`로 등록한 `decide`는 프로젝트와 무관하게 쓰이므로 훅도 같은 바이너리로
  어디서나 동작해야 한다. Python 스크립트는 저장소 안에서만 돈다. 서식 로직은 `cargo test`로 기록된
  JSON을 넣어 검증할 수 있다.
- 대상 도구는 `mcp__decide__decide`다. `decide_many`는 머지·릴리스되면 같은 훅에 추가한다(이번
  범위 밖).
- 훅은 절대 실패하지 않는다. 입력을 해석하지 못하면 아무것도 쓰지 않고 종료 코드 0이다(fail-open).

## 출력 형식

질문은 `tool_input.instructions`에서, 확률과 백엔드는 `tool_response`에서 얻는다. 확률은 정수 %로
반올림한다. 질문이 80자를 넘으면 `…`로 자른다.

```text
🔎 decide가 선택했습니다: "푸시하고 PR 생성" (96%)
   질문: 이 브랜치를 마무리하는 가장 적절한 방법은?
   나머지: 브랜치를 그대로 유지 3% · main에 로컬로 머지 1% · 작업을 폐기 0%
   typesafe · jev-1.13.0 · 203ms

🔎 decide 판단: 이 변경은 머지해도 될 만큼 검증되었는가? → 56%
   typesafe · jev-1.13.0 · 460ms

🔎 decide 점수: 남아 있는 위험의 크기는? → 기대값 1.17/4, 가장 가능성 높은 등급 "낮음" 83%
   typesafe · jev-1.13.0 · 181ms
```

- choice: `decide가 선택했습니다: "<라벨>" (<확률>%)`. 나머지는 확률 내림차순(최대 3개).
  조사를 쓰지 않는 문장이라 받침을 판별하지 않는다.
- noul: 질문과 확률만. 임계값 판단은 하지 않는다(확률을 그대로 보여준다).
- score: 기대값 `score/(등급 수-1)`과 확률이 가장 높은 등급 이름(`legend`)과 그 확률.
- `routing.cached`가 `true`면 마지막 줄에 `(캐시)`를 붙이고 지연은 생략한다.

## 입력 처리

훅 입력 JSON에서 `tool_name`이 `mcp__decide__decide`가 아니면 아무것도 쓰지 않는다. `tool_response`는
다음 형태를 모두 받는다. 문서는 MCP 도구 결과가 텍스트로 온다고 하지만 실제 세션에서 확인하지 않았기
때문이다.

1. 문자열: JSON으로 파싱해 `{answer, routing, latency_ms}`로 읽는다.
2. 객체 중 `structuredContent`가 있으면 그것을 쓴다.
3. 객체 중 `content[0].text`가 있으면 그 문자열을 JSON으로 파싱한다. `structuredContent`에 `answer`가
   없으면 이 경로로 폴백한다.
4. 객체 자체에 `answer`가 있으면 그 객체가 결과다.
5. 배열이면 `content` 배열로 보고 `[0].text`를 JSON으로 파싱한다.
6. 그 밖이거나 파싱에 실패하거나 `answer`가 없으면 조용히 종료한다. 도구 오류(`isError`)도 조용히
   종료한다.

## 구성 변경

- `crates/decide/src/show.rs`(새 파일): `pub fn render(input: &Value) -> Option<String>`. 입력 JSON을
  받아 사용자에게 보일 문자열을 만든다. 순수 함수다.
- `crates/decide/src/lib.rs`: `pub mod show;`
- `crates/decide/src/main.rs`: `Some("hook")`가 stdin 전체를 읽어 `show::render`를 부르고, 결과가 있으면
  `{"systemMessage": ...}`를 한 줄로 출력한다. 도움말 텍스트에 `hook` 한 줄을 더한다.
- README: 등록 방법과 모양, 끄는 방법. CHANGELOG `[Unreleased]`.

## 등록

사용자 설정(`~/.claude/settings.json`) 또는 프로젝트 설정에 다음을 넣는다. 이 저장소에서 `.claude/`는
gitignore 대상이라 설정은 저장소로 배포되지 않고 README의 스니펫으로만 안내한다.

```json
{
  "hooks": {
    "PostToolUse": [
      {
        "matcher": "mcp__decide__decide",
        "hooks": [{"type": "command", "command": "/opt/homebrew/bin/decide hook", "timeout": 5}]
      }
    ]
  }
}
```

설정을 바꾸는 것은 사용자가 직접 한다. 등록을 자동화하는 `decide install` 확장은 이번 범위 밖이다.

## 테스트

기본 `cargo test`는 네트워크와 키 없이 돈다. 이번 세션에서 실제로 받은 세 응답(choice, noul, score)을
픽스처로 쓴다.

- choice, noul, score가 위 형식과 정확히 일치한다(백엔드, 모델, 지연 포함).
- `tool_response`가 문자열, `structuredContent` 객체, `content[0].text` 객체일 때 같은 결과가 나온다.
- 다른 도구 이름, 깨진 JSON, `answer` 없음, `isError`, 빈 입력은 `None`이다.
- 질문이 80자를 넘으면 잘리고, 선택지가 5개여도 나머지는 최대 3개다.
- `routing.cached == true`이면 `(캐시)`가 붙고 지연이 빠진다.
- CLI: `decide hook`에 픽스처를 stdin으로 주면 stdout이 `{"systemMessage": ...}` 한 줄이고 종료 코드가 0이다.
  알 수 없는 입력이면 stdout이 비고 종료 코드가 0이다.
- 기존 CLI 테스트(도움말, 알 수 없는 인자는 종료 코드 2)는 수정 없이 통과한다.

## 실세션 확인

구현 뒤 사용자가 위 설정을 넣고(릴리스 전에는 `cargo build`로 만든 바이너리 경로로) 새 세션에서
`decide`를 부르는 프롬프트를 실행해, 한 줄이 실제로 보이고 모델이 그 줄을 인용하지 않는지 확인한다.

## 범위 밖

- `decide_many` 표시(머지·릴리스 뒤 추가), 색상과 터미널 제어 문자.
- 훅 등록 자동화, 모델에게 같은 요약을 주는 `additionalContext`.
- 임계값 판단과 경고 표시.

## 알려진 위험

- MCP 도구의 `tool_response`가 실제 세션에서 어떤 모양으로 오는지, `systemMessage`가 어떻게 보이는지는
  공식 문서 요약으로만 확인했다. 입력을 세 형태로 받는 이유이고, 실세션 확인 전에는 "동작한다"고 말할
  수 없다.
- 호출마다 훅 프로세스가 한 번 뜬다. 바이너리가 작고 네트워크를 쓰지 않아 지연은 작을 것으로 보지만
  재 보지 않았다.
- 바이너리에 `hook`이 들어 있어야 해서 새 릴리스가 필요하다. 설치된 0.0.4에는 없다.
