# decide gate 구현 계획

설계는 `docs/superpowers/specs/2026-10-04-decide-gate-design.md`다. 이 문서는 그 설계를 어떤 순서로,
무엇으로 검증하며 만들지 정한다.

## 0단계 선결 문제 — 로컬에서 빌드가 안 된다

이 작업 머신에서 `/Applications/Xcode.app`이 사라져 `cargo build`/`cargo test`가
`xcrun: unable to find utility "metal"`로 `mlx-sys` 빌드 스크립트에서 실패한다(디버그 캐시도 무효화됨).
`MLX_RS_METAL_PATH`는 결과물 위치만 정할 뿐 Metal 컴파일을 건너뛰지 않는다.
구현은 컴파일과 테스트를 반복해야 하므로 아래 중 하나가 먼저 필요하다.

| 길 | 내용 | 비용 |
|---|---|---|
| A. Xcode 16 이상 재설치(권장) | 로컬에서 `cargo test` 바로 반복 | 다운로드와 설치 수십 분, 디스크 약 10GB 이상 |
| B. CI로만 검증 | PR에서 돌리는 CI를 만들어 빌드·테스트 | 반복마다 약 16분(빌드 캐시를 넣어도 첫 회는 같음) |

A가 아니면 B의 PR용 CI가 이 계획의 일부가 된다. 어느 쪽이든 아래 1~9단계의 순수 함수 설계는 같다.

## 확정한 기본값

설정 경로 `~/.config/decide/gates.json`, `./.decide/gates.json`(저장소는 조이기만 허용), 표시
`display: "decisions"`, enforce 전환은 사용자가 로그를 검토한 뒤 직접, 첫 게이트는 `bash-risk` 하나.
감사 모드는 계산·표시·기록만 하고 훅 출력에 `permissionDecision`을 넣지 않는다.

## 구조

새 모듈 `crates/decide/src/gate/`. 모두 순수 함수 위주이고 I/O는 `client.rs`에만 둔다.

| 파일 | 역할 |
|---|---|
| `gate/mod.rs` | 진입점 `run(name, stdin, 환경) -> 출력 JSON`: 읽기, 필터, 질문, 판정, 출력의 흐름 |
| `gate/config.rs` | 설정 타입, 내장 기본값, 층 병합(저장소는 조이기만), `--show` 출력용 출처 추적 |
| `gate/bash_risk.rs` | state 만들기(명령, cwd 꼬리), 비밀값 가리기, 사전 필터, 확률 → 판정 |
| `gate/output.rs` | 판정 → 훅 출력 JSON(감사/enforce), systemMessage 문구, 실패 시 통과 문구, `--show` 표·JSON |
| `gate/client.rs` | 데몬 소켓 호출(시간 제한), 데몬 없으면 띄우기, `stale` 처리, 감사 로그 쓰기 |

기존 코드 변경은 최소로 한다. `show.rs`의 `truncate`/`pct`/`footer`는 `pub(crate)`로 열어 재사용하고,
`claude.rs`의 훅 설치는 이벤트와 matcher를 받도록 일반화한다.

## 작업 순서

각 작업은 "실패하는 테스트 → 구현 → 통과"로 진행하고 작업마다 커밋한다. 괄호는 검증 방법이다.

0. **빌드 환경 복구**(위 표). (검증: `cargo test --lib` 통과.)
1. **훅 설치 일반화.** `claude.rs`의 `add_hook`/`install_hook`이 `(event, matcher, command)`를 받게 한다.
   PostToolUse 동작과 기존 테스트는 그대로다. (검증: 기존 `claude::tests` 전부 + PreToolUse 병합·멱등·거부 케이스.)
2. **데몬 버전 확인.** 요청의 선택 필드 `client_version`이 데몬 버전과 다르면 `{"stale":true,"version":…}`로
   답하고 `serve` 루프를 끝낸다. 필드가 없으면 지금처럼 동작한다. (검증: 스크립트 전송으로 일치/불일치/없음,
   `serve`가 stale 뒤 소켓을 지우고 끝나는지.)
3. **설정.** 타입, 내장 기본값, 층 병합(저장소 설정은 임계값을 낮추거나 게이트를 켜는 쪽만 반영), 출처 추적.
   (검증: 병합 표 — 저장소가 풀려는 값은 무시되는지, 잘못된 JSON은 내장 기본값으로 떨어지고 경고가 남는지.)
4. **bash-risk 순수 로직.** 비밀값 가리기(키, 토큰, 비밀번호 패턴), state 만들기, 사전 필터(`git status` 등
   정확히 같은 명령 앞부분이고 파이프·`;`·`&&`·`$(`가 없을 때만), 확률 → 판정(경계값 0.5, 0.7).
   (검증: 경계값, 파이프가 든 명령은 사전 필터 제외, 비밀값 가림.)
5. **출력.** 감사 모드는 `systemMessage`만, enforce는 `permissionDecision`(+이유)과 `systemMessage`.
   근거 표시 문구는 설계서 형식 그대로다. 실패 시 통과 한 줄. `show.rs`의 `legend` 배열 한계를 함께 고친다.
   (검증: 문구 골든 문자열, 감사 모드 출력에 `permissionDecision`이 없음, score `legend` 배열 케이스.)
6. **클라이언트.** 소켓 호출과 시간 제한, 데몬 없으면 `current_exe daemon`을 띄우고 이번은 통과, `stale` 답이면
   통과하고 새 데몬 띄우기, 감사 로그 한 줄. (검증: 임시 `HOME`에 가짜 데몬 스레드를 띄워 정상/무응답/오류/stale.)
7. **CLI 연결.** `main.rs`에 `gate <이름>`과 `gate --show [이름] [--json]`, 도움말 갱신. (검증: `tests/gate.rs`가
   바이너리를 실행해 stdin 훅 JSON → stdout JSON, 잘못된 입력은 빈 출력과 종료 코드 0.)
8. **설치.** `decide install --claude`가 PreToolUse(`Bash`) 게이트 훅도 등록한다.
   (검증: `tests/install_claude.rs` 확장 — 처음 설치, 두 번째는 변화 없음, 백업 파일.)
9. **평가 세트.** 위험 명령과 일상 명령의 골든 케이스를 개발용과 처음 보는 검증용으로 나눠 둔다. 실제
   가중치가 필요하므로 `#[ignore]`로 기본 스위트에서 뺀다. (검증: 로컬 MLX로 정답률, 잘못된 거부, 지연.)
10. **문서.** README에 게이트 설치·감사 로그·enforce 전환 절차와 업그레이드 뒤 데몬 종료 안내,
    `CLAUDE.md` 아키텍처 항목. 릴리스(0.3.0)는 별도 PR이다.
11. **마무리 확인.** 전체 `cargo test`, 실제 훅 입력으로 `decide gate` 수동 실행(지연 측정), 새 Claude Code
    세션에서 감사 모드로 하루 정도 판정 로그를 본다.

## 위험과 대응

- **로컬 엔진 지연.** 짧은 명령도 0.5~0.9초라 모든 Bash 호출에 쓰기엔 느리다. 사전 필터가 일상 명령을 걸러야 하고,
  훅 timeout은 설정에서 조정 가능하게 둔다. 실제 호출 빈도는 11단계에서 로그로 본다.
- **오탐과 과신.** 감사 모드로 시작하고 자동 허용은 하지 않는다. 평가 세트에서 잘못된 거부가 0이어야 enforce를 검토한다.
- **비밀값 유출.** 질문 state에는 가린 명령만 담고, 로컬 엔진이면 기기 밖으로 나가지 않는다. TypeSafe 경로는 가린 문자열만 보낸다.
- **데몬 환경 상속.** 데몬은 처음 띄운 프로세스의 환경을 물려받는다(`TYPESAFE_API_KEY` 유무로 백엔드가 갈림).
  `decide gate`가 띄울 때 환경을 그대로 넘기고, 표시 문구에 실제 백엔드를 보여 혼동을 막는다.
- **설정 파일 신뢰.** 저장소 설정은 조이기만 허용한다. 저장소 `.decide/gates.json`이 게이트를 끄거나 임계값을 풀 수 없다.

## 범위 밖(후속)

Stop 완료 검증의 정식 기능화, PostToolUse 지시문 주입 탐지, UserPromptSubmit 조건부 지시, `decide gate --report`,
`mcp_tool` 훅용 출력. 모두 `gate/`에 게이트 정의를 추가하는 작업이다.
