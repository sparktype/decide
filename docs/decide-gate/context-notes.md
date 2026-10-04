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

## 2026-10-04 (2단계 완료)
- 결정: `daemon::handle_request`가 `(응답, 계속 서비스할지)`를 돌려주고 `handle_line`은 그 래퍼로 남겼다(기존 테스트 유지). 요청의 선택 필드 `client_version`이 `daemon::VERSION`(`CARGO_PKG_VERSION`)과 다르면 백엔드를 부르지 않고 `{"stale":true,"version":…}`로 답한 뒤 `serve` 루프가 끝난다. 필드가 없는 옛 클라이언트는 그대로 동작한다.
- 사실: `parse_arguments`는 알 수 없는 필드를 무시하므로 `client_version`이 캐시 키에 영향을 주지 않는다.
- 사실(기존 결함 발견): `daemon::tests::line_protocol_returns_one_json_object`는 로컬 백엔드를 부르면서 `CLEF_WEIGHTS` 보호 장치가 없어, 이 테스트만 단독 실행하면 실제로 HuggingFace에서 약 10GB를 내려받는다(244초). 전체 스위트에서는 `runtime()`의 `OnceLock`이 다른 테스트의 보호 장치 에러를 캐시해서 가려져 있었다. backend.rs 테스트와 같은 방식으로 없는 디렉터리를 가리키게 고쳤다(0.31초).
- 사실: 위 단독 실행 때문에 `~/.cache/huggingface/hub/models--mlx-community--clef-flash-8bit`(약 10GB)가 생겼다. `CLEF_WEIGHTS` 없이 설치본(`/opt/homebrew/bin/decide`)을 쓸 때 바로 쓰이는 캐시라 지우지 않았다. 로컬 `~/.cache/decide/clef-flash-8bit`와 같은 내용이라 필요 없으면 한쪽을 지워도 된다.
- 알려진 취약점(이번 범위 밖): 테스트들이 `CLEF_WEIGHTS`를 `set_var`/`remove_var`로 만지는 방식은 병렬 실행에서 경쟁 조건이 있다. 지금은 `OnceLock`이 첫 에러를 캐시해서 드러나지 않을 뿐이다.

## 2026-10-04 (3단계 완료)
- 결정: `gate::config::load(user, repo)`는 텍스트를 받는 순수 함수이고 `load_from_disk(home, cwd)`가 파일을 읽는 얇은 래퍼다. `Loaded { config, sources, warnings }`를 돌려줘 `--show`가 값마다 출처(내장/사용자/저장소)를, 무시한 항목의 경고를 보일 수 있다.
- 결정: 저장소 층이 조일 수 있는 것은 mode(audit→enforce), enabled(꺼짐→켜짐), thresholds.deny(낮추기), thresholds.confidence(올리기), prefilter(줄이기)다. display와 timeout_ms는 저장소가 못 바꾼다(사용자 취향과 성능 문제라 보안 방향이 없다). 풀려는 값은 경고와 함께 무시한다.
- 결정: 저장소가 이미 적용된 값과 같은 값을 쓰는 것은 풀려는 시도가 아니므로 경고도 출처 변경도 없다(테스트로 고정).
- 결정: 잘못된 JSON 층은 통째로 건너뛰고 경고하며, 잘못된 값은 그 값만 무시하고 같은 층의 나머지는 적용한다. 알 수 없는 게이트 이름은 경고 후 무시한다.
- 결정: 내장 사전 필터는 12개(git status/diff/log/show/branch, ls, pwd, cat, head, tail, wc, which)다. 실제 적용 규칙(파이프·`;`·`&&`·`$(`가 있으면 제외)은 4단계에서 구현한다.
- 참고: 새 파일 첫 줄에 한국어 한 줄 역할 주석을 둔다(저장소 규칙). 전체 101개 테스트 통과, 빌드 경고 없음.

## 2026-10-04 (4단계 완료)
- 결정: `gate/bash_risk.rs`는 순수 함수 모음이다(`from_hook`, `redact`, `cwd_tail`, `request`, `prefiltered`, `probs_from`, `judge`). 외부 크레이트 없이 단어 단위로 비밀값을 가린다(정규식 의존성을 늘리지 않음).
- 결정: 비밀값 가리기 규칙은 이름에 KEY/TOKEN/SECRET/PASSWORD/PASSWD/CREDENTIAL/AUTH가 든 `NAME=값`, `--password/--token/--secret/--api-key/--apikey/--auth/Bearer` 다음 단어, `scheme://user:pw@host`, 알려진 토큰 접두어(sk-, ghp_ 등, AKIA)다. `mkdir -p`처럼 일반 명령은 건드리지 않는 것을 테스트로 고정했다. 이름에 AUTH가 든 `GIT_AUTHOR_NAME` 같은 값은 과하게 가려질 수 있지만 무해하다.
- 결정: 사전 필터는 항목으로 시작하고(단어 경계) `; & | ` $ < > ( ) \` 개행이 하나도 없을 때만 참이다. 따라서 `ls; rm -rf /`, `ls | xargs rm`, `cat a > /etc/hosts`, `ls $(…)`은 필터를 통과하지 못하고 모델로 간다.
- 결정: 판정은 `deny` 확률 ≥ deny 임계값이면 Deny, 최고 확률 < confidence면 Ask, 아니면 최고 선택지이며 동률이면 더 엄격한 쪽이다. 경계값 0.5와 0.7을 테스트로 고정했다.
- 결정: 선택지 의미는 모델에 라벨만 가므로 질문 문장 뒤에 "선택지 — allow: …/ask: …/deny: …"로 같이 넣는다. 요청 state는 가린 명령(2000자 상한)과 cwd 끝 두 단계다.
- 참고: 의도적으로 "cat 같은 읽기 명령은 사전 필터 통과"다. 민감 파일을 읽는 것은 이 게이트의 질문(파괴·되돌리기 어려운 변경) 범위가 아니다.

## 2026-10-04 (5단계 완료)
- 결정: `gate/output.rs`의 `hook_output(&Outcome) -> Option<Value>`가 훅이 낼 JSON을 만든다. `Outcome.kind`는 `Prefiltered`/`Judged{verdict, probs, result}`/`Failed{reason}`이고 명령은 호출자가 가린 값을 넘긴다.
- 결정: 감사 모드는 `systemMessage`만 내고 `hookSpecificOutput`을 절대 내지 않는다(테스트로 고정). enforce는 Deny/Ask일 때만 `permissionDecision`(+이유)을 더한다. 실패(`Failed`)는 enforce여도 결정하지 않는다(실패 시 통과).
- 결정: 표시 규칙은 `Off`면 문구 없음(enforce 결정은 문구 없이도 낸다), `Decisions`면 Ask/Deny와 실패만, `All`이면 사전 필터와 allow까지다.
- 결정: 근거 문구는 설계서 형식 그대로이고 푸터는 `show::footer`를 재사용한다(캐시 표시 포함). 확률은 큰 순으로 정렬하되 동률이면 deny, ask, allow 순이다. 대상은 `show::truncate`로 80자에서 자른다. 설정 경고가 있으면 마지막에 "설정 경고 n건"을 한 줄 붙인다.
- 사실: `show.rs`의 알려진 한계(로컬 score의 `legend`가 배열이면 요약이 조용히 사라짐)를 고쳤다. 객체(TypeSafe)와 배열(로컬) 둘 다 읽는다. `truncate`, `pct`, `footer`는 `pub(crate)`로 열었다.
