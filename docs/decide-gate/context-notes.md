# decide gate 컨텍스트 노트

## 2026-10-04
- 결정: 사용자가 설계서의 미결 네 가지를 기본값대로 진행하기로 했다. 설정 경로는 `~/.config/decide/gates.json`과 `./.decide/gates.json`(저장소는 조이기만), 표시는 `decisions`, enforce 전환은 로그 검토 후 사용자가 직접, 첫 게이트는 bash-risk 하나다.
- 결정: 판정 근거 표시와 게이트 질문 템플릿 출력(`--show`)이 이번 범위에 들어간다. 사용자가 "프롬프트 출력"이 무엇인지 묻는 질문에 "판정 근거를 사용자에게 표시"와 "게이트 질문 템플릿 출력"을 골랐다. 모델 입력 원문 출력(dry-run)과 Claude 프롬프트 주입은 하지 않는다.
- 정정: 설계서 초안은 감사 모드에서 `deny`를 `ask`로 바꿨는데 "막지 않음"과 모순이었다. 감사 모드는 판정을 계산·표시·기록만 하고 `permissionDecision`을 내지 않는 것으로 고쳤다.
- 사실: Claude Code 훅의 `mcp_tool` 유형은 MCP 도구의 텍스트 결과를 훅 출력 JSON으로 해석한다. decide는 `{"answer": …}`를 돌려주므로 그대로는 판정이 안 된다. 그래서 `command` 훅 + `decide gate`로 간다. 데몬을 거치므로 MCP 연결 여부와 무관하다.
- 사실: `PreToolUse`는 `permissionDecision`(allow/deny/ask/defer)과 `updatedInput`을 낼 수 있고 `if` 필드는 한 개의 권한 규칙만 받는다. matcher는 도구 이름이다.
- 사실: 데몬 요청은 한 줄 JSON(`state`, `type`, `instructions`, `options|criteria` 또는 `questions`)이고 `routing.backend/model`과 `latency_ms`를 돌려준다. 같은 요청은 LRU 64개에서 캐시된다.
- 사실: `claude.rs`의 훅 설치는 PostToolUse만 안다. 게이트에는 PreToolUse가 필요해 일반화가 첫 구현 작업이다.
- 위험(선결): 이 머신에서 Xcode가 사라져 로컬 `cargo build/test`가 막혀 있다. Xcode 16 이상 재설치 또는 PR용 CI 도입 중 하나가 먼저 필요하다. `MLX_RS_METAL_PATH`로는 우회할 수 없다.
- 참고: 지연 예산은 TypeSafe 약 250ms, 로컬 MLX 0.5~0.9초(짧은 입력)다. 로컬 엔진은 사전 필터 없이는 모든 Bash 호출에 쓰기 느리다.

## 2026-10-04 (평가 세트 복원)
- 결정: 사용자가 앞서 뺐던 평가 세트와 정답률 확인 단계(9단계)를 다시 계획에 넣으라고 했다. 이전 조정("정답률을 확인하지 않아도 된다")은 철회됐고 그 커밋(5ea5e13)을 되돌려 계획, 설계서, 체크리스트를 원래대로 복원했다.
- 결정: 따라서 enforce 전환 전에 평가 세트에서 잘못된 거부가 0인지 확인한다는 기준이 다시 유효하다. 평가 세트는 실제 가중치가 필요해 `#[ignore]`로 기본 스위트에서 빼고, 로컬 MLX로 정답률·잘못된 거부·지연을 잰다.
- 참고: 평가 세트가 돌아오면서 실제 가중치로 하는 시험이 다시 생겼다. 로컬 가중치(`~/.cache/decide/clef-flash-8bit`)는 그대로 남아 있다.

## 2026-10-04 (0, 1단계 완료)
- 사실: 0단계는 Xcode 27.0 설치 후 `xcodebuild -runFirstLaunch`(비밀번호 불필요)와 `xcodebuild -downloadComponent MetalToolchain`으로 풀렸다. `cargo test` 82개와 실제 가중치 parity 5/5(로짓 최대 차이 0.0948)가 새 Xcode로 컴파일한 Metal 커널에서도 같다.
- 결정: 구현 브랜치는 `feat/decide-gate`이고 계획 문서 브랜치(`docs/decide-gate-spec`) 위에서 땄다. 문서와 구현이 한 PR로 묶인다.
- 결정: 1단계는 `HookSpec { event, matcher, command, timeout }`를 받는 `add_hook_spec`/`install_hooks`를 핵심 API로 하고, 기존 `add_hook`/`install_hook`은 표시 훅용 얇은 래퍼로 남겼다. 기존 테스트 13개는 그대로 통과한다.
- 결정: `install_hooks`는 스펙 여러 개를 한 번에 병합해 파일을 한 번만 읽고 쓴다. 백업이 처음 원본이어야 해서 스펙마다 따로 설치하지 않는다(테스트로 고정).
- 결정: 중복 판정은 "같은 이벤트에 같은 명령이 있으면"이다. 다른 이벤트에 같은 명령 문자열이 있어도 넣는다.
