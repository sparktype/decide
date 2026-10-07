// decide 전체 설정 파일(~/.config/decide/config.toml): 백엔드 선택, TypeSafe 키와 주소,
// 로컬 서버 주소를 담는다. 이 모듈은 환경변수를 모른다 —
// 환경변수와의 우선순위는 호출부(backend.rs)가 정한다.
use serde::Deserialize;
use std::path::PathBuf;

#[derive(Debug, Clone, Default, PartialEq)]
pub struct FileConfig {
    pub backend: Option<String>,
    pub typesafe_api_key: Option<String>,
    pub typesafe_url: Option<String>,
    pub local_url: Option<String>,
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
    url: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
struct RawLocal {
    url: Option<String>,
}

/// 공백만 있는 값은 미설정으로 본다.
fn nonblank(value: Option<String>) -> Option<String> {
    value.filter(|v| !v.trim().is_empty())
}

fn parse(text: &str) -> Result<FileConfig, String> {
    let raw: Raw = toml::from_str(text).map_err(|err| err.to_string())?;
    Ok(FileConfig {
        backend: nonblank(raw.backend),
        typesafe_api_key: nonblank(raw.typesafe.api_key),
        typesafe_url: nonblank(raw.typesafe.url),
        local_url: nonblank(raw.local.url),
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
                vec![format!(
                    "설정을 읽지 못해 건너뜁니다 ({}): {err}",
                    path.display()
                )],
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
url = "http://127.0.0.1:8009/v1/systemone"

[local]
url = "http://127.0.0.1:8009/v1/systemone"
"#,
        )
        .unwrap();
        let (file, warnings) = load_from_disk(Some(home.to_str().unwrap()));
        assert!(warnings.is_empty(), "{warnings:?}");
        assert_eq!(file.backend, Some("local".to_string()));
        assert_eq!(file.typesafe_api_key, Some("sk-test".to_string()));
        assert_eq!(file.typesafe_url, Some("http://127.0.0.1:8009/v1/systemone".to_string()));
        assert_eq!(file.local_url, Some("http://127.0.0.1:8009/v1/systemone".to_string()));
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
url = "   "
"#,
        )
        .unwrap();
        let (file, warnings) = load_from_disk(Some(home.to_str().unwrap()));
        assert!(warnings.is_empty(), "{warnings:?}");
        assert_eq!(file.backend, None);
        assert_eq!(file.local_url, None);
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
url = "http://h/v1/systemone"
weights = "/removed/key"
also_future = 42
"#,
        )
        .unwrap();
        let (file, warnings) = load_from_disk(Some(home.to_str().unwrap()));
        assert!(warnings.is_empty(), "{warnings:?}");
        assert_eq!(file.backend, Some("local".to_string()));
        assert_eq!(file.local_url, Some("http://h/v1/systemone".to_string()));
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
