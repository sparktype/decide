# config.toml 설정 파일 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** `~/.config/decide/config.toml`로 백엔드 선택, TypeSafe 키, 로컬 가중치 경로,
HuggingFace 환경(엔드포인트/캐시 위치/토큰)을 설정할 수 있게 하고, 그 과정에서
`local/mod.rs`가 `HF_ENDPOINT`/`HF_HOME` 환경변수를 전혀 읽지 않던 기존 버그를 고친다.

**Architecture:** `gate/config.rs`의 파일-읽기·경고-후-진행 패턴을 그대로 따르는 새
모듈 `config.rs`를 추가한다. `backend.rs::Env::from_process`와 `local/mod.rs`의
가중치/HF 해석 지점에서 "환경변수 > config.toml > None" 우선순위로 그 값을 섞어
쓴다. `config.rs` 자체는 환경변수를 읽지 않는다 — 우선순위 판단은 소비하는 쪽
(`backend.rs`, `local/mod.rs`)에 둔다.

**Tech Stack:** Rust, `toml` crate(신규 의존성, 로컬 Cargo 캐시에 이미 있음),
`hf_hub::api::sync::ApiBuilder`(기존 의존성, 지금까지 안 쓰던 빌더 메서드로 교체).

**Spec:** `docs/superpowers/specs/2026-10-06-decide-config-file-design.md`

## Global Constraints

- 설정 파일 위치는 `~/.config/decide/config.toml` 하나다. 저장소 레이어는 없다.
- 우선순위는 환경변수 > `config.toml` > 미설정(하드 기본값 없음)이다. 환경변수가
  있으면 같은 키의 TOML 값은 완전히 무시한다(부분 병합 아님, 키 단위 override).
- 파일이 없으면 조용히 건너뛴다. 읽기 실패·TOML 파싱 실패는 stderr 경고 한 줄 +
  그 레이어를 빈 값으로 — `decide`를 멈추지 않고 종료 코드도 바꾸지 않는다.
- 모르는 TOML 키는 조용히 무시한다(전진 호환).
- `Env` 구조체(`backend.rs`)의 필드는 바꾸지 않는다. 로컬 전용 값(`weights`/`hf_*`)은
  `Env`에 넣지 않고 `local/mod.rs` 안에서만 해석한다.
- `HF_TOKEN`은 `hf-hub` 0.5.0이 환경변수로는 지원하지 않으므로, `decide`가
  `ApiBuilder::with_token(Some(...))`으로 직접 꽂아야 한다.

## Review Focus

- **TOML 파일이 섹션 없이 최상위 키만 있거나, 섹션이 있지만 키가 하나도 없는 경우** —
  빈 섹션은 파싱 오류가 아니라 "그 섹션의 모든 값이 미설정"으로 처리해야 한다.
- **`backend`/`api_key`/`weights`/`hf_*` 값이 빈 문자열("")인 경우** — 기존
  `CLEF_WEIGHTS` 처리(`pinned_weights_dir`)가 공백만 있는 값을 미설정으로 보듯,
  TOML에서 온 빈 문자열도 "값이 있다"로 잘못 취급하면 안 된다.
- **환경변수가 설정은 돼 있지만 빈 문자열인 경우** (`DECIDE_BACKEND=""`) — 환경변수
  우선 규칙이 "env가 존재하면 TOML을 보지 않는다"가 아니라 "env에 실질적인 값이
  있으면"이어야 한다. 그렇지 않으면 빈 환경변수 하나가 TOML의 유효한 설정을 조용히
  가린다.
- **`~/.config/decide/config.toml`의 부모 디렉터리(`~/.config/decide/`)가 아직
  없는 경우** — 파일이 없는 것과 같은 취급(건너뛰기)이어야 하며 오류로 보면 안 된다.
- **`[local]` 섹션에 `hf_endpoint`만 있고 `hf_home`/`hf_token`은 없는 경우** —
  필드별로 독립적으로 적용돼야 한다(섹션 전체가 all-or-nothing이면 안 된다).

---

## File Structure

| 파일 | 역할 |
| --- | --- |
| `crates/decide/src/config.rs` (신규) | `~/.config/decide/config.toml`을 읽고 파싱해 `FileConfig`로 돌려주는 순수 모듈. 환경변수는 모른다. |
| `crates/decide/src/lib.rs` (수정) | `pub mod config;` 한 줄 추가. |
| `crates/decide/src/backend.rs` (수정) | `Env::from_process()`가 `config::load_from_disk`를 불러 `backend`/`api_key`에 환경변수 우선순위를 적용. |
| `crates/decide/src/local/mod.rs` (수정) | `pinned_weights_dir()`와 `download_weights()`가 `config::load_from_disk`의 로컬 섹션 값을 환경변수 우선순위로 섞어 쓰고, `ApiBuilder::from_env()` + 명시적 override로 HF 엔드포인트/캐시/토큰을 반영. |
| `crates/decide/Cargo.toml` (수정) | `toml = "0.8"` 의존성 추가. |
| `README.md` (수정) | "환경 변수" 절에 설정 파일 경로·스키마·우선순위 추가. |
| `CLAUDE.md` (수정) | `local/mod.rs` 설명에 설정 파일 언급 추가. |

---

## Task 1: `config.rs` — 파일 읽기와 파싱

**Files:**
- Create: `crates/decide/src/config.rs`
- Modify: `crates/decide/src/lib.rs` (모듈 선언 추가)
- Modify: `crates/decide/Cargo.toml` (`toml` 의존성 추가)

**Interfaces:**
- Consumes: 없음 (최초 태스크).
- Produces:
  - `pub struct FileConfig { pub backend: Option<String>, pub typesafe_api_key: Option<String>, pub local_weights: Option<String>, pub local_hf_endpoint: Option<String>, pub local_hf_home: Option<String>, pub local_hf_token: Option<String> }` — `#[derive(Debug, Clone, Default, PartialEq)]`
  - `pub fn config_path(home: Option<&str>) -> Option<PathBuf>`
  - `pub fn load_from_disk(home: Option<&str>) -> (FileConfig, Vec<String>)` — 두 번째 값은 경고 문자열 목록(보통 0~1개).

- [ ] **Step 1: Cargo.toml에 `toml` 의존성 추가**

`crates/decide/Cargo.toml`의 `[dependencies]`에 한 줄 추가:

```toml
toml = "0.8"
```

- [ ] **Step 2: 실패하는 테스트 먼저 — 파일 없음, 전체 키 왕복, 빈 문자열, 빈 섹션**

`crates/decide/src/config.rs`를 새로 만들고 아래 테스트부터 쓴다(구현은 아직 없다. 모듈
전체가 비어서 컴파일이 실패하는 상태가 "빨강"이다):

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn temp_dir(label: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "decide-config-{label}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn config_path_joins_home_and_is_none_without_home() {
        assert_eq!(
            config_path(Some("/home/u")),
            Some(PathBuf::from("/home/u/.config/decide/config.toml"))
        );
        assert_eq!(config_path(None), None);
        assert_eq!(config_path(Some("")), None);
    }

    #[test]
    fn missing_file_is_silently_empty() {
        let home = temp_dir("missing");
        let (file, warnings) = load_from_disk(Some(home.to_str().unwrap()));
        assert_eq!(file, FileConfig::default());
        assert!(warnings.is_empty());
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn missing_config_subdir_is_also_silently_empty() {
        // ~/.config/decide/ 자체가 없는 경우(디렉터리를 만들지 않은 임시 HOME).
        let home = temp_dir("no-subdir");
        let (file, warnings) = load_from_disk(Some(home.to_str().unwrap()));
        assert_eq!(file, FileConfig::default());
        assert!(warnings.is_empty());
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn all_keys_round_trip() {
        let home = temp_dir("roundtrip");
        std::fs::create_dir_all(home.join(".config/decide")).unwrap();
        std::fs::write(
            home.join(".config/decide/config.toml"),
            r#"
backend = "local"

[typesafe]
api_key = "sk-test"

[local]
weights = "/path/to/weights"
hf_endpoint = "https://nexus.example/hf"
hf_home = "/path/to/cache"
hf_token = "hf-test"
"#,
        )
        .unwrap();
        let (file, warnings) = load_from_disk(Some(home.to_str().unwrap()));
        assert!(warnings.is_empty(), "{warnings:?}");
        assert_eq!(file.backend, Some("local".to_string()));
        assert_eq!(file.typesafe_api_key, Some("sk-test".to_string()));
        assert_eq!(file.local_weights, Some("/path/to/weights".to_string()));
        assert_eq!(file.local_hf_endpoint, Some("https://nexus.example/hf".to_string()));
        assert_eq!(file.local_hf_home, Some("/path/to/cache".to_string()));
        assert_eq!(file.local_hf_token, Some("hf-test".to_string()));
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn blank_values_are_treated_as_unset() {
        let home = temp_dir("blank");
        std::fs::create_dir_all(home.join(".config/decide")).unwrap();
        std::fs::write(
            home.join(".config/decide/config.toml"),
            r#"
backend = ""

[local]
weights = "   "
"#,
        )
        .unwrap();
        let (file, warnings) = load_from_disk(Some(home.to_str().unwrap()));
        assert!(warnings.is_empty(), "{warnings:?}");
        assert_eq!(file.backend, None);
        assert_eq!(file.local_weights, None);
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn empty_sections_are_not_an_error() {
        let home = temp_dir("empty-section");
        std::fs::create_dir_all(home.join(".config/decide")).unwrap();
        std::fs::write(home.join(".config/decide/config.toml"), "[local]\n").unwrap();
        let (file, warnings) = load_from_disk(Some(home.to_str().unwrap()));
        assert!(warnings.is_empty(), "{warnings:?}");
        assert_eq!(file, FileConfig::default());
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn unknown_keys_are_ignored() {
        let home = temp_dir("unknown-key");
        std::fs::create_dir_all(home.join(".config/decide")).unwrap();
        std::fs::write(
            home.join(".config/decide/config.toml"),
            r#"
backend = "local"
future_key = "whatever"

[local]
weights = "/w"
also_future = 42
"#,
        )
        .unwrap();
        let (file, warnings) = load_from_disk(Some(home.to_str().unwrap()));
        assert!(warnings.is_empty(), "{warnings:?}");
        assert_eq!(file.backend, Some("local".to_string()));
        assert_eq!(file.local_weights, Some("/w".to_string()));
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn invalid_toml_is_a_warning_not_a_failure() {
        let home = temp_dir("invalid");
        std::fs::create_dir_all(home.join(".config/decide")).unwrap();
        std::fs::write(home.join(".config/decide/config.toml"), "not = [valid toml").unwrap();
        let (file, warnings) = load_from_disk(Some(home.to_str().unwrap()));
        assert_eq!(file, FileConfig::default());
        assert_eq!(warnings.len(), 1);
        assert!(warnings[0].contains("설정"), "{warnings:?}");
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn unreadable_path_is_a_warning_not_a_failure() {
        let home = temp_dir("unreadable");
        // 파일 자리에 디렉터리를 둬서 읽기가 실패하게 만든다.
        std::fs::create_dir_all(home.join(".config/decide/config.toml")).unwrap();
        let (file, warnings) = load_from_disk(Some(home.to_str().unwrap()));
        assert_eq!(file, FileConfig::default());
        assert_eq!(warnings.len(), 1);
        let _ = std::fs::remove_dir_all(&home);
    }
}
```

- [ ] **Step 3: 테스트 실행해 실패 확인**

Run: `cargo test --manifest-path crates/decide/Cargo.toml config:: 2>&1 | tail -30`
Expected: FAIL — `FileConfig`, `config_path`, `load_from_disk`가 아직 없어 컴파일 오류.

- [ ] **Step 4: 최소 구현 작성**

`crates/decide/src/config.rs` 테스트 모듈 위에 추가:

```rust
// decide 전체 설정 파일(~/.config/decide/config.toml): 백엔드 선택, TypeSafe 키,
// 로컬 가중치 경로, HuggingFace 환경을 담는다. 이 모듈은 환경변수를 모른다 —
// 환경변수와의 우선순위는 호출부(backend.rs, local/mod.rs)가 정한다.
use serde::Deserialize;
use std::path::PathBuf;

#[derive(Debug, Clone, Default, PartialEq)]
pub struct FileConfig {
    pub backend: Option<String>,
    pub typesafe_api_key: Option<String>,
    pub local_weights: Option<String>,
    pub local_hf_endpoint: Option<String>,
    pub local_hf_home: Option<String>,
    pub local_hf_token: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
struct Raw {
    backend: Option<String>,
    #[serde(default)]
    typesafe: RawTypesafe,
    #[serde(default)]
    local: RawLocal,
}

#[derive(Debug, Default, Deserialize)]
struct RawTypesafe {
    api_key: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
struct RawLocal {
    weights: Option<String>,
    hf_endpoint: Option<String>,
    hf_home: Option<String>,
    hf_token: Option<String>,
}

/// 공백만 있는 값은 미설정으로 본다(`CLEF_WEIGHTS` 처리와 같은 규칙).
fn nonblank(value: Option<String>) -> Option<String> {
    value.filter(|v| !v.trim().is_empty())
}

fn parse(text: &str) -> Result<FileConfig, String> {
    let raw: Raw = toml::from_str(text).map_err(|err| err.to_string())?;
    Ok(FileConfig {
        backend: nonblank(raw.backend),
        typesafe_api_key: nonblank(raw.typesafe.api_key),
        local_weights: nonblank(raw.local.weights),
        local_hf_endpoint: nonblank(raw.local.hf_endpoint),
        local_hf_home: nonblank(raw.local.hf_home),
        local_hf_token: nonblank(raw.local.hf_token),
    })
}

/// `~/.config/decide/config.toml`. `home`이 없거나 비면 None.
pub fn config_path(home: Option<&str>) -> Option<PathBuf> {
    home.filter(|home| !home.is_empty())
        .map(|home| PathBuf::from(home).join(".config/decide/config.toml"))
}

/// 파일을 읽어 파싱한다. 파일이 없으면 조용히 빈 값, 읽기·파싱 실패는 경고 문자열 하나와 빈 값.
pub fn load_from_disk(home: Option<&str>) -> (FileConfig, Vec<String>) {
    let Some(path) = config_path(home) else {
        return (FileConfig::default(), Vec::new());
    };
    let text = match std::fs::read_to_string(&path) {
        Ok(text) => text,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            return (FileConfig::default(), Vec::new());
        }
        Err(err) => {
            return (
                FileConfig::default(),
                vec![format!("설정을 읽지 못해 건너뜁니다 ({}): {err}", path.display())],
            );
        }
    };
    match parse(&text) {
        Ok(config) => (config, Vec::new()),
        Err(err) => (
            FileConfig::default(),
            vec![format!("설정을 TOML로 읽지 못해 건너뜁니다: {err}")],
        ),
    }
}
```

`crates/decide/src/lib.rs`에 `pub mod config;` 한 줄을 다른 `pub mod` 선언들과
나란히 추가한다(알파벳 순서 유지: `backend` 다음, `claude` 전).

- [ ] **Step 5: 테스트 실행해 통과 확인**

Run: `cargo test --manifest-path crates/decide/Cargo.toml config:: 2>&1 | tail -30`
Expected: PASS — 10개 테스트 모두 통과.

- [ ] **Step 6: 커밋**

```bash
git add crates/decide/Cargo.toml crates/decide/src/config.rs crates/decide/src/lib.rs
git commit -m "feat(config): ~/.config/decide/config.toml 읽기 모듈을 추가한다"
```

---

## Task 2: `Env::from_process` — 백엔드/키에 환경변수 우선순위 적용

**Files:**
- Modify: `crates/decide/src/backend.rs`

**Interfaces:**
- Consumes: `crate::config::load_from_disk(home: Option<&str>) -> (FileConfig, Vec<String>)`,
  `FileConfig.backend: Option<String>`, `FileConfig.typesafe_api_key: Option<String>` (Task 1).
- Produces: `Env::from_process()`의 동작 변경(시그니처는 그대로 `fn from_process() -> Self`).
  이후 태스크는 영향받지 않는다.

- [ ] **Step 1: 실패하는 테스트 먼저 — TOML 값 반영, 환경변수가 TOML을 이긴다**

`crates/decide/src/backend.rs`의 `#[cfg(test)] mod tests`에 추가(기존 테스트 모듈이
이미 있다 — 그 안에 추가한다):

```rust
    #[test]
    fn from_process_falls_back_to_config_file_when_env_is_unset() {
        let home = std::env::temp_dir().join(format!(
            "decide-backend-env-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(home.join(".config/decide")).unwrap();
        std::fs::write(
            home.join(".config/decide/config.toml"),
            "backend = \"local\"\n\n[typesafe]\napi_key = \"from-file\"\n",
        )
        .unwrap();

        std::env::remove_var("DECIDE_BACKEND");
        std::env::remove_var("TYPESAFE_API_KEY");
        std::env::set_var("HOME", &home);
        let env = Env::from_process();
        assert_eq!(env.backend, Some("local".to_string()));
        assert_eq!(env.api_key, Some("from-file".to_string()));

        // 환경변수가 있으면 같은 키의 TOML 값을 완전히 가린다.
        std::env::set_var("DECIDE_BACKEND", "typesafe");
        std::env::set_var("TYPESAFE_API_KEY", "from-env");
        let env = Env::from_process();
        assert_eq!(env.backend, Some("typesafe".to_string()));
        assert_eq!(env.api_key, Some("from-env".to_string()));

        std::env::remove_var("DECIDE_BACKEND");
        std::env::remove_var("TYPESAFE_API_KEY");
        std::env::remove_var("HOME");
        let _ = std::fs::remove_dir_all(&home);
    }
```

주의: 이 테스트는 `HOME`을 실제로 바꾼다. `cargo test`는 기본적으로 테스트를
병렬 실행하므로, 다른 테스트가 `HOME`에 의존하면 간섭할 수 있다 — 이 저장소에서
`HOME`을 바꾸는 테스트는 이 하나뿐임을 Step 5(기본 병렬 실행)의 결과로 확인한다
(기존 `claude.rs` 테스트들은 `HOME`이 아니라 자체 임시 디렉터리를 직접 함수에
넘기는 방식이라 간섭하지 않는다).

- [ ] **Step 2: 테스트 실행해 실패 확인**

Run: `cargo test --manifest-path crates/decide/Cargo.toml backend::tests::from_process_falls_back 2>&1 | tail -20`
Expected: FAIL — 지금의 `from_process`는 `config.toml`을 보지 않으므로
`env.backend`가 `None`이 되어 `assert_eq!(env.backend, Some("local"...))`에서 실패.

- [ ] **Step 3: 구현 변경**

`crates/decide/src/backend.rs`의 기존 `impl Env`를 아래로 교체. `std::env::var(...).ok()`는
변수가 아예 없으면 `None`을 주지만 빈 문자열로 설정돼 있으면 `Some("")`을 준다 —
Review Focus의 "환경변수가 빈 문자열인 경우"를 놓치지 않으려면 빈 문자열을
미설정으로 접어야 한다:

```rust
impl Env {
    pub fn from_process() -> Self {
        let (file, warnings) = crate::config::load_from_disk(std::env::var("HOME").ok().as_deref());
        for warning in &warnings {
            eprintln!("{warning}");
        }
        let env_var = |name: &str| std::env::var(name).ok().filter(|v| !v.trim().is_empty());
        Self {
            backend: env_var("DECIDE_BACKEND").or(file.backend),
            api_key: env_var("TYPESAFE_API_KEY").or(file.typesafe_api_key),
        }
    }
}
```

- [ ] **Step 4: 빈 환경변수 케이스 테스트 추가**

같은 테스트 함수 안, "환경변수가 있으면 TOML을 가린다" 검증 다음에 추가(아직
`let _ = std::fs::remove_dir_all(&home);`로 끝내기 전에 끼워 넣는다):

```rust
        // 빈 문자열 환경변수는 미설정과 같다 — TOML 값이 다시 보여야 한다.
        std::env::set_var("DECIDE_BACKEND", "");
        std::env::set_var("TYPESAFE_API_KEY", "");
        let env = Env::from_process();
        assert_eq!(env.backend, Some("local".to_string()));
        assert_eq!(env.api_key, Some("from-file".to_string()));
```

- [ ] **Step 5: 테스트 실행해 통과 확인**

Run: `cargo test --manifest-path crates/decide/Cargo.toml backend:: 2>&1 | tail -40`
Expected: PASS — 새 테스트 포함, 기존 `backend.rs` 테스트도 모두 통과(기존 테스트는
`HOME`을 건드리지 않으므로 영향 없음).

- [ ] **Step 6: 전체 테스트 스위트 회귀 확인**

Run: `cargo test --manifest-path crates/decide/Cargo.toml 2>&1 | tail -20`
Expected: PASS, 0 failed. (MLX 네이티브 빌드가 로컬 환경에서 깨져 있다면 이 저장소는
`cargo test`가 빌드 단계에서 실패할 수 있다 — 그 경우는 이 태스크의 범위가 아니다.
빌드 자체가 실패하면 Task 6에서 환경을 점검한다.)

- [ ] **Step 7: 커밋**

```bash
git add crates/decide/src/backend.rs
git commit -m "feat(config): Env::from_process가 config.toml을 환경변수 다음 우선순위로 읽는다"
```

---

## Task 3: `local/mod.rs` — 가중치 경로에 config.toml 반영

**Files:**
- Modify: `crates/decide/src/local/mod.rs`

**Interfaces:**
- Consumes: `crate::config::load_from_disk`, `FileConfig.local_weights: Option<String>`
  (Task 1). `pinned_weights_dir()`는 기존에도 있던 비공개 함수.
- Produces: `pinned_weights_dir() -> Option<PathBuf>`의 동작 변경(시그니처 동일).
  Task 4가 이어서 같은 파일의 `download_weights()`를 바꾼다.

- [ ] **Step 1: 실패하는 테스트 먼저**

`crates/decide/src/local/mod.rs`의 `#[cfg(test)] mod tests`에 추가:

```rust
    #[test]
    fn pinned_weights_dir_falls_back_to_config_file_when_env_is_unset() {
        let home = std::env::temp_dir().join(format!(
            "decide-local-weights-cfg-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(home.join(".config/decide")).unwrap();
        std::fs::write(
            home.join(".config/decide/config.toml"),
            "[local]\nweights = \"/from/file\"\n",
        )
        .unwrap();

        std::env::remove_var("CLEF_WEIGHTS");
        std::env::set_var("HOME", &home);
        assert_eq!(pinned_weights_dir(), Some(PathBuf::from("/from/file")));

        std::env::set_var("CLEF_WEIGHTS", "/from/env");
        assert_eq!(pinned_weights_dir(), Some(PathBuf::from("/from/env")));

        std::env::remove_var("CLEF_WEIGHTS");
        std::env::remove_var("HOME");
        let _ = std::fs::remove_dir_all(&home);
    }
```

이 테스트는 기존 `pinned_weights_dir_treats_blank_as_unset`와 같은 파일, 같은
`mod tests` 안에 있다 — 둘 다 `CLEF_WEIGHTS`/`HOME`을 건드리므로 `cargo test`의
기본 병렬 실행과 충돌할 수 있다. Step 3에서 `-- --test-threads=1`로 먼저 검증한다.

- [ ] **Step 2: 테스트 실행해 실패 확인**

Run: `cargo test --manifest-path crates/decide/Cargo.toml local::tests::pinned_weights_dir_falls_back -- --test-threads=1 2>&1 | tail -20`
Expected: FAIL — 지금의 `pinned_weights_dir`는 `config.toml`을 보지 않아 첫
`assert_eq!`에서 `None != Some("/from/file")`.

- [ ] **Step 3: 구현 변경**

`crates/decide/src/local/mod.rs`의 기존 함수를 교체:

```rust
/// `CLEF_WEIGHTS` 환경변수, 없으면 `config.toml`의 `[local].weights`(둘 다 공백만이면 미설정).
fn pinned_weights_dir() -> Option<PathBuf> {
    let env = std::env::var("CLEF_WEIGHTS")
        .ok()
        .filter(|v| !v.trim().is_empty())
        .map(|v| v.trim().to_string());
    let value = match env {
        Some(value) => Some(value),
        None => {
            let (file, warnings) = crate::config::load_from_disk(std::env::var("HOME").ok().as_deref());
            for warning in &warnings {
                eprintln!("{warning}");
            }
            file.local_weights
        }
    };
    value.map(PathBuf::from)
}
```

(`config::load_from_disk`가 이미 `nonblank`로 공백을 걸러 주므로 `file.local_weights`는
추가 트림이 필요 없다.)

- [ ] **Step 4: 테스트 실행해 통과 확인**

Run: `cargo test --manifest-path crates/decide/Cargo.toml local::tests::pinned_weights_dir -- --test-threads=1 2>&1 | tail -30`
Expected: PASS — `pinned_weights_dir_falls_back_to_config_file_when_env_is_unset`와
기존 `pinned_weights_dir_treats_blank_as_unset` 둘 다 통과.

경고를 두 번 내는 문제에 주의: 이 함수가 `load_from_disk`를 부르고 Task 2의
`Env::from_process`도 각자 부른다 — 둘 다 설정 파일이 깨졌을 때 경고를 낸다.
이는 받아들인다(각 소비자가 독립적으로 읽고 독립적으로 경고하는 것이 "환경변수
우선순위는 소비하는 쪽에 둔다"는 Architecture 결정과 맞는다). 중복 경고를 하나로
합치는 캐싱은 범위 밖이다 — 설계 문서에 없고, 두 호출 지점이 프로세스 생애에서
드물게만(백엔드 선택 1회, 가중치 로딩 1회) 실행되어 체감 비용이 없다.

- [ ] **Step 5: 커밋**

```bash
git add crates/decide/src/local/mod.rs
git commit -m "feat(config): CLEF_WEIGHTS가 없으면 config.toml의 [local].weights를 쓴다"
```

---

## Task 4: `local/mod.rs` — HuggingFace 엔드포인트/캐시/토큰 해석과 버그 수정

**Files:**
- Modify: `crates/decide/src/local/mod.rs`

**Interfaces:**
- Consumes: `crate::config::load_from_disk`, `FileConfig.local_hf_endpoint`,
  `FileConfig.local_hf_home`, `FileConfig.local_hf_token` (Task 1).
  `hf_hub::api::sync::ApiBuilder::from_env/with_endpoint/with_cache_dir/with_token/build`
  (외부 crate, 이미 의존성에 있음 — 지금까지 쓰지 않던 메서드).
- Produces: `download_weights()`의 동작 변경(시그니처 동일, `Result<Weights, String>`).
  `LocalTokenizer::load()`는 바꾸지 않는다(토크나이저는 공개 저장소라 엔드포인트
  재정의가 급하지 않다 — Review Focus에도 없고 설계 문서도 `download_weights`만
  대상으로 한다).

- [ ] **Step 1: 실패하는 테스트 먼저 — 해석 함수의 우선순위**

`download_weights()`가 실제로 네트워크를 치므로 그 자체를 테스트하지 않는다.
대신 "환경변수 > config.toml" 우선순위를 뽑아내는 순수 함수 `resolved_hf()`를
새로 만들고 그 함수를 테스트한다. `crates/decide/src/local/mod.rs`의
`#[cfg(test)] mod tests`에 추가:

```rust
    #[test]
    fn resolved_hf_prefers_env_over_config_file_per_field() {
        let home = std::env::temp_dir().join(format!(
            "decide-local-hf-cfg-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(home.join(".config/decide")).unwrap();
        std::fs::write(
            home.join(".config/decide/config.toml"),
            "[local]\nhf_endpoint = \"https://file.example\"\nhf_home = \"/file/cache\"\nhf_token = \"file-token\"\n",
        )
        .unwrap();

        std::env::remove_var("HF_ENDPOINT");
        std::env::remove_var("HF_HOME");
        std::env::remove_var("HF_TOKEN");
        std::env::set_var("HOME", &home);

        // 파일만 있을 때: 셋 다 파일 값.
        let resolved = resolved_hf();
        assert_eq!(resolved.endpoint, Some("https://file.example".to_string()));
        assert_eq!(resolved.home, Some("/file/cache".to_string()));
        assert_eq!(resolved.token, Some("file-token".to_string()));

        // endpoint만 환경변수로 덮으면 나머지 둘은 그대로 파일 값(필드별 독립).
        std::env::set_var("HF_ENDPOINT", "https://env.example");
        let resolved = resolved_hf();
        assert_eq!(resolved.endpoint, Some("https://env.example".to_string()));
        assert_eq!(resolved.home, Some("/file/cache".to_string()));
        assert_eq!(resolved.token, Some("file-token".to_string()));

        std::env::remove_var("HF_ENDPOINT");
        std::env::remove_var("HOME");
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn resolved_hf_is_all_none_without_env_or_file() {
        let home = std::env::temp_dir().join(format!(
            "decide-local-hf-empty-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&home).unwrap();
        std::env::remove_var("HF_ENDPOINT");
        std::env::remove_var("HF_HOME");
        std::env::remove_var("HF_TOKEN");
        std::env::set_var("HOME", &home);
        let resolved = resolved_hf();
        assert_eq!(resolved.endpoint, None);
        assert_eq!(resolved.home, None);
        assert_eq!(resolved.token, None);
        std::env::remove_var("HOME");
        let _ = std::fs::remove_dir_all(&home);
    }
```

- [ ] **Step 2: 테스트 실행해 실패 확인**

Run: `cargo test --manifest-path crates/decide/Cargo.toml local::tests::resolved_hf -- --test-threads=1 2>&1 | tail -20`
Expected: FAIL — `resolved_hf`가 아직 없어 컴파일 오류.

- [ ] **Step 3: 구현 — `resolved_hf()`와 `download_weights()` 교체**

`crates/decide/src/local/mod.rs`에 `pinned_weights_dir` 함수 바로 아래 추가:

```rust
struct ResolvedHf {
    endpoint: Option<String>,
    home: Option<String>,
    token: Option<String>,
}

/// HF_ENDPOINT/HF_HOME/HF_TOKEN 환경변수, 없으면 config.toml의 [local] 쪽 값. 필드별 독립.
fn resolved_hf() -> ResolvedHf {
    let env = |name: &str| std::env::var(name).ok().filter(|v| !v.trim().is_empty());
    let (file, warnings) = crate::config::load_from_disk(std::env::var("HOME").ok().as_deref());
    for warning in &warnings {
        eprintln!("{warning}");
    }
    ResolvedHf {
        endpoint: env("HF_ENDPOINT").or(file.local_hf_endpoint),
        home: env("HF_HOME").or(file.local_hf_home),
        token: env("HF_TOKEN").or(file.local_hf_token),
    }
}
```

이제 `download_weights()`를 교체(기존 `hf_hub::api::sync::Api::new()` 한 줄짜리
초기화를 대체):

```rust
/// HuggingFace 캐시(`~/.cache/huggingface/hub`, `HF_HOME`/config.toml로 바꿀 수 있다)에서
/// 받는다. 이미 있으면 다운로드 없이 그 경로를 쓴다. 첫 사용에는 약 10GB를 받는다.
fn download_weights() -> Result<Weights, String> {
    let resolved = resolved_hf();
    // `ApiBuilder::from_env()`는 HF_HOME/HF_ENDPOINT 환경변수를 반영한다.
    // `Api::new()`는 반영하지 않는다 — 지금까지 이 환경변수들이 전혀 적용되지 않던 버그.
    let mut builder = hf_hub::api::sync::ApiBuilder::from_env();
    if let Some(endpoint) = resolved.endpoint {
        builder = builder.with_endpoint(endpoint);
    }
    if let Some(home) = resolved.home {
        builder = builder.with_cache_dir(PathBuf::from(home).join("hub"));
    }
    if let Some(token) = resolved.token {
        builder = builder.with_token(Some(token));
    }
    let api = builder
        .build()
        .map_err(|err| format!("HuggingFace API 초기화에 실패했습니다: {err}"))?;
    let repo = api.model(MLX_REPO.to_string());
    let fetch = |name: &str| {
        repo.get(name)
            .map_err(|err| format!("{MLX_REPO}/{name} 다운로드에 실패했습니다: {err}"))
    };
    let config = fetch("config.json")?;
    let index = fetch("model.safetensors.index.json")?;
    let head = fetch("joint_head.safetensors")?;
    let shards = mlx_backbone::shard_names(&index)?
        .iter()
        .map(|name| fetch(name))
        .collect::<Result<Vec<_>, _>>()?;
    Ok((mlx_backbone::MlxFiles { config, shards }, head))
}
```

`with_cache_dir`에 `.join("hub")`를 붙이는 이유: `hf_hub::Cache::from_env()`가
`HF_HOME`을 읽을 때도 `path.push("hub")`를 하므로(저장소 조사에서 확인), config.toml의
`hf_home`도 같은 하위구조를 맞춘다 — 그렇지 않으면 `HF_HOME` 환경변수와 config.toml의
`hf_home`이 같은 값을 넣어도 실제 캐시 경로가 달라진다.

- [ ] **Step 4: 테스트 실행해 통과 확인**

Run: `cargo test --manifest-path crates/decide/Cargo.toml local::tests::resolved_hf -- --test-threads=1 2>&1 | tail -30`
Expected: PASS — 두 테스트 모두 통과.

- [ ] **Step 5: 전체 로컬 모듈 테스트 회귀 확인(순차 실행)**

Run: `cargo test --manifest-path crates/decide/Cargo.toml local:: -- --test-threads=1 2>&1 | tail -60`
Expected: PASS. `ensure_weights_downloads_when_cache_empty`처럼 실제 네트워크가
필요한 `#[ignore]` 테스트는 건드리지 않았으므로 기본 실행에서 스킵된다(기존과 동일).

- [ ] **Step 6: 커밋**

```bash
git add crates/decide/src/local/mod.rs
git commit -m "fix(local): HF_ENDPOINT/HF_HOME/HF_TOKEN과 config.toml을 가중치 다운로드에 반영한다"
```

---

## Task 5: 문서 갱신

**Files:**
- Modify: `README.md`
- Modify: `CLAUDE.md`

**Interfaces:**
- Consumes: Task 1–4의 최종 동작(설정 파일 경로, 스키마, 우선순위).
- Produces: 없음(문서만).

- [ ] **Step 1: README의 환경 변수 문장 뒤에 설정 파일 절 추가**

`README.md:274`는 다음 한 줄짜리 문단이다(`grep -n "환경 변수는 " README.md`로
먼저 그 줄이 아직 274번째인지 확인한다 — 이전 태스크의 수정으로 줄 번호가
밀렸을 수 있다):

```text
환경 변수는 `DECIDE_BACKEND`(`typesafe` 또는 `local`), `TYPESAFE_API_KEY`, `CLEF_WEIGHTS`(로컬 가중치 디렉터리)다. 현재 버전은 0.5.0이다. formula는 GitHub Release의 사전 빌드 arm64 바이너리(`decide`와 GPU 커널 묶음 `mlx.metallib`)를 그대로 설치한다.
```

이 줄 바로 다음에 새 문단으로 아래 내용을 끼워 넣는다(이 문단은 "현재 버전은
0.5.0이다"로 시작하는 다음 문장과는 무관한 별도 주제이므로, 같은 줄을 고치지
말고 그 줄 뒤에 빈 줄 하나를 두고 새 문단을 추가한다):

README에 추가할 본문은 아래 두 조각이다. 먼저 안내 문단:

```text
같은 값을 `~/.config/decide/config.toml`로도 지정할 수 있다. 환경변수가 있으면
같은 키의 TOML 값은 무시한다. 파일이 없으면 조용히 건너뛰고, 읽기나 TOML 파싱에
실패하면 stderr에 경고만 내고 그 레이어 없이 계속 진행한다(`decide`를 멈추지
않는다).
```

그다음 TOML 코드 펜스(README 본문에 그대로 ` ```toml ` 블록으로 들어간다):

```toml
backend = "local"        # DECIDE_BACKEND와 같은 뜻

[typesafe]
api_key = "sk-..."       # TYPESAFE_API_KEY와 같은 뜻

[local]
weights = "/path/to/weights"   # CLEF_WEIGHTS와 같은 뜻
hf_endpoint = "https://nexus.example/hf-proxy"  # HF_ENDPOINT와 같은 뜻
hf_home = "/path/to/cache"      # HF_HOME과 같은 뜻
hf_token = "hf_..."             # HF_TOKEN과 같은 뜻(hf-hub 자체는 이 환경변수를 지원하지 않는다)
```

- [ ] **Step 2: CLAUDE.md의 `local/mod.rs` 설명에 한 문장 추가**

`CLAUDE.md`는 영어로 쓰여 있다. `CLAUDE.md:110`(`grep -n "local/mod.rs. resolves weights" CLAUDE.md`로
확인)의 문장은 다음과 같다:

```text
- `local/mod.rs` resolves weights (`CLEF_WEIGHTS` env override, else the HuggingFace
  cache; `ensure_weights`/`resolve_pinned`/`download_weights`), lazily builds the backbone + joint
```

`env override, else the HuggingFace` 바로 다음(같은 줄, "else the HuggingFace"
앞)에 config.toml 폴백을 끼워 넣어 아래처럼 고친다:

```text
- `local/mod.rs` resolves weights (`CLEF_WEIGHTS` env override, else
  `~/.config/decide/config.toml`'s `[local].weights`, else the HuggingFace
  cache; `ensure_weights`/`resolve_pinned`/`download_weights`), lazily builds the backbone + joint
```

같은 문단에 HuggingFace 환경(엔드포인트/캐시/토큰)도 config.toml로 지정할 수
있다는 점을 한 문장 더 추가한다. `CLAUDE.md:114`는 지금:

```text
  `postprocess::to_answer`). `local/mlx_backbone.rs` is the MLX port of the Qwen3.5
```

`postprocess::to_answer`).와 `local/mlx_backbone.rs` 사이에 새 문장을 끼워 넣는다
(bullet을 나누지 않고 같은 bullet 안에 이어 쓴다):

```text
  `postprocess::to_answer`). `download_weights` also honors
  `HF_ENDPOINT`/`HF_HOME`/`HF_TOKEN` env vars (via `ApiBuilder::from_env`,
  previously unused — `Api::new()` ignored them) or the same keys under
  `config.toml`'s `[local]` section when the env var is unset; README
  documents the full schema. `local/mlx_backbone.rs` is the MLX port of the Qwen3.5
```

- [ ] **Step 3: 커밋**

```bash
git add README.md CLAUDE.md
git commit -m "docs(config): config.toml 설정 파일을 README와 CLAUDE.md에 반영한다"
```

---

## Task 6: 전체 검증

**Files:** 없음(검증만).

**Interfaces:** 없음.

- [ ] **Step 1: 전체 테스트 스위트**

Run: `cargo test --manifest-path crates/decide/Cargo.toml 2>&1 | tail -40`
Expected: PASS, 0 failed.

로컬 환경에 전체 Xcode(Metal Toolchain)가 없으면 MLX 네이티브 빌드 단계에서
`cargo test` 자체가 실패할 수 있다(이 저장소의 알려진 환경 문제, 이 플랜과
무관). 그 경우는 사용자에게 "Metal Toolchain 미설치로 빌드가 안 돼 이 태스크의
테스트를 실행하지 못했다"고 보고하고, 환경 복구는 별도 작업으로 넘긴다 — 이
플랜의 범위가 아니다.

- [ ] **Step 2: `cargo fmt --check`로 서식 확인**

Run: `cargo fmt --manifest-path crates/decide/Cargo.toml --check`
Expected: 변경한 파일(`config.rs`, `backend.rs`, `local/mod.rs`)에 서식 차이 없음.
차이가 있으면 `cargo fmt --manifest-path crates/decide/Cargo.toml`을 실행하되,
`git diff`로 이 플랜이 건드리지 않은 파일(예: 과거에도 포맷되지 않은 상태였던
`tests/cli.rs`, `daemon.rs`)까지 같이 바뀌지 않는지 확인한다 — 바뀐다면 그 hunk만
`git checkout -- <file>`로 되돌린다(컨텍스트 노트의 과거 기록: "baseline이 `cargo
fmt --check` 미준수"였다).

- [ ] **Step 3: 수동 스모크 — TOML만으로 백엔드가 바뀌는지**

```bash
TMP=$(mktemp -d)
mkdir -p "$TMP/.config/decide"
cat > "$TMP/.config/decide/config.toml" <<'EOF'
backend = "local"
EOF
HOME="$TMP" DECIDE_BACKEND= TYPESAFE_API_KEY= /opt/homebrew/bin/decide mcp <<'EOF2' 2>&1 | head -5
{"jsonrpc":"2.0","id":1,"method":"tools/list"}
EOF2
rm -rf "$TMP"
```

Expected: 오류 없이 `tools/list` 응답이 온다(이 스모크는 백엔드 선택이 실제로
TOML을 읽었는지보다는, `config.toml`이 있어도 `decide mcp`가 정상 기동하는지를
확인하는 것이다 — `routing.backend`를 보려면 `decide` 도구를 호출해야 하는데
로컬 백엔드는 가중치가 없으면 하드 에러이므로, 이 스모크는 기동 자체만 본다).
이 스텝은 자동화된 테스트가 아니라 수동 확인이다 — 결과를 사용자에게 보고한다.

- [ ] **Step 4: 사용자에게 최종 보고**

이 태스크에는 커밋이 없다(검증만). 다음 단계로 PR 생성 여부를 사용자에게 묻는다.

---

## Self-Review 체크 결과 (writing-plans 요구사항)

- **Spec coverage:** 설계 문서의 "결정"(우선순위, 오류 처리, 저장소 레이어 없음) →
  Task 2 테스트. "스키마" → Task 1. "구성 변경"의 `config.rs`/`Env::from_process`/
  `local/mod.rs`/`Cargo.toml`/`lib.rs` → Task 1–4. "테스트" 절의 네 항목 → Task
  1(파일 없음/왕복/깨짐/모르는키), Task 2(env 우선), Task 4(resolved_hf 우선순위).
  "문서" → Task 5. "범위 밖"(저장소 레이어, gates.json과 통합, install이 생성) →
  어떤 태스크도 이걸 하지 않음, 명시적으로 Global Constraints에 반영.
- **Placeholder scan:** 모든 스텝에 실행 가능한 코드/명령이 있다. "TBD"/"나중에"
  없음.
- **Type consistency:** `FileConfig` 필드명(`backend`, `typesafe_api_key`,
  `local_weights`, `local_hf_endpoint`, `local_hf_home`, `local_hf_token`)이
  Task 1 정의부터 Task 2/3/4 소비부까지 동일하게 쓰였다. `ResolvedHf`
  (`endpoint`/`home`/`token`)는 Task 4 안에서만 쓰이고 다른 태스크가 그 이름을
  참조하지 않는다.
- **Review Focus 반영:** 빈 섹션(Task 1 `empty_sections_are_not_an_error`),
  빈 문자열 값(Task 1 `blank_values_are_treated_as_unset`), 빈 환경변수(Task 2
  Step 4), `~/.config/decide/` 미생성(Task 1
  `missing_config_subdir_is_also_silently_empty`), 필드별 독립 적용(Task 4
  `resolved_hf_prefers_env_over_config_file_per_field`) — 다섯 항목 모두 소유
  태스크에 테스트가 있다.
