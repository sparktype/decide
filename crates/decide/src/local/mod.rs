mod backbone;
mod tokenizer;

use crate::protocol::Question;
use serde_json::Value;
use std::path::PathBuf;

pub const DEFAULT_URL: &str = "http://127.0.0.1:8765/v1/systemone";
pub const CONNECT_HINT: &str = "jev-style serve가 실행 중인지 확인하세요";
pub const NOT_READY: &str = "로컬 백엔드가 아직 준비되지 않았습니다";

pub fn url() -> String {
    std::env::var("DECIDE_LOCAL_URL")
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| DEFAULT_URL.to_string())
}

pub fn weights_dir() -> Result<PathBuf, String> {
    if let Ok(dir) = std::env::var("CLEF_WEIGHTS") {
        let trimmed = dir.trim();
        if !trimmed.is_empty() {
            return Ok(PathBuf::from(trimmed));
        }
    }
    let home = std::env::var("HOME").map_err(|_| "HOME 환경변수를 읽을 수 없습니다".to_string())?;
    Ok(PathBuf::from(home)
        .join(".cache")
        .join("huggingface")
        .join("hub"))
}

pub fn infer(_state: &str, _question: &Question) -> Result<Value, String> {
    Err(NOT_READY.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn weights_dir_prefers_env_override() {
        std::env::set_var("CLEF_WEIGHTS", "/tmp/clef-weights-test");
        assert_eq!(weights_dir().unwrap(), PathBuf::from("/tmp/clef-weights-test"));
        std::env::remove_var("CLEF_WEIGHTS");
    }

    #[test]
    fn weights_dir_falls_back_to_hf_cache() {
        std::env::remove_var("CLEF_WEIGHTS");
        let dir = weights_dir().unwrap();
        assert!(dir.ends_with(".cache/huggingface/hub") || dir.to_string_lossy().contains(".cache"));
    }

    #[test]
    fn infer_is_not_ready_before_the_gate() {
        let question = crate::protocol::Question::Noul {
            instructions: "참인가?".into(),
        };
        assert_eq!(infer("상태", &question).unwrap_err(), NOT_READY);
    }
}
