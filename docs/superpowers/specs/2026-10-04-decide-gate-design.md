# decide gate 설계 — Claude Code 훅에서 decide로 판정하기

## 배경

decide는 지금 Claude Code와 세 군데에서 만난다. MCP 도구(`decide`, `decide_many`)는 Claude가 스스로
부를 때만 쓰이고, PostToolUse 표시 훅(`decide hook`)은 결과를 보여 주기만 하며, Stop 훅
(`.claude/hooks/stop_verify.py`)은 이 저장소 안에서만 돈다. 어느 것도 훅의 판정(허용·거부·차단)을
내리지 못한다.

Claude Code 훅에는 `mcp_tool` 유형이 있지만, 도구의 텍스트 결과를 훅 출력 JSON(`permissionDecision`,
`decision: "block"` 등)으로 해석한다. decide는 `{"answer": ...}`를 돌려주므로 그대로는 판정이 되지 않는다.

커뮤니티에는 같은 자리에 Jev를 붙인 사례가 많다(hookgate, jev-harness, jev-permission-gate, jev-claude).
대부분 TypeSafe API로 상태를 보낸다. decide는 로컬 MLX 엔진이 있어 명령어·파일·웹 결과를 기기 밖으로
보내지 않고 판정할 수 있다는 점이 다르다.

## 목표

1. `decide gate <이름>` — 훅 입력 JSON을 stdin으로 받아 게이트 질문을 데몬에 보내고, 이벤트에 맞는 훅
   출력 JSON을 stdout에 쓴다.
2. 판정 근거를 사용자에게 표시한다 — 보낸 질문과 결과를 `systemMessage`로 보여 준다.
3. 게이트 질문 템플릿을 출력한다 — `decide gate --show [이름]`으로 설정된 질문·임계값·동작을 보여 준다.
4. 데몬 버전 확인 — 업그레이드 뒤 옛 데몬이 계속 답하지 않게 한다.
5. 첫 게이트는 PreToolUse Bash 위험 게이트 하나다. 기본은 감사 모드(기록만, 막지 않음)다.

## 범위 밖

Stop 완료 검증의 정식 기능화, PostToolUse 지시문 주입 탐지, UserPromptSubmit 조건부 지시는 이 설계가
깔리면 게이트 정의만 추가하는 후속 작업으로 둔다. `mcp_tool` 훅용 출력도 이번에는 하지 않는다 — 데몬을
거치는 `command` 훅이 MCP 연결 여부와 무관하고 실패 시 통과가 쉽다.

## 동작

### 실행 흐름

```
Claude Code 훅(command) ── stdin: 훅 입력 JSON ──▶ decide gate bash-risk
  1. 게이트 정의를 읽는다(내장 기본값 → 사용자 설정 → 저장소 설정 순, 저장소는 조이기만 가능).
  2. 사전 필터: 규칙에 걸리는 명령(예: git status, ls)은 데몬을 부르지 않고 끝낸다.
  3. 훅 입력에서 state를 만든다(필요한 것만: 명령 문자열, cwd 끝부분).
  4. 데몬(~/.cache/decide/decide.sock)에 질문한다. 데몬이 없으면 띄워 두고 이번은 통과한다.
  5. 결과를 임계값으로 판정으로 바꾼다.
  6. stdout: 이벤트별 훅 출력 JSON(판정 + systemMessage). 감사 로그에 한 줄 쓴다.
```

### 실패 시 통과

데몬 연결 실패, 시간 초과, 백엔드 오류, 입력을 읽을 수 없음 — 모두 판정 없이 종료 코드 0으로 끝내
Claude Code의 기본 권한 흐름을 그대로 둔다. 실패 사실은 감사 로그와 `systemMessage`(한 줄)에 남긴다.

### Bash 위험 게이트

| 항목 | 값 |
|---|---|
| 이벤트 / matcher | `PreToolUse` / `Bash` |
| 질문 타입 | `choice` |
| 질문 | "이 셸 명령은 저장소 밖의 데이터나 상태를 파괴하거나 되돌리기 어렵게 바꾸는가?" |
| 선택지 | `allow`(무해), `ask`(확인 필요), `deny`(파괴적) |
| state | 명령 문자열, cwd 끝 두 단계 |
| 판정 | `deny` 확률 ≥ 0.7 → `deny`, 최고 확률 < 0.7 → `ask`, 그 외 `allow`가 가장 높으면 판정 없음(기본 흐름) |
| 감사 모드 | 위 판정을 계산하고 표시·기록만 한다. 훅 출력에 `permissionDecision`을 **넣지 않아** Claude Code의 권한 흐름이 바뀌지 않는다. enforce 모드에서만 `permissionDecision`을 낸다 |
| 자동 허용 | 하지 않는다. `allow`는 "판정하지 않음"으로 처리한다 |

### 판정 근거 표시

모든 판정은 `systemMessage`에 다음을 담는다. `show.rs`의 기존 한 줄 형식을 재사용하고, 게이트 이름과
실제로 보낸 질문을 앞에 붙인다.

```
🛡 decide gate bash-risk: ask (감사 모드 — 막지 않음)
   질문: 이 셸 명령은 저장소 밖의 데이터나 상태를 파괴하거나 되돌리기 어렵게 바꾸는가?
   대상: rm -rf ~/Downloads/old
   선택: deny 62% · ask 30% · allow 8%
   local · clef-flash · 540ms
```

- 대상(state)은 80자에서 자른다. 비밀값으로 보이는 토큰(키, 토큰, 비밀번호 패턴)은 표시와 전송 전에 가린다.
- 사전 필터로 끝난 경우와 판정 없음(`allow`)인 경우는 기본적으로 표시하지 않는다(`display: "decisions"`).
  모든 호출을 보고 싶으면 `display: "all"`, 끄려면 `display: "off"`.
- 실패 시 통과한 경우는 한 줄만 표시한다: `🛡 decide gate bash-risk: 판정 없이 통과 (데몬 응답 없음)`.
- `show.rs`의 알려진 한계(로컬 `score`의 `legend`가 배열이면 요약을 못 만듦)를 이번에 함께 고친다.

### 게이트 질문 템플릿 출력

```
$ decide gate --show
bash-risk   PreToolUse/Bash   choice   감사 모드   display=decisions
user-rules  ~/.config/decide/gates.json (없음)
repo-rules  ./.decide/gates.json (없음)

$ decide gate --show bash-risk
이벤트:   PreToolUse (matcher: Bash)
타입:     choice
질문:     이 셸 명령은 저장소 밖의 데이터나 상태를 파괴하거나 되돌리기 어렵게 바꾸는가?
선택지:   allow — 저장소 안 작업이거나 읽기 전용
          ask   — 영향 범위가 불분명함
          deny  — 저장소 밖을 지우거나 되돌리기 어렵게 바꿈
state:    {command}, {cwd_tail}
임계값:   deny ≥ 0.50, 최고 확률 < 0.70 이면 ask
모드:     audit
사전 필터: git status|git diff|git log|ls|pwd|cat …(12개)
출처:     내장 기본값
```

- 출력은 사람이 읽는 표가 기본이고, `--json`이면 같은 내용을 JSON으로 낸다(설정 파일로 복사해 고칠 수 있게).
- 설정이 여러 층에서 합쳐졌으면 각 값의 출처(내장/사용자/저장소)를 표시한다.

### 설정

```json
{
  "mode": "audit",
  "display": "decisions",
  "timeout_ms": 2000,
  "gates": {
    "bash-risk": {
      "enabled": true,
      "thresholds": { "deny": 0.7, "confidence": 0.7 },
      "prefilter": ["git status", "git diff", "git log", "ls", "pwd"]
    }
  }
}
```

우선순위: 내장 기본값 → `~/.config/decide/gates.json` → 저장소 `./.decide/gates.json`. 저장소 설정은
임계값을 낮추거나 게이트를 켜는 쪽(더 엄격한 쪽)만 허용한다.

### 감사 로그

`~/.cache/decide/gate.log`에 판정마다 JSON 한 줄: 시각, 게이트, 판정, 확률, 백엔드, 지연, 사전 필터 여부,
실패 사유. 명령 원문은 가린 형태로만 남긴다. 로그 요약(`decide gate --report`)은 후속 작업이다.

### 데몬 버전 확인

데몬 요청에 `"client_version"`을 싣는다. 데몬은 자기 버전과 다르면 오류 대신 `"stale": true`로 답하고
종료한다. `decide gate`는 이 답을 받으면 판정 없이 통과하고 새 데몬을 띄운다. 옛 클라이언트(버전 필드
없음)는 지금처럼 동작한다.

### 설치

`decide install --claude`가 표시 훅에 더해 게이트 훅도 등록한다(PreToolUse, matcher `Bash`, command
`/opt/homebrew/bin/decide gate bash-risk`). 기존 `claude.rs`의 병합 규칙(정확히 같은 명령이면 멱등, 병합할 수
없는 모양이면 거부, 백업 후 교체)을 그대로 쓴다.

## 지연 예산

| 경로 | 지연 | 근거 |
|---|---|---|
| 사전 필터 적중 | 수 ms | 데몬 호출 없음 |
| TypeSafe | 약 250ms | 2026-10-04 MCP 호출 실측 |
| 로컬 MLX, 짧은 명령 | 0.5~0.9초 | 2026-10-04 실측 |

로컬 엔진은 모든 Bash 호출에 쓰기엔 느리므로 사전 필터가 필수다. 데몬 LRU 캐시는 같은 명령 반복에만 듣는다.

## 테스트

- 순수 함수: 훅 입력 → state, 결과 → 판정(임계값 경계), 판정 → 훅 출력 JSON, 판정 → systemMessage 문구,
  설정 병합(저장소는 조이기만), 비밀값 가리기, `--show` 출력(표/JSON). 백엔드는 지금처럼 스크립트 전송으로 주입.
- 실패 시 통과: 데몬 없음, 시간 초과, 잘못된 입력, `stale` 응답.
- 게이트 평가 세트: 위험 명령과 일상 명령 골든 케이스(처음 보는 세트를 따로 둔다). 실제 가중치가 필요하므로
  `#[ignore]`나 기능 플래그로 기본 스위트에서 뺀다.

## 확정한 결정 (2026-10-04, 사용자가 기본값대로 진행하기로 함)

1. 설정 파일 위치는 `~/.config/decide/gates.json`과 저장소 `./.decide/gates.json`이다.
2. 판정 근거 표시 기본값은 `decisions`(판정한 것만)이다.
3. 감사 모드에서 enforce로 넘어가는 것은 자동이 아니다. 판정 로그를 사용자가 검토한 뒤 `mode`를 직접 바꾼다.
4. 첫 게이트는 Bash 위험 게이트 하나다.

구현 계획은 `docs/decide-gate/plan.md`다.
