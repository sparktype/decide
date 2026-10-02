mod backbone;
mod joint_head;
mod postprocess;
// parity 테스트(`crates/decide/tests/parity.rs`)가 `decide::local::tokenizer::spans`로
// 실제 토큰 스팬을 검증해야 하므로 `pub`으로 재노출한다.
pub mod tokenizer;

use crate::protocol::Question;
use serde_json::Value;
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};

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
    if clef_weights_is_set() {
        let dir = std::env::var("CLEF_WEIGHTS").expect("clef_weights_is_set()가 true이면 존재한다");
        return Ok(PathBuf::from(dir.trim().to_string()));
    }
    let home = std::env::var("HOME").map_err(|_| "HOME 환경변수를 읽을 수 없습니다".to_string())?;
    Ok(PathBuf::from(home)
        .join(".cache")
        .join("huggingface")
        .join("hub"))
}

struct Runtime {
    backbone: Mutex<backbone::Backbone>,
    joint_head: joint_head::JointHead,
    tokenizer: LocalTokenizer,
}

static RUNTIME: OnceLock<Result<Runtime, String>> = OnceLock::new();

fn runtime() -> &'static Result<Runtime, String> {
    RUNTIME.get_or_init(|| {
        let (backbone_path, head_path) = ensure_weights()?;
        let backbone = backbone::Backbone::from_gguf_path(&backbone_path)?;
        let hidden_size = backbone.hidden_size();
        let joint_head = joint_head::JointHead::load(&head_path, hidden_size)?;
        let tokenizer = LocalTokenizer::load()?;
        Ok(Runtime {
            backbone: Mutex::new(backbone),
            joint_head,
            tokenizer,
        })
    })
}

/// `question`에 대한 원시 로짓(softmax/sigmoid 이전), 옵션 라벨과 함께 —
/// `infer`와 parity 테스트가 공유하는 추론 경로. 리뷰에서 발견된 버그
/// 재발 방지: `JointHead::score`가 돌려주는 `(label, logit)` 쌍을 그대로
/// 전달한다 — 라벨 없는 `Vec<f32>`로 바꿔 돌리면 호출부가 다시 "이 벡터와
/// Question의 options/criteria가 같은 순서"라고 암묵적으로 가정하게 될
/// 위험이 있다.
fn score(runtime: &Runtime, state: &str, question: &Question) -> Result<Vec<(String, f32)>, String> {
    let (token_ids, offsets) = runtime.tokenizer.encode(state, question)?;
    let (question_span, option_spans) = tokenizer::spans(state, question);
    let device = candle_core::Device::Cpu;
    let input_ids = candle_core::Tensor::new(token_ids.as_slice(), &device)
        .map_err(|err| err.to_string())?
        .unsqueeze(0)
        .map_err(|err| err.to_string())?;
    let (hidden_states, output_embeddings) = {
        let mut backbone = runtime.backbone.lock().map_err(|_| "백본 락 획득에 실패했습니다".to_string())?;
        let hidden_states = backbone.hidden_states(&input_ids)?;
        let output_embeddings = backbone.output_embeddings().clone();
        (hidden_states, output_embeddings)
    };
    let question_type = tokenizer::question_type_id(question);
    runtime.joint_head.score(
        &hidden_states,
        &input_ids,
        question_type,
        &question_span,
        &option_spans,
        &offsets,
        &output_embeddings,
    )
}

pub fn infer(state: &str, question: &Question) -> Result<Value, String> {
    let runtime = match runtime() {
        Ok(runtime) => runtime,
        Err(err) => return Err(err.clone()),
    };
    let labeled_logits = score(runtime, state, question)?;
    Ok(postprocess::to_answer(question, &labeled_logits))
}

/// parity 테스트 전용 — postprocess(softmax/sigmoid) 이전의 원시 로짓을
/// `(옵션 label, 로짓)` 쌍으로 그대로 돌려준다. Python 오라클
/// (`scripts/clef_flash_oracle.py`)이 저장하는 `logits`는 위치 기반
/// 배열이므로, 호출부(parity.rs)가 `tokenizer::option_labels`로 같은
/// 순서 규칙을 적용해 오라클 쪽에도 라벨을 붙인 뒤 라벨 기준으로
/// 비교해야 한다.
#[cfg(feature = "parity")]
pub fn raw_logits(state: &str, question: &Question) -> Result<Vec<(String, f32)>, String> {
    let runtime = match runtime() {
        Ok(runtime) => runtime,
        Err(err) => return Err(err.clone()),
    };
    score(runtime, state, question)
}

enum WeightsDecision {
    Found(PathBuf, PathBuf),
    HardError(String),
    NeedsDownload,
}

fn resolve_weights(dir: &std::path::Path) -> WeightsDecision {
    let backbone_path = dir.join("clef-flash.Q4_K_M.gguf");
    let head_path = dir.join("joint_head.safetensors");
    if backbone_path.exists() && head_path.exists() {
        return WeightsDecision::Found(backbone_path, head_path);
    }
    if clef_weights_is_set() {
        return WeightsDecision::HardError(format!(
            "CLEF_WEIGHTS={}에서 가중치 파일을 찾을 수 없습니다 (clef-flash.Q4_K_M.gguf, joint_head.safetensors 필요)",
            dir.display()
        ));
    }
    WeightsDecision::NeedsDownload
}

pub fn ensure_weights() -> Result<(PathBuf, PathBuf), String> {
    let dir = weights_dir()?;
    match resolve_weights(&dir) {
        WeightsDecision::Found(backbone_path, head_path) => Ok((backbone_path, head_path)),
        WeightsDecision::HardError(err) => Err(err),
        WeightsDecision::NeedsDownload => download_weights(&dir),
    }
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

    /// 세그먼트 하나를 토큰화한다. `add_special_tokens`는 항상 `false`다 —
    /// 실제 `encode_record`가 모든 세그먼트를 `add_special_tokens=False`로
    /// 토큰화하고, 특수 토큰(`<|im_start|>` 등)은 세그먼트 텍스트 자체에
    /// 리터럴로 포함시킨다(모델이 그 리터럴을 단일 특수 토큰으로 병합하도록
    /// 어휘에 등록되어 있다). 여기서 `true`를 주면 모델이 학습하지 않은
    /// BOS/EOS가 끼어들어 Task 7에서 실측한 토큰 ID 시퀀스와 달라진다.
    fn encode_segment(&self, text: &str) -> Result<(Vec<u32>, Vec<(usize, usize)>), String> {
        let encoding = self
            .inner
            .encode(text, false)
            .map_err(|err| format!("토큰화에 실패했습니다: {err}"))?;
        Ok((encoding.get_ids().to_vec(), encoding.get_offsets().to_vec()))
    }

    /// `tokenizer::segments()`가 내놓는 세그먼트들을 각각 개별적으로
    /// 토큰화해 이어붙인다(세그먼트 경계를 넘는 BPE 병합을 막기 위해 —
    /// `tokenizer.rs` 모듈 상단 주석 참고). 반환하는 오프셋은 세그먼트별
    /// 바이트 오프셋을 전체 조립 텍스트 기준으로 이동(shift)한 값이라
    /// `tokenizer::spans()`가 돌려주는 문자(바이트) 스팬과 같은 좌표계를
    /// 쓴다.
    pub fn encode(
        &self,
        state: &str,
        question: &crate::protocol::Question,
    ) -> Result<(Vec<u32>, Vec<(usize, usize)>), String> {
        let mut ids = Vec::new();
        let mut offsets = Vec::new();
        let mut cursor = 0usize;
        for seg in tokenizer::segments(state, question) {
            if let tokenizer::Segment::Text(part) = seg {
                let (seg_ids, seg_offsets) = self.encode_segment(&part)?;
                ids.extend(seg_ids);
                offsets.extend(seg_offsets.into_iter().map(|(s, e)| (s + cursor, e + cursor)));
                cursor += part.len();
            }
        }
        Ok((ids, offsets))
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
    #[ignore] // 실제 가중치를 받아 추론을 돌린다 — CI 기본 실행에서 제외
    fn infer_runs_end_to_end_with_real_weights() {
        let question = crate::protocol::Question::Noul {
            instructions: "참인가?".into(),
        };
        let answer = infer("서버가 다운됐습니다", &question).unwrap();
        assert_eq!(answer["type"], "noul");
        assert!(answer["noul"].as_f64().is_some());
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
        // 같아야 한다. 파일을 미리 깔아두지 않은 디렉터리를 써서 resolve_weights()의
        // exists-체크를 반드시 지나치게 만들고, 그 다음 분기(clef_weights_is_set())가
        // 하드 에러가 아니라 다운로드로 가는지를 직접 관찰한다 — 수정 전 코드라면
        // std::env::var("CLEF_WEIGHTS").is_ok()가 true이므로 HardError를 돌려주지만,
        // 수정 후에는 trim 결과가 비어 있어 NeedsDownload여야 한다.
        let dir = std::env::temp_dir().join(format!("clef-weights-decision-{}", std::process::id()));
        std::fs::remove_dir_all(&dir).ok();
        std::env::set_var("CLEF_WEIGHTS", "");

        let decision = resolve_weights(&dir);

        std::env::remove_var("CLEF_WEIGHTS");

        match decision {
            WeightsDecision::NeedsDownload => {}
            WeightsDecision::HardError(err) => {
                panic!("CLEF_WEIGHTS=\"\"는 미설정으로 취급되어야 하는데 하드 에러가 반환됨: {err}")
            }
            WeightsDecision::Found(..) => {
                panic!("파일을 깔아두지 않았는데 Found가 반환됨 — 테스트 전제가 깨짐")
            }
        }
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
