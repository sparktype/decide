mod backbone;
mod joint_head;
mod postprocess;
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

fn clef_weights_is_set() -> bool {
    std::env::var("CLEF_WEIGHTS")
        .map(|value| !value.trim().is_empty())
        .unwrap_or(false)
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

pub fn ensure_weights() -> Result<(PathBuf, PathBuf), String> {
    let dir = weights_dir()?;
    let backbone_path = dir.join("clef-flash.Q4_K_M.gguf");
    let head_path = dir.join("joint_head.safetensors");
    if backbone_path.exists() && head_path.exists() {
        return Ok((backbone_path, head_path));
    }
    if clef_weights_is_set() {
        return Err(format!(
            "CLEF_WEIGHTS={}에서 가중치 파일을 찾을 수 없습니다 (clef-flash.Q4_K_M.gguf, joint_head.safetensors 필요)",
            dir.display()
        ));
    }
    download_weights(&dir)
}

fn download_weights(dir: &std::path::Path) -> Result<(PathBuf, PathBuf), String> {
    use hf_hub::api::sync::Api;
    std::fs::create_dir_all(dir).map_err(|err| format!("가중치 디렉터리를 만들 수 없습니다: {err}"))?;
    let api = Api::new().map_err(|err| format!("HuggingFace API 초기화에 실패했습니다: {err}"))?;

    let backbone_repo = api.model("prithivMLmods/clef-flash-GGUF".to_string());
    let backbone_src = backbone_repo
        .get("clef-flash.Q4_K_M.gguf")
        .map_err(|err| format!("백본 GGUF 다운로드에 실패했습니다: {err}"))?;
    let backbone_dst = dir.join("clef-flash.Q4_K_M.gguf");
    std::fs::copy(&backbone_src, &backbone_dst).map_err(|err| format!("백본 파일 복사에 실패했습니다: {err}"))?;

    let head_repo = api.model("Cloudflare/clef-flash".to_string());
    let head_src = head_repo
        .get("joint_head.safetensors")
        .map_err(|err| format!("joint_head 다운로드에 실패했습니다: {err}"))?;
    let head_dst = dir.join("joint_head.safetensors");
    std::fs::copy(&head_src, &head_dst).map_err(|err| format!("joint_head 파일 복사에 실패했습니다: {err}"))?;

    Ok((backbone_dst, head_dst))
}

pub struct LocalTokenizer {
    inner: tokenizers::Tokenizer,
}

impl LocalTokenizer {
    pub fn load() -> Result<Self, String> {
        let api = hf_hub::api::sync::Api::new()
            .map_err(|err| format!("HuggingFace API 초기화에 실패했습니다: {err}"))?;
        let repo = api.model("Cloudflare/clef-flash".to_string());
        let tokenizer_path = repo
            .get("tokenizer.json")
            .map_err(|err| format!("tokenizer.json 다운로드에 실패했습니다: {err}"))?;
        let inner = tokenizers::Tokenizer::from_file(&tokenizer_path)
            .map_err(|err| format!("토크나이저 로드에 실패했습니다: {err}"))?;
        Ok(Self { inner })
    }

    pub fn encode(&self, text: &str) -> Result<(Vec<u32>, Vec<(usize, usize)>), String> {
        let encoding = self
            .inner
            .encode(text, true)
            .map_err(|err| format!("토큰화에 실패했습니다: {err}"))?;
        Ok((encoding.get_ids().to_vec(), encoding.get_offsets().to_vec()))
    }
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

    #[test]
    fn ensure_weights_finds_files_in_clef_weights_dir() {
        let dir = std::env::temp_dir().join(format!("clef-weights-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("clef-flash.Q4_K_M.gguf"), b"fake").unwrap();
        std::fs::write(dir.join("joint_head.safetensors"), b"fake").unwrap();
        std::env::set_var("CLEF_WEIGHTS", &dir);

        let (backbone_path, head_path) = ensure_weights().unwrap();
        assert_eq!(backbone_path, dir.join("clef-flash.Q4_K_M.gguf"));
        assert_eq!(head_path, dir.join("joint_head.safetensors"));

        std::env::remove_var("CLEF_WEIGHTS");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn ensure_weights_errors_when_files_missing() {
        let dir = std::env::temp_dir().join(format!("clef-weights-empty-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::env::set_var("CLEF_WEIGHTS", &dir);

        let err = ensure_weights().unwrap_err();
        assert!(err.contains("찾을 수 없습니다") || err.contains("다운로드"));

        std::env::remove_var("CLEF_WEIGHTS");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn ensure_weights_treats_empty_clef_weights_as_unset() {
        // CLEF_WEIGHTS="" (설정은 됐지만 빈 값)은 weights_dir()의 정의상 "미설정"과
        // 같아야 한다. HOME을 가짜 캐시 디렉터리로 돌려서, 하드 에러 분기를 타지
        // 않고 (네트워크 호출 없이) 기존 파일을 그대로 찾아내는지로 "다운로드 경로로
        // 빠진다"는 분기 선택을 관찰한다.
        let original_home = std::env::var("HOME").ok();
        let fake_home = std::env::temp_dir().join(format!("clef-fake-home-{}", std::process::id()));
        let fake_hub = fake_home.join(".cache").join("huggingface").join("hub");
        std::fs::create_dir_all(&fake_hub).unwrap();
        std::fs::write(fake_hub.join("clef-flash.Q4_K_M.gguf"), b"fake").unwrap();
        std::fs::write(fake_hub.join("joint_head.safetensors"), b"fake").unwrap();

        std::env::set_var("HOME", &fake_home);
        std::env::set_var("CLEF_WEIGHTS", "");

        let result = ensure_weights();

        std::env::remove_var("CLEF_WEIGHTS");
        match original_home {
            Some(home) => std::env::set_var("HOME", home),
            None => std::env::remove_var("HOME"),
        }
        std::fs::remove_dir_all(&fake_home).ok();

        let (backbone_path, head_path) = result.unwrap();
        assert_eq!(backbone_path, fake_hub.join("clef-flash.Q4_K_M.gguf"));
        assert_eq!(head_path, fake_hub.join("joint_head.safetensors"));
    }

    #[test]
    #[ignore] // 실제 네트워크로 HuggingFace에서 수 GB를 받는다 — CI 기본 실행에서 제외
    fn ensure_weights_downloads_when_cache_empty() {
        std::env::remove_var("CLEF_WEIGHTS");
        let (backbone_path, head_path) = ensure_weights().unwrap();
        assert!(backbone_path.exists());
        assert!(head_path.exists());
    }
}
