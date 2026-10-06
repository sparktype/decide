# 설계: 설정 파일로 백엔드와 HuggingFace 환경을 지정

## 배경

`decide`는 지금 `DECIDE_BACKEND`, `TYPESAFE_API_KEY`, `CLEF_WEIGHTS`를 프로세스
환경변수로만 읽는다(`backend.rs::Env::from_process`, `local/mod.rs::pinned_weights_dir`).
MCP 서버로 띄우는 `decide mcp`는 `.mcp.json`의 `env`에 적지 않으면 셸의 환경변수를
물려받지 못하는 경우가 있어, 값을 영속적으로 바꾸고 싶을 때 매번 `.mcp.json`을
고쳐야 한다. 사용자가 `~/.config/decide/config.toml` 설정 파일로 이 값들을 지정하고,
HuggingFace 쪽 환경변수(`HF_ENDPOINT`, `HF_HOME`, `HF_TOKEN` — 사내망 Nexus HF 미러를
쓰려면 `HF_ENDPOINT`가 필요하다)도 같은 파일에서 설정할 수 있게 해 달라고 요청했다.

조사 중 기존 버그를 하나 발견했다: `local/mod.rs::download_weights`는
`hf_hub::api::sync::Api::new()`를 쓰는데, 이 생성자는 `HF_ENDPOINT`/`HF_HOME`
환경변수를 **전혀 읽지 않는다**(`hf-hub` 0.5.0은 `ApiBuilder::from_env()`를 써야
읽는다). 그래서 지금은 셸에서 `HF_ENDPOINT`를 export해도 효과가 없다. `HF_TOKEN`은
`hf-hub`가 환경변수로는 전혀 지원하지 않는다(파일 기반 토큰만 지원). 이 설계는
설정 파일 추가와 함께 이 버그도 고친다.

## 결정

- 새 설정 파일은 `~/.config/decide/config.toml` 하나다. `gates.json`과 달리
  저장소 레이어(`<cwd>/.decide/config.toml`)는 두지 않는다 — API 키·로컬 경로는
  기기별·비밀 성격의 설정이라 저장소에 커밋할 것이 아니다.
- **우선순위는 환경변수 > `config.toml` > 없음(하드 기본값 없음)이다.** 환경변수가
  설정돼 있으면 같은 키의 TOML 값은 무시한다. 둘 다 없으면 기존 동작과 같다
  (`select_backend`가 키 유무로 TypeSafe/로컬을 고르고, 가중치는 HuggingFace에서
  내려받는다).
- 오류 처리는 `gates.json`과 같은 관례를 따른다: 파일이 없으면 조용히 건너뛰고,
  읽기 실패나 TOML 파싱 실패는 stderr에 경고 한 줄을 내되 `decide`를 멈추지 않고
  그 레이어를 빈 채로(환경변수/기본값으로) 계속 진행한다. 종료 코드는 바뀌지 않는다.
  모르는 키는 조용히 무시한다(전진 호환).

## 스키마

```toml
backend = "local"        # DECIDE_BACKEND와 같은 뜻 ("typesafe" 또는 "local")

[typesafe]
api_key = "sk-..."       # TYPESAFE_API_KEY와 같은 뜻

[local]
weights = "/path/to/weights"   # CLEF_WEIGHTS와 같은 뜻
hf_endpoint = "https://nexus.auto-hmg.io/repository/huggingface-proxy"  # HF_ENDPOINT
hf_home = "/path/to/cache"      # HF_HOME
hf_token = "hf_..."             # HF_TOKEN (hf-hub가 env로는 지원하지 않아 decide가 직접 적용)
```

모든 키는 선택이고, 섹션이 없거나 비어 있으면 해당 값은 미설정으로 본다.

## 구성 변경

### 새 모듈 `crates/decide/src/config.rs`

`gate/config.rs`의 파일-읽기 패턴을 그대로 따른다.

```rust
#[derive(Debug, Clone, Default, PartialEq)]
pub struct FileConfig {
    pub backend: Option<String>,
    pub typesafe_api_key: Option<String>,
    pub local_weights: Option<String>,
    pub local_hf_endpoint: Option<String>,
    pub local_hf_home: Option<String>,
    pub local_hf_token: Option<String>,
}

/// `~/.config/decide/config.toml`. HOME이 없거나 비면 None.
pub fn config_path(home: Option<&str>) -> Option<PathBuf>;

/// 파일을 읽어 파싱한다. 파일 없음은 조용히 빈 값, 읽기·파싱 실패는 경고 문자열과 빈 값.
pub fn load_from_disk(home: Option<&str>) -> (FileConfig, Vec<String>);

fn parse(text: &str) -> Result<FileConfig, String>; // toml::from_str, 알려진 키만 꺼낸다
```

### `backend.rs::Env::from_process`

```rust
pub fn from_process() -> Self {
    let (file, warnings) = crate::config::load_from_disk(std::env::var("HOME").ok().as_deref());
    for warning in &warnings {
        eprintln!("{warning}");
    }
    Self {
        backend: std::env::var("DECIDE_BACKEND").ok().or(file.backend),
        api_key: std::env::var("TYPESAFE_API_KEY").ok().or(file.typesafe_api_key),
    }
}
```

`Env` 구조체 자체(필드)는 바꾸지 않는다. 로컬 전용 설정(`weights`/`hf_*`)은 `Env`에
넣지 않는다 — `Env`는 `decide`/`decide_many` 호출마다 만들어지는 가벼운 구조체이고,
로컬 가중치·HF 설정은 `local::runtime()`이 프로세스당 한 번만 초기화할 때만 필요하다.

### `local/mod.rs`

```rust
struct ResolvedHf {
    weights: Option<PathBuf>,
    endpoint: Option<String>,
    home: Option<String>,
    token: Option<String>,
}

/// 환경변수(CLEF_WEIGHTS/HF_ENDPOINT/HF_HOME/HF_TOKEN) > config.toml > None, 필드별로 독립 적용.
fn resolved_hf() -> ResolvedHf;
```

- `pinned_weights_dir()`는 `resolved_hf().weights`를 쓰도록 바꾼다(지금의 공백 트림
  규칙은 유지).
- `download_weights()`는 `hf_hub::api::sync::Api::new()` 대신
  `ApiBuilder::from_env()`(이 호출 자체가 `HF_HOME`/`HF_ENDPOINT` 환경변수를
  반영한다 — 버그 수정)로 시작해, `resolved_hf()`의 `endpoint`/`home`/`token`이
  있으면 `.with_endpoint(...)`/`.with_cache_dir(...)`/`.with_token(Some(...))`로
  덮어쓴다. `resolved_hf()`가 이미 "환경변수 > config.toml" 우선순위를 적용했으므로
  이 지점에서는 추가 분기가 필요 없다.

### `Cargo.toml`

`toml = "0.8"`을 의존성에 추가한다(로컬 Cargo 캐시에 이미 있어 네트워크 불필요).

### `lib.rs`

`pub mod config;` 추가.

## 테스트

- `config.rs`: 파일 없음(빈 `FileConfig`, 경고 없음), 전체 키 왕복(기존 `gates.json`
  테스트의 `temp_dir` 헬퍼 재사용), 깨진 TOML(경고 1개 + 빈 `FileConfig`), 모르는
  키 무시, 디렉터리를 파일 자리에 둔 경우(읽기 실패 경고).
- `backend.rs`: 임시 HOME에 `config.toml`을 두고 `DECIDE_BACKEND` 환경변수가
  없으면 TOML 값을 쓰고, 있으면 환경변수가 이긴다는 테스트.
- `local/mod.rs`: `resolved_hf()`가 환경변수와 TOML 값을 올바른 우선순위로
  합치는 단위 테스트(필드별 독립 — 예: `CLEF_WEIGHTS`만 환경변수에 있고 나머지는
  TOML). 실제 가중치 다운로드·`ApiBuilder` 네트워크 호출은 테스트하지 않는다(기존
  `ensure_weights_downloads_when_cache_empty`처럼 네트워크가 필요한 테스트는 그대로
  둔다).

## 문서

README의 "환경 변수" 절에 설정 파일 경로와 스키마, 우선순위를 추가한다. CLAUDE.md의
`local/mod.rs` 설명에 `CLEF_WEIGHTS`와 함께 설정 파일을 언급한다.

## 범위 밖

- 저장소 레이어(`<cwd>/.decide/config.toml`)는 두지 않는다.
- 게이트 설정(`gates.json`)과의 통합이나 공통 모듈화는 하지 않는다 — 두 설정은
  성격(비밀/경로 vs 정책 규칙)이 달라 당장 합칠 이유가 없다.
- `decide install` 등 설치 명령이 `config.toml`을 생성하거나 편집하지 않는다.
  사용자가 직접 쓴다.
