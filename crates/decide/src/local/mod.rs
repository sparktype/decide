mod joint_head;
mod mlx_backbone;
mod postprocess;
mod timing;
// parity 테스트(`crates/decide/tests/parity.rs`)가 `decide::local::tokenizer::spans`로
// 실제 토큰 스팬을 검증해야 하므로 `pub`으로 재노출한다.
pub mod tokenizer;

use crate::protocol::Question;
use serde_json::Value;
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};

/// `CLEF_WEIGHTS` 환경변수, 없으면 `config.toml`의 `[local].weights`(둘 다 공백만이면 미설정).
fn pinned_weights_dir() -> Option<PathBuf> {
    let env = std::env::var("CLEF_WEIGHTS")
        .ok()
        .filter(|v| !v.trim().is_empty())
        .map(|v| v.trim().to_string());
    let value = match env {
        Some(value) => Some(value),
        None => {
            let (file, warnings) =
                crate::config::load_from_disk(std::env::var("HOME").ok().as_deref());
            for warning in &warnings {
                eprintln!("{warning}");
            }
            file.local_weights
        }
    };
    value.map(PathBuf::from)
}

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

struct Runtime {
    backbone: Mutex<mlx_backbone::MlxBackbone>,
    joint_head: joint_head::JointHead,
    tokenizer: LocalTokenizer,
}

static RUNTIME: OnceLock<Result<Runtime, String>> = OnceLock::new();

fn runtime() -> &'static Result<Runtime, String> {
    RUNTIME.get_or_init(|| {
        let (files, head_path) = ensure_weights()?;
        let backbone = mlx_backbone::MlxBackbone::load(&files)?;
        let joint_head = joint_head::JointHead::load(&head_path, backbone.hidden_size())?;
        Ok(Runtime {
            backbone: Mutex::new(backbone),
            joint_head,
            tokenizer: LocalTokenizer::load()?,
        })
    })
}

/// 은닉 상태를 헤드가 받는 `[1, L, hidden]` 텐서로 바꾸고, 헤드가 어휘 임베딩을 `input_ids`로
/// `index_select`하므로 옵션 구간 토큰만 dequantize한 압축 테이블과 그 테이블 기준으로 다시 매긴
/// `input_ids`를 만든다 — 어휘 전체(248320×hidden) f32 행렬을 만들지 않기 위해서다.
fn head_inputs(
    backbone: &mlx_backbone::MlxBackbone,
    token_ids: &[u32],
    offsets: &[(usize, usize)],
    option_spans: &[tokenizer::OptionSpan],
) -> Result<(candle_core::Tensor, candle_core::Tensor, candle_core::Tensor), String> {
    use candle_core::{Device, Tensor};
    let device = Device::Cpu;
    let hidden = backbone.hidden_size();
    let flat = backbone.hidden_states(token_ids)?;
    let hidden_states = Tensor::from_vec(flat, (1, token_ids.len(), hidden), &device).map_err(|err| err.to_string())?;

    let mut compact_ids: Vec<u32> = Vec::new();
    let mut slot_of = std::collections::HashMap::new();
    let mut remapped = vec![0u32; token_ids.len()];
    for span in option_spans {
        let (start, end) = joint_head::JointHead::char_span_to_token_span(offsets, span.start_char, span.end_char);
        for position in start..end.min(token_ids.len()) {
            let id = token_ids[position];
            let slot = *slot_of.entry(id).or_insert_with(|| {
                compact_ids.push(id);
                (compact_ids.len() - 1) as u32
            });
            remapped[position] = slot;
        }
    }
    let rows = backbone.lexical_rows(&compact_ids)?;
    let table = Tensor::from_vec(rows, (compact_ids.len(), hidden), &device).map_err(|err| err.to_string())?;
    let input_ids = Tensor::new(remapped.as_slice(), &device)
        .and_then(|t| t.unsqueeze(0))
        .map_err(|err| err.to_string())?;
    timing::lap("head_tensors");
    Ok((hidden_states, input_ids, table))
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
    timing::lap("tokenize");
    let (hidden_states, input_ids, output_embeddings) = {
        let backbone = runtime.backbone.lock().map_err(|_| "백본 락 획득에 실패했습니다".to_string())?;
        head_inputs(&backbone, &token_ids, &offsets, &option_spans)?
    };
    let question_type = tokenizer::question_type_id(question);
    let scored = runtime.joint_head.score(
        &hidden_states,
        &input_ids,
        question_type,
        &question_span,
        &option_spans,
        &offsets,
        &output_embeddings,
    );
    timing::lap("head");
    timing::finish(token_ids.len());
    scored
}

/// 추론 한 번이 끝나면(성공·실패 어느 쪽이든) MLX의 allocator 캐시를 비운다. 비우지 않으면
/// forward pass의 중간 텐서(은닉 상태, dequantize된 어휘 행 등)가 프로세스에 계속 쌓여, 양자화
/// 비트 수를 줄여도 GPU 메모리가 줄지 않는다(활성 상태 보기의 "GPU 프로세스" 메모리로 관찰됨).
struct ClearCacheGuard;

impl Drop for ClearCacheGuard {
    fn drop(&mut self) {
        let _ = mlx_rs::memory::clear_cache();
    }
}

/// 데몬이 시작할 때 모델을 미리 올리고 더미 추론을 한 번 돌린다. 가중치 로딩·페이지인·Metal 커널
/// 컴파일 비용을 첫 실제 요청이 떠안지 않게 한다.
pub fn warmup() -> Result<(), String> {
    infer("warmup", &Question::Noul { instructions: "참인가?".into() }).map(|_| ())
}

pub fn infer(state: &str, question: &Question) -> Result<Value, String> {
    let _guard = ClearCacheGuard;
    timing::start();
    let runtime = match runtime() {
        Ok(runtime) => runtime,
        Err(err) => return Err(err.clone()),
    };
    timing::lap("load");
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

/// 기본 MLX 체크포인트 저장소(8bit). 비전 텐서는 샤드에 섞여 있지만 읽을 때 건너뛴다.
/// `DECIDE_LOCAL_REPO` 환경변수나 config.toml의 `[local].repo`로 바꿀 수 있다(`resolved_repo`).
const MLX_REPO: &str = "mlx-community/clef-flash-8bit";

/// `DECIDE_LOCAL_REPO` 환경변수, 없으면 config.toml의 `[local].repo`, 둘 다 없으면 `MLX_REPO`.
fn resolved_repo() -> String {
    let env = std::env::var("DECIDE_LOCAL_REPO").ok().filter(|v| !v.trim().is_empty());
    if let Some(repo) = env {
        return repo;
    }
    let (file, warnings) = crate::config::load_from_disk(std::env::var("HOME").ok().as_deref());
    for warning in &warnings {
        eprintln!("{warning}");
    }
    file.local_repo.unwrap_or_else(|| MLX_REPO.to_string())
}

const FIXED_FILES: [&str; 3] = ["config.json", "model.safetensors.index.json", "joint_head.safetensors"];

type Weights = (mlx_backbone::MlxFiles, PathBuf);

/// `CLEF_WEIGHTS` 디렉터리에서 체크포인트를 찾는다. 디렉터리를 직접 지정했으니 하나라도 없으면
/// 다운로드로 넘어가지 않고 하드 에러다.
fn resolve_pinned(dir: &std::path::Path) -> Result<Weights, String> {
    if let Some(missing) = FIXED_FILES.iter().find(|name| !dir.join(name).exists()) {
        return Err(format!(
            "CLEF_WEIGHTS={}에서 MLX 체크포인트 파일을 찾을 수 없습니다 ({missing} 없음, 필요: {})",
            dir.display(),
            FIXED_FILES.join(", ")
        ));
    }
    let shards = mlx_backbone::shard_names(&dir.join("model.safetensors.index.json"))?
        .into_iter()
        .map(|name| dir.join(name))
        .collect::<Vec<_>>();
    if let Some(missing) = shards.iter().find(|path| !path.exists()) {
        return Err(format!("CLEF_WEIGHTS에 샤드 {}가 없습니다", missing.display()));
    }
    Ok((
        mlx_backbone::MlxFiles { config: dir.join("config.json"), shards },
        dir.join("joint_head.safetensors"),
    ))
}

/// HuggingFace 캐시(`~/.cache/huggingface/hub`, `HF_HOME`/config.toml로 바꿀 수 있다)에서
/// 받는다. 이미 있으면 다운로드 없이 그 경로를 쓴다. 첫 사용에는 약 10GB를 받는다.
/// HF_ENDPOINT/HF_HOME/HF_TOKEN 환경변수나 config.toml의 [local] 값을 반영한 HuggingFace API.
/// `ApiBuilder::from_env()`는 HF_HOME/HF_ENDPOINT 환경변수를 반영한다. `Api::new()`는 반영하지
/// 않는다 — 지금까지 이 환경변수들이 가중치·토크나이저 다운로드 모두에서 적용되지 않던 버그.
fn hf_api() -> Result<hf_hub::api::sync::Api, String> {
    let resolved = resolved_hf();
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
    builder
        .build()
        .map_err(|err| format!("HuggingFace API 초기화에 실패했습니다: {err}"))
}

fn download_weights() -> Result<Weights, String> {
    let api = hf_api()?;
    let repo_name = resolved_repo();
    let repo = api.model(repo_name.clone());
    let fetch = |name: &str| {
        repo.get(name)
            .map_err(|err| format!("{repo_name}/{name} 다운로드에 실패했습니다: {err}"))
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

fn ensure_weights() -> Result<Weights, String> {
    match pinned_weights_dir() {
        Some(dir) => resolve_pinned(&dir),
        None => download_weights(),
    }
}

pub struct LocalTokenizer {
    inner: tokenizers::Tokenizer,
}

impl LocalTokenizer {
    pub fn load() -> Result<Self, String> {
        let api = hf_api()?;
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
    fn clear_cache_guard_calls_clear_cache_on_drop_even_on_early_return() {
        fn returns_early(fail: bool) -> Result<(), String> {
            let _guard = ClearCacheGuard;
            if fail {
                return Err("실패".to_string());
            }
            Ok(())
        }

        mlx_rs::memory::clear_cache().unwrap();
        let before = mlx_rs::memory::cache_memory().unwrap();

        // 실패 경로에서도 가드가 드롭되며 clear_cache를 부른다 — 패닉하지 않으면 통과.
        let _ = returns_early(true);
        let _ = returns_early(false);

        let after = mlx_rs::memory::cache_memory().unwrap();
        assert!(after <= before, "clear_cache 뒤에는 캐시가 늘어 있지 않아야 한다");
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
    #[ignore] // 실제 가중치로 단계별 지연을 본다 — `DECIDE_LOCAL_TIMING=1 cargo test -- --ignored --nocapture timing_breakdown`
    fn timing_breakdown_with_real_weights() {
        let question = crate::protocol::Question::Noul { instructions: "참인가?".into() };
        let long_state = "서버 로그에서 결제 서비스의 응답 지연이 관측되었고 재시도가 늘었습니다. ".repeat(40);
        for (name, state) in [("cold-short", "서버가 다운됐습니다"), ("warm-short", "서버가 다운됐습니다"), ("warm-long", long_state.as_str())] {
            let start = std::time::Instant::now();
            infer(state, &question).unwrap();
            eprintln!("{name}: wall={:.1}ms", start.elapsed().as_secs_f64() * 1000.0);
        }
    }

    #[test]
    #[ignore] // 실제 가중치 필요 — 선로딩 뒤 첫 요청이 웜 상태(두 번째 요청과 비슷한 지연)인지 본다
    fn warmup_moves_cold_cost_out_of_first_request() {
        let start = std::time::Instant::now();
        warmup().unwrap();
        eprintln!("warmup: wall={:.1}ms", start.elapsed().as_secs_f64() * 1000.0);
        let question = crate::protocol::Question::Noul { instructions: "참인가?".into() };
        let start = std::time::Instant::now();
        infer("서버가 다운됐습니다", &question).unwrap();
        eprintln!("first-after-warmup: wall={:.1}ms", start.elapsed().as_secs_f64() * 1000.0);
    }

    fn weights_fixture(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("clef-weights-{name}-{}", std::process::id()));
        std::fs::remove_dir_all(&dir).ok();
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn resolve_pinned_finds_shards_listed_in_the_index() {
        let dir = weights_fixture("found");
        std::fs::write(dir.join("config.json"), b"{}").unwrap();
        std::fs::write(dir.join("joint_head.safetensors"), b"fake").unwrap();
        std::fs::write(
            dir.join("model.safetensors.index.json"),
            br#"{"weight_map":{"a":"model-00002-of-00002.safetensors","b":"model-00001-of-00002.safetensors","c":"model-00001-of-00002.safetensors"}}"#,
        )
        .unwrap();
        std::fs::write(dir.join("model-00001-of-00002.safetensors"), b"fake").unwrap();
        std::fs::write(dir.join("model-00002-of-00002.safetensors"), b"fake").unwrap();

        let (files, head) = resolve_pinned(&dir).unwrap();
        assert_eq!(head, dir.join("joint_head.safetensors"));
        assert_eq!(files.config, dir.join("config.json"));
        assert_eq!(
            files.shards,
            vec![dir.join("model-00001-of-00002.safetensors"), dir.join("model-00002-of-00002.safetensors")]
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn resolve_pinned_errors_when_files_are_missing() {
        let dir = weights_fixture("missing");
        let err = resolve_pinned(&dir).unwrap_err();
        assert!(err.contains("찾을 수 없습니다"));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn resolve_pinned_errors_when_a_listed_shard_is_missing() {
        let dir = weights_fixture("no-shard");
        std::fs::write(dir.join("config.json"), b"{}").unwrap();
        std::fs::write(dir.join("joint_head.safetensors"), b"fake").unwrap();
        std::fs::write(
            dir.join("model.safetensors.index.json"),
            br#"{"weight_map":{"a":"model-00001-of-00001.safetensors"}}"#,
        )
        .unwrap();
        let err = resolve_pinned(&dir).unwrap_err();
        assert!(err.contains("샤드"));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn pinned_weights_dir_treats_blank_as_unset() {
        // CLEF_WEIGHTS가 공백이면 pinned_weights_dir가 config.toml로 넘어간다 — 실제
        // $HOME에 있을 수 있는 config.toml을 보지 않도록 이 테스트만의 임시 HOME을 쓴다.
        let home = std::env::temp_dir().join(format!(
            "decide-local-weights-blank-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&home).unwrap();
        std::env::set_var("HOME", &home);
        std::env::set_var("CLEF_WEIGHTS", "  ");
        let blank = pinned_weights_dir();
        std::env::set_var("CLEF_WEIGHTS", " /tmp/clef-weights-pinned ");
        let set = pinned_weights_dir();
        std::env::remove_var("CLEF_WEIGHTS");
        std::env::remove_var("HOME");
        let _ = std::fs::remove_dir_all(&home);
        assert_eq!(blank, None);
        assert_eq!(set, Some(PathBuf::from("/tmp/clef-weights-pinned")));
    }

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

    #[test]
    fn resolved_repo_falls_back_to_config_file_then_default() {
        let home = std::env::temp_dir().join(format!(
            "decide-local-repo-cfg-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&home).unwrap();
        std::env::remove_var("DECIDE_LOCAL_REPO");
        std::env::set_var("HOME", &home);

        // 환경변수도, config.toml도 없으면 기본값(8bit).
        assert_eq!(resolved_repo(), MLX_REPO);

        // config.toml만 있으면 그 값.
        std::fs::create_dir_all(home.join(".config/decide")).unwrap();
        std::fs::write(
            home.join(".config/decide/config.toml"),
            "[local]\nrepo = \"mlx-community/clef-flash-4bit\"\n",
        )
        .unwrap();
        assert_eq!(resolved_repo(), "mlx-community/clef-flash-4bit");

        // 환경변수가 있으면 config.toml을 가린다.
        std::env::set_var("DECIDE_LOCAL_REPO", "mlx-community/clef-flash-env");
        assert_eq!(resolved_repo(), "mlx-community/clef-flash-env");

        std::env::remove_var("DECIDE_LOCAL_REPO");
        std::env::remove_var("HOME");
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    #[ignore] // 실제 네트워크로 HuggingFace에서 약 10GB를 받는다 — CI 기본 실행에서 제외
    fn ensure_weights_downloads_when_cache_empty() {
        std::env::remove_var("CLEF_WEIGHTS");
        let (files, head) = ensure_weights().unwrap();
        assert!(files.config.exists());
        assert!(head.exists());
        assert!(files.shards.iter().all(|path| path.exists()));
    }
}
