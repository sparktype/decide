# 설계: 게이트 설정을 `gates.json`에서 `config.toml`로 통합

## 배경

지금 `decide gate bash-risk`의 설정(`mode`, `display`, `timeout_ms`, 위험 임계값,
사전 필터, 정적 규칙)은 `~/.config/decide/gates.json`(JSON)과 `<cwd>/.decide/gates.json`
(저장소 레이어, "더 엄격한 쪽으로만" 병합)로 따로 관리된다. 백엔드·HuggingFace
설정은 올해 추가한 `~/.config/decide/config.toml`(TOML)로 관리한다. 사용자가 설정
파일을 하나로 합치고 싶어 해, 게이트 설정도 `config.toml`로 옮긴다.

## 결정

- **저장소 레이어(`<cwd>/.decide/gates.json`, "조이기만 허용")는 폐기한다.** 팀
  전체에 강제되는 안전장치라는 특성을 잃지만, 사용자가 범위를 좁혀 명시적으로
  택한 단순화다. 게이트 설정은 이제 사용자 `config.toml` 하나뿐이다.
- `gates.json` 파일과 그 로더는 완전히 제거한다. `decide gate --show --json`이
  내는 JSON도 이 설계에 맞춰 TOML로 복사해 붙일 수 있는 모양으로 바꾼다(아래
  "출력 변경").
- 섹션 구조는 `config.toml`의 기존 `[local]`/`[typesafe]`와 같은 스타일로 둔다.
  `[gate]`에 전역 키, `[gate.bash-risk]`에 그 게이트 전용 키.
- 값이 없거나 `[gate]`/`[gate.bash-risk]` 섹션 자체가 없으면 필드별로 독립적으로
  내장 기본값(`builtin()`)을 쓴다 — 기존 `[local]` 필드들과 같은 규칙(`nonblank`로
  빈 문자열은 미설정, 모르는 키는 무시).
- 환경변수는 두지 않는다(기존 게이트 설정에 환경변수 오버라이드가 없었고, 이
  통합도 그 범위를 넓히지 않는다).

## 스키마

```toml
[gate]
mode = "audit"            # "audit" 또는 "enforce"
display = "decisions"     # "decisions", "all", "off"
timeout_ms = 2000

[gate.bash-risk]
enabled = true
deny = 0.7
confidence = 0.7
prefilter = ["git status", "git diff", "..."]
deny_patterns = ["rm -* / *", "..."]
ask_patterns = ["git push --force *", "..."]
```

모든 키는 선택이다. 비어 있거나 없으면 그 필드만 내장 기본값으로 떨어진다.

## 구성 변경

### `crates/decide/src/config.rs`

`FileConfig`에 게이트 필드를 추가한다. 배열 필드(`prefilter`,
`deny_patterns`, `ask_patterns`)는 `Vec<String>`을 직접 받는다(`[local]`의
스칼라 필드들과 달리 "미설정"은 `None`, "빈 배열로 명시"는 `Some(vec![])`로
구분한다 — `Option<Vec<String>>`).

```rust
pub struct FileConfig {
    // ...기존 필드...
    pub gate_mode: Option<String>,
    pub gate_display: Option<String>,
    pub gate_timeout_ms: Option<u64>,
    pub gate_bash_risk_enabled: Option<bool>,
    pub gate_bash_risk_deny: Option<f64>,
    pub gate_bash_risk_confidence: Option<f64>,
    pub gate_bash_risk_prefilter: Option<Vec<String>>,
    pub gate_bash_risk_deny_patterns: Option<Vec<String>>,
    pub gate_bash_risk_ask_patterns: Option<Vec<String>>,
}
```

`Raw`에 `#[serde(default)] gate: RawGate` 추가, `RawGate`/`RawBashRisk`로
TOML `[gate]`/`[gate.bash-risk]`를 받는다. `mode`/`display`는 문자열 그대로
받아두고(파싱은 `gate/config.rs`가 한다 — `config.rs`는 게이트의 `Mode`/`Display`
타입을 모른다), 숫자·불린·배열은 그대로 받는다. 문자열 스칼라만 `nonblank`를
거치고, 배열은 "키가 있으면 그 값, 없으면 `None`"으로 둔다(빈 배열을 명시하는
것도 유효한 설정이라 빈 문자열과 같은 취급을 하지 않는다).

### `crates/decide/src/gate/config.rs`

- `config_paths`, `load_from_disk(home, cwd)`, `apply_layer`(JSON 기반),
  저장소 레이어 관련 코드를 모두 제거한다.
- `load(user: Option<&str>, repo: Option<&str>)`(JSON 텍스트를 받는 지금 시그니처)를
  `load_from_file_config(file: &crate::config::FileConfig) -> Loaded`로 바꾼다.
  각 필드를 `builtin()` 위에 개별적으로 덮어쓰고, 덮어쓴 필드만 `Source::User`로
  표시한다(`Source::Repo`는 제거).
- `mode`/`display` 문자열은 `"audit"|"enforce"`, `"decisions"|"all"|"off"`
  외의 값이면 경고를 남기고 그 필드만 내장 기본값으로 둔다(기존 JSON 파싱이
  잘못된 값을 다루던 방식과 동일한 관용).
- 새 공개 함수 `load_from_disk(home: Option<&str>) -> Loaded`가
  `crate::config::load_from_disk(home)`를 불러 `FileConfig`를 얻고
  `load_from_file_config`에 넘긴다. 경고는 `config::load_from_disk`가 낸
  파일 읽기/TOML 파싱 경고와 합친다.
- `to_json`(지금 `--show --json`이 쓰는 함수)을 `to_toml_string(config: &Config) -> String`으로
  바꾸고, `[gate]`/`[gate.bash-risk]` 두 섹션을 담은 TOML 문자열을 직접 만든다
  (`toml` crate의 직렬화 기능을 쓰거나, 이 저장소의 다른 출력 함수들처럼
  문자열을 직접 조립한다 — 어느 쪽이든 결과는 그 설정을 그대로
  `config.toml`에 복사해 붙일 수 있는 TOML 텍스트). `gates.<이름>.thresholds.deny`
  같은 지금의 JSON 중첩은 사라지고 스키마 절의 모양 그대로 나온다.

### `--show`/`--json` 출력 변경

`decide gate --show bash-risk --json`은 지금 JSON을 낸다. 통합 후에는 TOML을
낸다(`config.toml`에 그대로 붙여 쓸 수 있어야 하므로). **`decide gate stats --json`은
바꾸지 않는다** — 그건 감사 로그 집계 결과를 내는 별개 명령이고, "설정 파일에
복사해 붙이기"라는 목적이 없다. 같은 `--json` 플래그 이름이 하위 명령에 따라
다른 포맷(설정 조회는 TOML, 통계 조회는 JSON)을 낸다는 비일관성은 받아들인다
— 새 플래그(`--toml`)를 만드는 대안도 검토했으나, 플래그를 늘리는 것보다
"무엇을 보여주는 명령인가"에 따라 그 명령에 맞는 포맷을 내는 쪽을 택했다.
`help.rs`의 `gate` 도움말에서 "`--json`은 `--show <이름>`과 함께 쓴다. 설정
파일에 그대로 복사할 수 있는 JSON으로 낸다"를 "...TOML로 낸다"로 고치고,
`stats`의 `--json` 설명(JSON)은 그대로 둔다. 기존 `--show --json` 사용자가
있다면 깨지는 변경이지만, 이 저장소는 아직 널리 배포되지 않았고 사용자가
직접 요청한 통합이라 받아들인다.

### `Source::Repo` 제거

`Source` enum에서 `Repo`를 제거하고(`Builtin`/`User`만 남는다), `source_label`과
`--show`의 "저장소" 출처 표시를 뺀다.

### `crates/decide/src/gate/output.rs`

`show_overview(loaded, user, repo)`(`output.rs:40`)가 `Location`을 두 개
받아 `user-rules`/`repo-rules` 두 줄을 낸다. 저장소 레이어가 없어지므로
`repo` 인자와 그 줄을 지우고 `show_overview(loaded, user)`로 바꾼다. `user`
줄의 라벨은 `~/.config/decide/gates.json`에서 `~/.config/decide/config.toml`로
고친다(`main.rs:233`에서 만드는 `Location.label`도 같이 고친다).

### `main.rs`/`gate/mod.rs`의 호출부

`run_gate_show`/`run_hook`/`run_gate_stats`(`main.rs:185,217`)와 `run_hook`
(`gate/mod.rs:38`)이 지금 `config::load_from_disk(home, cwd)`(두 인자, 저장소
레이어 경로를 `cwd` 기준으로 찾음)를 부르는 지점을 `gate::config::load_from_disk(home)`
(한 인자)로 바꾼다.

`Context.fallback_cwd`(`gate/mod.rs:21`)와 `run_hook`이 쓰는 `cwd`(`gate/mod.rs:37`)는
**지우지 않는다** — 이 값은 저장소 레이어 조회에도 쓰였지만, 그와 별개로
감사 로그의 `cwd_tail`(`gate/mod.rs:111`)과 모델에 보내는 state의 작업
디렉터리 표시(`bash_risk::request`, `gate/mod.rs:51`)에도 쓰이는 값이라
게이트 설정과 무관하게 계속 필요하다. `main.rs:231`의
`config::config_paths(ctx.home.as_deref(), &ctx.fallback_cwd)` 호출(저장소
설정 경로를 보여주는 `--show`용)만 제거한다.

## 마이그레이션

기존 `~/.config/decide/gates.json`을 읽어 `config.toml`로 자동 변환하는 도구는
만들지 않는다(범위 밖, 사용자가 수동으로 옮긴다). `gates.json`이 남아 있어도
새 코드는 더 이상 그 파일을 읽지 않으므로 조용히 무시된다 — 경고도 내지 않는다
(파일 존재 자체를 검사하지 않기 때문).

## 테스트

- `config.rs`: 새 게이트 필드들의 왕복 테스트(`all_keys_round_trip`에 `[gate]`/
  `[gate.bash-risk]` 추가), 배열 필드가 "없음"과 "빈 배열 명시"를 구분하는 테스트.
- `gate/config.rs`: 기존 JSON 기반 테스트(`invalid_json_skips_that_layer_with_a_warning`,
  저장소 레이어 테스트 전부)를 삭제하고, TOML `FileConfig` 기반으로 다시 쓴다.
  필드별 독립 기본값 테스트, mode/display 잘못된 문자열 경고 테스트, `to_json`
  (또는 새 이름)이 TOML로 왕복되는 테스트.
- `tests/gate.rs`, `tests/cli.rs`: `--show --json`의 출력 형식이 TOML로
  바뀌었으므로 그 출력을 검증하는 테스트를 TOML 파싱 기준으로 고친다.
- `tests/gate_rules.rs`: 저장소 레이어 테스트가 없어지므로 관련 부분만 제거하고,
  규칙 자체(`heldout3`, 실제 로그 재생)의 검증은 그대로 둔다.

## 문서

README의 게이트 설정 절(`~/.config/decide/gates.json`, `.decide/gates.json`
언급)을 `config.toml`의 `[gate]`/`[gate.bash-risk]`로 고친다. CLAUDE.md의
`gate/config.rs` 설명도 "내장 기본값, `gates.json`, 저장소 레이어 병합"에서
"내장 기본값과 `config.toml`의 `[gate]`/`[gate.bash-risk]` 병합(저장소 레이어
없음)"으로 고친다.

## 범위 밖

- `gates.json` → `config.toml` 자동 마이그레이션 도구.
- 저장소 레이어("조이기만 허용")를 다른 형태로 되살리는 것.
- 새 게이트 종류 추가(지금 `bash-risk` 하나뿐이고 이 설계도 하나만 다룬다 —
  `[gate.<다른 이름>]`을 받는 일반화는 필요해지면 그때 설계한다).
