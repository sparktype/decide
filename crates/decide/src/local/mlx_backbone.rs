// MLX(mlx-rs)로 Clef-flash 8비트 백본을 돌려 전 시퀀스 은닉 상태를 만든다.
//
// 포팅 원본은 mlx-lm 0.32.0의 `models/qwen3_5.py`·`qwen3_next.py`·`gated_delta.py`(ops 경로)다.
// 체크포인트(`mlx-community/clef-flash-8bit`)는 이미 mlx 형식이다 — conv1d는 `[C, K, 1]`,
// RMSNorm 가중치는 +1이 반영돼 있고, 모든 선형층과 임베딩이 8비트 affine(group 64)이라
// 따로 sanitize하지 않는다. 비전 텐서는 읽지 않는다.
//
// 배치는 항상 1이다. 조인트 헤드는 candle CPU에 그대로 두므로 여기서는 은닉 상태를
// 헤드가 받는 형태(`[1, L, hidden]` f32)로 바꿔 넘기는 데까지만 맡는다.
use mlx_rs::error::Exception;
use mlx_rs::ops::indexing::IndexOp;
use mlx_rs::ops::{self, concatenate};
use mlx_rs::{fast, nn, Array, Dtype};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

type R<T> = Result<T, Exception>;

struct Config {
    hidden: i32,
    layers: usize,
    heads: i32,
    kv_heads: i32,
    head_dim: i32,
    rope_dims: i32,
    rope_theta: f32,
    eps: f32,
    full_attention_interval: usize,
    lin_k_heads: i32,
    lin_v_heads: i32,
    lin_k_dim: i32,
    lin_v_dim: i32,
    conv_kernel: i32,
    group_size: i32,
    bits: i32,
}

impl Config {
    fn from_json(root: &serde_json::Value) -> Result<Self, String> {
        let text = root.get("text_config").ok_or("config.json에 text_config가 없습니다")?;
        let int = |key: &str| -> Result<i64, String> {
            text.get(key)
                .and_then(|v| v.as_i64())
                .ok_or_else(|| format!("config.json text_config.{key}를 읽을 수 없습니다"))
        };
        let quant = root.get("quantization").ok_or("config.json에 quantization이 없습니다")?;
        let quant_int = |key: &str| -> Result<i32, String> {
            quant
                .get(key)
                .and_then(|v| v.as_i64())
                .map(|v| v as i32)
                .ok_or_else(|| format!("config.json quantization.{key}를 읽을 수 없습니다"))
        };
        let rope = text
            .get("rope_parameters")
            .ok_or("config.json text_config.rope_parameters가 없습니다")?;
        let head_dim = int("head_dim")? as i32;
        let partial = rope
            .get("partial_rotary_factor")
            .and_then(|v| v.as_f64())
            .ok_or("rope_parameters.partial_rotary_factor를 읽을 수 없습니다")?;
        Ok(Self {
            hidden: int("hidden_size")? as i32,
            layers: int("num_hidden_layers")? as usize,
            heads: int("num_attention_heads")? as i32,
            kv_heads: int("num_key_value_heads")? as i32,
            head_dim,
            rope_dims: (head_dim as f64 * partial) as i32,
            rope_theta: rope
                .get("rope_theta")
                .and_then(|v| v.as_f64())
                .ok_or("rope_parameters.rope_theta를 읽을 수 없습니다")? as f32,
            eps: text
                .get("rms_norm_eps")
                .and_then(|v| v.as_f64())
                .ok_or("config.json text_config.rms_norm_eps를 읽을 수 없습니다")? as f32,
            full_attention_interval: int("full_attention_interval")? as usize,
            lin_k_heads: int("linear_num_key_heads")? as i32,
            lin_v_heads: int("linear_num_value_heads")? as i32,
            lin_k_dim: int("linear_key_head_dim")? as i32,
            lin_v_dim: int("linear_value_head_dim")? as i32,
            conv_kernel: int("linear_conv_kernel_dim")? as i32,
            group_size: quant_int("group_size")?,
            bits: quant_int("bits")?,
        })
    }
}

type Weights = HashMap<String, Array>;

fn take(weights: &mut Weights, key: &str) -> R<Array> {
    weights
        .remove(key)
        .ok_or_else(|| Exception::custom(format!("가중치 {key}가 체크포인트에 없습니다")))
}

/// 8비트 affine 양자화 선형층. 가중치는 `[out, in * bits / 32]`(u32)로 묶여 있다.
struct QLinear {
    weight: Array,
    scales: Array,
    biases: Array,
    group_size: i32,
    bits: i32,
}

impl QLinear {
    fn load(weights: &mut Weights, prefix: &str, cfg: &Config) -> R<Self> {
        Ok(Self {
            weight: take(weights, &format!("{prefix}.weight"))?,
            scales: take(weights, &format!("{prefix}.scales"))?,
            biases: take(weights, &format!("{prefix}.biases"))?,
            group_size: cfg.group_size,
            bits: cfg.bits,
        })
    }

    fn forward(&self, x: &Array) -> R<Array> {
        ops::quantized_matmul(x, &self.weight, &self.scales, &self.biases, true, self.group_size, self.bits)
    }

    /// 지정한 행들만 dequantize한다 — 어휘 전체(248320행)를 펼치지 않기 위해서다.
    fn dequantize_rows(&self, ids: &Array) -> R<Array> {
        let w = self.weight.take_axis(ids, 0)?;
        let s = self.scales.take_axis(ids, 0)?;
        let b = self.biases.take_axis(ids, 0)?;
        ops::dequantize(&w, &s, &b, self.group_size, self.bits)
    }
}

fn rms_norm(x: &Array, weight: &Array, eps: f32) -> R<Array> {
    fast::rms_norm(x, Some(weight), eps)
}

struct Mlp {
    gate: QLinear,
    up: QLinear,
    down: QLinear,
}

impl Mlp {
    fn load(weights: &mut Weights, prefix: &str, cfg: &Config) -> R<Self> {
        Ok(Self {
            gate: QLinear::load(weights, &format!("{prefix}.gate_proj"), cfg)?,
            up: QLinear::load(weights, &format!("{prefix}.up_proj"), cfg)?,
            down: QLinear::load(weights, &format!("{prefix}.down_proj"), cfg)?,
        })
    }

    fn forward(&self, x: &Array) -> R<Array> {
        let gated = ops::multiply(nn::silu(self.gate.forward(x)?)?, self.up.forward(x)?)?;
        self.down.forward(&gated)
    }
}

struct FullAttention {
    q_proj: QLinear,
    k_proj: QLinear,
    v_proj: QLinear,
    o_proj: QLinear,
    q_norm: Array,
    k_norm: Array,
}

impl FullAttention {
    fn load(weights: &mut Weights, prefix: &str, cfg: &Config) -> R<Self> {
        Ok(Self {
            q_proj: QLinear::load(weights, &format!("{prefix}.q_proj"), cfg)?,
            k_proj: QLinear::load(weights, &format!("{prefix}.k_proj"), cfg)?,
            v_proj: QLinear::load(weights, &format!("{prefix}.v_proj"), cfg)?,
            o_proj: QLinear::load(weights, &format!("{prefix}.o_proj"), cfg)?,
            q_norm: take(weights, &format!("{prefix}.q_norm.weight"))?,
            k_norm: take(weights, &format!("{prefix}.k_norm.weight"))?,
        })
    }

    fn forward(&self, x: &Array, cfg: &Config) -> R<Array> {
        let len = x.shape()[1];
        // q_proj는 헤드마다 [query(head_dim), gate(head_dim)]를 이어서 낸다.
        let q_out = self.q_proj.forward(x)?.reshape(&[1, len, cfg.heads, cfg.head_dim * 2])?;
        let parts = q_out.split_equal(2, -1)?;
        let queries = &parts[0];
        let gate = parts[1].reshape(&[1, len, cfg.heads * cfg.head_dim])?;

        let keys = self.k_proj.forward(x)?.reshape(&[1, len, cfg.kv_heads, cfg.head_dim])?;
        let values = self.v_proj.forward(x)?.reshape(&[1, len, cfg.kv_heads, cfg.head_dim])?;

        let to_heads = |a: &Array| a.transpose_axes(&[0, 2, 1, 3]);
        let rope = |a: &Array| fast::rope(a, cfg.rope_dims, false, cfg.rope_theta, 1.0, 0, None);
        let q = rope(&to_heads(&rms_norm(queries, &self.q_norm, cfg.eps)?)?)?;
        let k = rope(&to_heads(&rms_norm(&keys, &self.k_norm, cfg.eps)?)?)?;
        let v = to_heads(&values)?;

        let scale = (cfg.head_dim as f32).powf(-0.5);
        let attended = fast::scaled_dot_product_attention(
            &q,
            &k,
            &v,
            scale,
            fast::ScaledDotProductAttentionMask::Causal,
            None,
        )?;
        let merged = attended.transpose_axes(&[0, 2, 1, 3])?.reshape(&[1, len, cfg.heads * cfg.head_dim])?;
        self.o_proj.forward(&ops::multiply(&merged, ops::sigmoid(&gate)?)?)
    }
}

struct GatedDeltaNet {
    in_proj_qkv: QLinear,
    in_proj_z: QLinear,
    in_proj_b: QLinear,
    in_proj_a: QLinear,
    out_proj: QLinear,
    conv_weight: Array, // [conv_dim, K, 1]
    dt_bias: Array,
    a_log: Array,
    norm_weight: Array,
}

impl GatedDeltaNet {
    fn load(weights: &mut Weights, prefix: &str, cfg: &Config) -> R<Self> {
        Ok(Self {
            in_proj_qkv: QLinear::load(weights, &format!("{prefix}.in_proj_qkv"), cfg)?,
            in_proj_z: QLinear::load(weights, &format!("{prefix}.in_proj_z"), cfg)?,
            in_proj_b: QLinear::load(weights, &format!("{prefix}.in_proj_b"), cfg)?,
            in_proj_a: QLinear::load(weights, &format!("{prefix}.in_proj_a"), cfg)?,
            out_proj: QLinear::load(weights, &format!("{prefix}.out_proj"), cfg)?,
            conv_weight: take(weights, &format!("{prefix}.conv1d.weight"))?,
            dt_bias: take(weights, &format!("{prefix}.dt_bias"))?,
            a_log: take(weights, &format!("{prefix}.A_log"))?,
            norm_weight: take(weights, &format!("{prefix}.norm.weight"))?,
        })
    }

    fn forward(&self, x: &Array, cfg: &Config) -> R<Array> {
        let len = x.shape()[1];
        let key_dim = cfg.lin_k_heads * cfg.lin_k_dim;
        let value_dim = cfg.lin_v_heads * cfg.lin_v_dim;
        let conv_dim = key_dim * 2 + value_dim;

        let qkv = self.in_proj_qkv.forward(x)?;
        let z = self.in_proj_z.forward(x)?.reshape(&[1, len, cfg.lin_v_heads, cfg.lin_v_dim])?;
        let b = self.in_proj_b.forward(x)?;
        let a = self.in_proj_a.forward(x)?;

        let conv_out = nn::silu(depthwise_causal_conv(&qkv, &self.conv_weight, cfg.conv_kernel, conv_dim)?)?;
        let qkv_parts = ops::split_at_indices(&conv_out, &[key_dim, 2 * key_dim], -1)?;
        let q = qkv_parts[0].reshape(&[1, len, cfg.lin_k_heads, cfg.lin_k_dim])?;
        let k = qkv_parts[1].reshape(&[1, len, cfg.lin_k_heads, cfg.lin_k_dim])?;
        let v = qkv_parts[2].reshape(&[1, len, cfg.lin_v_heads, cfg.lin_v_dim])?;

        // q/k L2 정규화 + 읽기 스케일. `l2norm`의 eps는 sum(x²)에 더해지는데 `rms_norm`의 eps는
        // mean(x²)에 더해지므로 inv_scale²를 곱해 맞춘다(mlx-lm `normalize_qk`).
        let inv_scale = (cfg.lin_k_dim as f32).powf(-0.5);
        let rms_eps = 1e-6 * inv_scale * inv_scale;
        let q = ops::multiply(fast::rms_norm(&q, None, rms_eps)?, Array::from_f32(inv_scale * inv_scale))?;
        let k = ops::multiply(fast::rms_norm(&k, None, rms_eps)?, Array::from_f32(inv_scale))?;

        // g = exp(-exp(A_log) * softplus(a + dt_bias)), beta = sigmoid(b) — 둘 다 f32.
        let a_f32 = a.as_dtype(Dtype::Float32)?;
        let g = ops::exp(ops::negative(ops::multiply(
            ops::exp(self.a_log.as_dtype(Dtype::Float32)?)?,
            nn::softplus(ops::add(&a_f32, self.dt_bias.as_dtype(Dtype::Float32)?)?)?,
        )?)?)?;
        let beta = ops::sigmoid(b.as_dtype(Dtype::Float32)?)?;

        let out = gated_delta_rule(&q, &k, &v, &g, &beta, cfg)?;
        let out_dtype = x.dtype();
        // 게이트드 RMSNorm: rms_norm(out) * silu(z), 곱은 f32로 한다.
        let normed = fast::rms_norm(&out, Some(&self.norm_weight), cfg.eps)?;
        let gated = ops::multiply(
            nn::silu(z.as_dtype(Dtype::Float32)?)?,
            normed.as_dtype(Dtype::Float32)?,
        )?
        .as_dtype(out_dtype)?;
        self.out_proj.forward(&gated.reshape(&[1, len, value_dim])?)
    }
}

/// 깊이별(depthwise) causal conv1d. 커널이 4로 작아서 슬라이스 곱의 합으로 직접 계산한다 —
/// `qkv`는 `[1, L, C]`, `weight`는 `[C, K, 1]`. 앞쪽을 0으로 K-1칸 채워 t번째 출력이
/// 입력 t-K+1..=t만 보게 한다.
fn depthwise_causal_conv(qkv: &Array, weight: &Array, kernel: i32, channels: i32) -> R<Array> {
    let len = qkv.shape()[1];
    let pad = ops::zeros_dtype(&[1, kernel - 1, channels], qkv.dtype())?;
    let padded = concatenate(&[&pad, qkv], 1)?;
    let mut acc: Option<Array> = None;
    for k in 0..kernel {
        let window = padded.index((.., k..k + len, ..));
        let tap = weight.index((.., k, 0)).as_dtype(qkv.dtype())?; // [C]
        let term = ops::multiply(&window, &tap)?;
        acc = Some(match acc {
            Some(sum) => ops::add(&sum, &term)?,
            None => term,
        });
    }
    Ok(acc.expect("kernel >= 1"))
}

/// mlx-lm `gated_delta.py`의 기본(스칼라 게이트, 마스크 없음) Metal 커널. 스레드 하나가 상태 행
/// 하나(Dv 한 줄)를 맡고 시간 축을 커널 안에서 돈다 — ops 루프처럼 스텝마다 커널을 띄우지 않는다.
const DELTA_KERNEL_SOURCE: &str = r#"
    auto n = thread_position_in_grid.z;
    auto b_idx = n / Hv;
    auto hv_idx = n % Hv;
    auto hk_idx = hv_idx / (Hv / Hk);
    constexpr int n_per_t = Dk / 32;

    auto q_ = q + b_idx * T * Hk * Dk + hk_idx * Dk;
    auto k_ = k + b_idx * T * Hk * Dk + hk_idx * Dk;
    auto v_ = v + b_idx * T * Hv * Dv + hv_idx * Dv;
    y += b_idx * T * Hv * Dv + hv_idx * Dv;

    auto dk_idx = thread_position_in_threadgroup.x;
    auto dv_idx = thread_position_in_grid.y;

    auto i_state = state_in + (n * Dv + dv_idx) * Dk;
    auto o_state = state_out + (n * Dv + dv_idx) * Dk;

    float state[n_per_t];
    for (int i = 0; i < n_per_t; ++i) {
      auto s_idx = n_per_t * dk_idx + i;
      state[i] = static_cast<float>(i_state[s_idx]);
    }

    auto g_ = g + b_idx * T * Hv;
    auto beta_ = beta + b_idx * T * Hv;

    for (int t = 0; t < T; ++t) {
      float kv_mem = 0.0f;
      for (int i = 0; i < n_per_t; ++i) {
        auto s_idx = n_per_t * dk_idx + i;
        state[i] = state[i] * g_[hv_idx];
        kv_mem += state[i] * k_[s_idx];
      }
      kv_mem = simd_sum(kv_mem);

      auto delta = (v_[dv_idx] - kv_mem) * beta_[hv_idx];

      float out = 0.0f;
      for (int i = 0; i < n_per_t; ++i) {
        auto s_idx = n_per_t * dk_idx + i;
        state[i] = state[i] + k_[s_idx] * delta;
        out += state[i] * q_[s_idx];
      }
      out = simd_sum(out);
      if (thread_index_in_simdgroup == 0) {
        y[dv_idx] = static_cast<InT>(out);
      }
      q_ += Hk * Dk;
      k_ += Hk * Dk;
      v_ += Hv * Dv;
      y += Hv * Dv;
      g_ += Hv;
      beta_ += Hv;
    }
    for (int i = 0; i < n_per_t; ++i) {
      auto s_idx = n_per_t * dk_idx + i;
      o_state[s_idx] = static_cast<StT>(state[i]);
    }
"#;

struct DeltaKernel(mlx_sys::mlx_fast_metal_kernel);

// 커널 핸들은 만든 뒤 읽기만 하고, 호출은 백본 `Mutex` 안에서만 일어난다.
unsafe impl Send for DeltaKernel {}
unsafe impl Sync for DeltaKernel {}

fn delta_kernel() -> &'static DeltaKernel {
    static KERNEL: std::sync::OnceLock<DeltaKernel> = std::sync::OnceLock::new();
    KERNEL.get_or_init(|| unsafe {
        let names = |items: &[&str]| {
            let vector = mlx_sys::mlx_vector_string_new();
            for item in items {
                let c = std::ffi::CString::new(*item).expect("NUL 없는 상수");
                mlx_sys::mlx_vector_string_append_value(vector, c.as_ptr());
            }
            vector
        };
        let inputs = names(&["q", "k", "v", "g", "beta", "state_in", "T"]);
        let outputs = names(&["y", "state_out"]);
        let name = std::ffi::CString::new("gated_delta_step").expect("NUL 없는 상수");
        let source = std::ffi::CString::new(DELTA_KERNEL_SOURCE).expect("NUL 없는 상수");
        let header = std::ffi::CString::new("").expect("NUL 없는 상수");
        let kernel = mlx_sys::mlx_fast_metal_kernel_new(
            name.as_ptr(),
            inputs,
            outputs,
            source.as_ptr(),
            header.as_ptr(),
            true,
            false,
        );
        mlx_sys::mlx_vector_string_free(inputs);
        mlx_sys::mlx_vector_string_free(outputs);
        DeltaKernel(kernel)
    })
}

/// 게이트드 델타 규칙을 Metal 커널로 계산한다. 입력 규약은 `gated_delta_rule_ops`와 같지만
/// `q`/`k`는 헤드를 늘리지 않은 `[1, L, Hk, Dk]`다(커널이 `hv / (Hv/Hk)`로 매핑한다).
/// 커널은 `Dk`가 32의 배수여야 한다.
fn gated_delta_rule(q: &Array, k: &Array, v: &Array, g: &Array, beta: &Array, cfg: &Config) -> R<Array> {
    if cfg.lin_k_dim % 32 != 0 {
        return Err(Exception::custom("델타 커널은 linear_key_head_dim이 32의 배수여야 합니다"));
    }
    let len = q.shape()[1];
    let state = ops::zeros::<f32>(&[1, cfg.lin_v_heads, cfg.lin_v_dim, cfg.lin_k_dim])?;
    let t = Array::from_int(len);
    let check = |code: i32, what: &str| -> R<()> {
        if code == 0 { Ok(()) } else { Err(Exception::custom(format!("Metal 커널 설정 실패: {what}"))) }
    };
    unsafe {
        let config = mlx_sys::mlx_fast_metal_kernel_config_new();
        let result = (|| -> R<Array> {
            let y_shape = [1, len, cfg.lin_v_heads, cfg.lin_v_dim];
            let state_shape = [1, cfg.lin_v_heads, cfg.lin_v_dim, cfg.lin_k_dim];
            check(
                mlx_sys::mlx_fast_metal_kernel_config_add_output_arg(config, y_shape.as_ptr(), 4, q.dtype().into()),
                "출력 y",
            )?;
            check(
                mlx_sys::mlx_fast_metal_kernel_config_add_output_arg(
                    config,
                    state_shape.as_ptr(),
                    4,
                    Dtype::Float32.into(),
                ),
                "출력 state",
            )?;
            check(
                mlx_sys::mlx_fast_metal_kernel_config_set_grid(config, 32, cfg.lin_v_dim, cfg.lin_v_heads),
                "grid",
            )?;
            check(mlx_sys::mlx_fast_metal_kernel_config_set_thread_group(config, 32, 4, 1), "threadgroup")?;
            let dtype_arg = |name: &str, dtype: Dtype| -> R<()> {
                let c = std::ffi::CString::new(name).expect("NUL 없는 상수");
                check(
                    mlx_sys::mlx_fast_metal_kernel_config_add_template_arg_dtype(config, c.as_ptr(), dtype.into()),
                    name,
                )
            };
            let int_arg = |name: &str, value: i32| -> R<()> {
                let c = std::ffi::CString::new(name).expect("NUL 없는 상수");
                check(mlx_sys::mlx_fast_metal_kernel_config_add_template_arg_int(config, c.as_ptr(), value), name)
            };
            dtype_arg("InT", q.dtype())?;
            dtype_arg("StT", Dtype::Float32)?;
            int_arg("Dk", cfg.lin_k_dim)?;
            int_arg("Dv", cfg.lin_v_dim)?;
            int_arg("Hk", cfg.lin_k_heads)?;
            int_arg("Hv", cfg.lin_v_heads)?;

            let inputs = [q.as_ptr(), k.as_ptr(), v.as_ptr(), g.as_ptr(), beta.as_ptr(), state.as_ptr(), t.as_ptr()];
            let input_vec = mlx_sys::mlx_vector_array_new_data(inputs.as_ptr(), inputs.len());
            let mut output_vec = mlx_sys::mlx_vector_array_new();
            let stream = mlx_rs::Stream::thread_local_or_default();
            let code = mlx_sys::mlx_fast_metal_kernel_apply(
                &mut output_vec,
                delta_kernel().0,
                input_vec,
                config,
                stream.as_ref().as_ptr(),
            );
            mlx_sys::mlx_vector_array_free(input_vec);
            if code != 0 {
                mlx_sys::mlx_vector_array_free(output_vec);
                return Err(Exception::custom("Metal 델타 커널 실행에 실패했습니다"));
            }
            let mut y = mlx_sys::mlx_array_new();
            let got = mlx_sys::mlx_vector_array_get(&mut y, output_vec, 0);
            mlx_sys::mlx_vector_array_free(output_vec);
            check(got, "출력 y 읽기")?;
            Ok(Array::from_ptr(y))
        })();
        mlx_sys::mlx_fast_metal_kernel_config_free(config);
        result
    }
}

enum Mixer {
    Linear(GatedDeltaNet),
    Full(FullAttention),
}

struct Layer {
    mixer: Mixer,
    input_norm: Array,
    post_norm: Array,
    mlp: Mlp,
}

pub struct MlxBackbone {
    cfg: Config,
    embed: QLinear,
    layers: Vec<Layer>,
    norm: Array,
    lm_head: QLinear,
}

// MLX 배열은 GPU 스트림을 쓰지만 이 백본은 `Mutex` 뒤에서 한 번에 한 스레드만 접근한다.
unsafe impl Send for MlxBackbone {}

/// `config.json`, 샤드 safetensors를 담은 체크포인트 파일 위치.
#[derive(Debug)]
pub struct MlxFiles {
    pub config: PathBuf,
    pub shards: Vec<PathBuf>,
}

impl MlxBackbone {
    pub fn load(files: &MlxFiles) -> Result<Self, String> {
        let config_text = std::fs::read_to_string(&files.config)
            .map_err(|err| format!("config.json을 읽을 수 없습니다: {err}"))?;
        let config_json: serde_json::Value =
            serde_json::from_str(&config_text).map_err(|err| format!("config.json 파싱에 실패했습니다: {err}"))?;
        let cfg = Config::from_json(&config_json)?;

        let mut weights = Weights::new();
        for shard in &files.shards {
            let loaded = Array::load_safetensors(shard)
                .map_err(|err| format!("{}를 읽을 수 없습니다: {err}", shard.display()))?;
            weights.extend(loaded);
        }
        Self::from_weights(cfg, weights).map_err(|err| format!("MLX 백본 구성에 실패했습니다: {}", err.what()))
    }

    fn from_weights(cfg: Config, mut weights: Weights) -> R<Self> {
        let root = "language_model.model";
        let mut layers = Vec::with_capacity(cfg.layers);
        for i in 0..cfg.layers {
            let prefix = format!("{root}.layers.{i}");
            let mixer = if (i + 1) % cfg.full_attention_interval == 0 {
                Mixer::Full(FullAttention::load(&mut weights, &format!("{prefix}.self_attn"), &cfg)?)
            } else {
                Mixer::Linear(GatedDeltaNet::load(&mut weights, &format!("{prefix}.linear_attn"), &cfg)?)
            };
            layers.push(Layer {
                mixer,
                input_norm: take(&mut weights, &format!("{prefix}.input_layernorm.weight"))?,
                post_norm: take(&mut weights, &format!("{prefix}.post_attention_layernorm.weight"))?,
                mlp: Mlp::load(&mut weights, &format!("{prefix}.mlp"), &cfg)?,
            });
        }
        Ok(Self {
            embed: QLinear::load(&mut weights, &format!("{root}.embed_tokens"), &cfg)?,
            norm: take(&mut weights, &format!("{root}.norm.weight"))?,
            lm_head: QLinear::load(&mut weights, "language_model.lm_head", &cfg)?,
            layers,
            cfg,
        })
    }

    pub fn hidden_size(&self) -> usize {
        self.cfg.hidden as usize
    }

    /// `token_ids` 전체에 대한 마지막 RMSNorm 이후 은닉 상태 `[L, hidden]`을 f32로 돌려준다.
    pub fn hidden_states(&self, token_ids: &[u32]) -> Result<Vec<f32>, String> {
        let run = || -> R<Vec<f32>> {
            let ids = Array::from_slice(token_ids, &[token_ids.len() as i32]);
            let mut h = self.embed.dequantize_rows(&ids)?.reshape(&[1, token_ids.len() as i32, self.cfg.hidden])?;
            for layer in &self.layers {
                let normed = rms_norm(&h, &layer.input_norm, self.cfg.eps)?;
                let mixed = match &layer.mixer {
                    Mixer::Linear(m) => m.forward(&normed, &self.cfg)?,
                    Mixer::Full(m) => m.forward(&normed, &self.cfg)?,
                };
                h = ops::add(&h, &mixed)?;
                let mlp_out = layer.mlp.forward(&rms_norm(&h, &layer.post_norm, self.cfg.eps)?)?;
                h = ops::add(&h, &mlp_out)?;
            }
            let out = rms_norm(&h, &self.norm, self.cfg.eps)?.as_dtype(Dtype::Float32)?;
            out.eval()?;
            super::timing::lap("backbone_eval");
            let flat = out.as_slice::<f32>().to_vec();
            super::timing::lap("host_copy");
            Ok(flat)
        };
        run().map_err(|err| format!("MLX 백본 추론에 실패했습니다: {}", err.what()))
    }

    /// 출력 임베딩(lm_head) 8비트 행을 `ids` 순서대로 dequantize해 `[ids.len(), hidden]` f32로 돌려준다.
    pub fn lexical_rows(&self, ids: &[u32]) -> Result<Vec<f32>, String> {
        let run = || -> R<Vec<f32>> {
            let index = Array::from_slice(ids, &[ids.len() as i32]);
            let rows = self.lm_head.dequantize_rows(&index)?.as_dtype(Dtype::Float32)?;
            rows.eval()?;
            Ok(rows.as_slice::<f32>().to_vec())
        };
        run().map_err(|err| format!("MLX 출력 임베딩 조회에 실패했습니다: {}", err.what()))
    }
}

/// 샤드 이름 목록을 `model.safetensors.index.json`의 `weight_map`에서 뽑는다(중복 제거, 정렬).
pub fn shard_names(index_path: &Path) -> Result<Vec<String>, String> {
    let text = std::fs::read_to_string(index_path)
        .map_err(|err| format!("model.safetensors.index.json을 읽을 수 없습니다: {err}"))?;
    let json: serde_json::Value =
        serde_json::from_str(&text).map_err(|err| format!("index.json 파싱에 실패했습니다: {err}"))?;
    let map = json
        .get("weight_map")
        .and_then(|v| v.as_object())
        .ok_or("index.json에 weight_map이 없습니다")?;
    let mut names: Vec<String> = map.values().filter_map(|v| v.as_str().map(String::from)).collect();
    names.sort();
    names.dedup();
    Ok(names)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 게이트드 델타 규칙(mlx-lm `gated_delta_ops`의 순차 루프와 같은 수식). 테스트에서 Metal 커널
    /// 결과를 검증하는 참조 구현이다.
    ///
    /// `q`/`k`는 `[1, L, Hk, Dk]`, `v`는 `[1, L, Hv, Dv]`, `g`/`beta`는 `[1, L, Hv]`.
    /// 상태는 `[1, Hv, Dv, Dk]` f32. 스텝마다 브로드캐스트+합 대신 matmul을 써서 연산 수를 줄인다.
    fn gated_delta_rule_ops(q: &Array, k: &Array, v: &Array, g: &Array, beta: &Array, cfg: &Config) -> R<Array> {
        let len = q.shape()[1];
        let repeat = cfg.lin_v_heads / cfg.lin_k_heads;
        let (q, k) = if repeat > 1 {
            (ops::repeat_axis::<f32>(q.clone(), repeat, 2)?, ops::repeat_axis::<f32>(k.clone(), repeat, 2)?)
        } else {
            (q.clone(), k.clone())
        };

        // 헤드를 앞으로 보내고(`[1, H, L, D]`), 시간 축으로 미리 잘라 둔다.
        let rows = |a: &Array| -> R<Vec<Array>> { a.transpose_axes(&[0, 2, 1, 3])?.split_equal(len, 2) };
        let cols = |a: &Array| -> R<Vec<Array>> { a.transpose_axes(&[0, 2, 3, 1])?.split_equal(len, 3) };
        let k_rows = rows(&k)?; // [1, Hv, 1, Dk]
        let k_cols = cols(&k)?; // [1, Hv, Dk, 1]
        let q_cols = cols(&q)?;
        let v_cols = cols(v)?; // [1, Hv, Dv, 1]
        let g_steps = g.transpose_axes(&[0, 2, 1])?.reshape(&[1, cfg.lin_v_heads, len, 1, 1])?.split_equal(len, 2)?;
        let beta_steps = beta.transpose_axes(&[0, 2, 1])?.reshape(&[1, cfg.lin_v_heads, len, 1, 1])?.split_equal(len, 2)?;

        let mut state = ops::zeros::<f32>(&[1, cfg.lin_v_heads, cfg.lin_v_dim, cfg.lin_k_dim])?;
        let mut outputs = Vec::with_capacity(len as usize);
        for t in 0..len as usize {
            let decay = g_steps[t].reshape(&[1, cfg.lin_v_heads, 1, 1])?;
            let beta_t = beta_steps[t].reshape(&[1, cfg.lin_v_heads, 1, 1])?;
            state = ops::multiply(&state, &decay)?;
            let kv_mem = ops::matmul(&state, k_cols[t].as_dtype(Dtype::Float32)?)?; // [1, Hv, Dv, 1]
            let delta = ops::multiply(ops::subtract(v_cols[t].as_dtype(Dtype::Float32)?, &kv_mem)?, &beta_t)?;
            let update = ops::matmul(&delta, k_rows[t].as_dtype(Dtype::Float32)?)?; // [1, Hv, Dv, Dk]
            state = ops::add(&state, &update)?;
            outputs.push(ops::matmul(&state, q_cols[t].as_dtype(Dtype::Float32)?)?); // [1, Hv, Dv, 1]
        }
        let stacked = concatenate(&outputs, 3)?; // [1, Hv, Dv, L]
        stacked.transpose_axes(&[0, 3, 1, 2])?.as_dtype(q.dtype())
    }

    fn close(a: &[f32], b: &[f32], tol: f32) -> bool {
        a.len() == b.len() && a.iter().zip(b).all(|(x, y)| (x - y).abs() <= tol)
    }

    #[test]
    fn depthwise_causal_conv_matches_manual_sum() {
        // 채널 2개, 길이 3, 커널 2 — t번째 출력 = w[c,0]*x[t-1] + w[c,1]*x[t] (x[-1]=0).
        let qkv = Array::from_slice(&[1.0f32, 10.0, 2.0, 20.0, 3.0, 30.0], &[1, 3, 2]);
        let weight = Array::from_slice(&[1.0f32, 2.0, 3.0, 4.0], &[2, 2, 1]);
        let out = depthwise_causal_conv(&qkv, &weight, 2, 2).unwrap();
        out.eval().unwrap();
        // 채널0: w=[1,2], x=[1,2,3] → [0*1+1*2, 1*1+2*2, 2*1+3*2] = [2,5,8]
        // 채널1: w=[3,4], x=[10,20,30] → [0*3+10*4, 10*3+20*4, 20*3+30*4] = [40,110,180]
        assert!(close(out.as_slice::<f32>(), &[2.0, 40.0, 5.0, 110.0, 8.0, 180.0], 1e-4));
    }

    #[test]
    fn quantized_rows_match_dequantized_reference() {
        let w = ops::arange::<_, f32>(None, 4.0 * 64.0, None).unwrap().reshape(&[4, 64]).unwrap();
        let (qw, scales, biases) = ops::quantize(&w, 64, 8).unwrap();
        let layer = QLinear { weight: qw, scales, biases, group_size: 64, bits: 8 };
        let rows = layer.dequantize_rows(&Array::from_slice(&[2u32, 0], &[2])).unwrap();
        rows.eval().unwrap();
        let got = rows.as_slice::<f32>();
        // 8비트 양자화 오차는 그룹 범위(63)/255 이내.
        assert!(close(&got[..64], &(128..192).map(|v| v as f32).collect::<Vec<_>>(), 0.3));
        assert!(close(&got[64..], &(0..64).map(|v| v as f32).collect::<Vec<_>>(), 0.3));
    }

    #[test]
    fn gated_delta_rule_single_step_matches_closed_form() {
        // 헤드 1개, Dk=Dv=2, 한 스텝, 상태 0: kv_mem=0 → delta=v*beta → state=k⊗delta → y=state·q.
        let cfg = Config {
            hidden: 0, layers: 0, heads: 0, kv_heads: 0, head_dim: 0, rope_dims: 0, rope_theta: 0.0, eps: 0.0,
            full_attention_interval: 4, lin_k_heads: 1, lin_v_heads: 1, lin_k_dim: 2, lin_v_dim: 2,
            conv_kernel: 4, group_size: 64, bits: 8,
        };
        let q = Array::from_slice(&[1.0f32, 2.0], &[1, 1, 1, 2]);
        let k = Array::from_slice(&[3.0f32, 4.0], &[1, 1, 1, 2]);
        let v = Array::from_slice(&[5.0f32, 6.0], &[1, 1, 1, 2]);
        let g = Array::from_slice(&[0.5f32], &[1, 1, 1]);
        let beta = Array::from_slice(&[0.5f32], &[1, 1, 1]);
        let y = gated_delta_rule_ops(&q, &k, &v, &g, &beta, &cfg).unwrap();
        y.eval().unwrap();
        // delta=[2.5,3]; state[d][j]=delta[d]*k[j]; y[d]=delta[d]*(k·q)=delta[d]*11
        assert_eq!(y.shape(), &[1, 1, 1, 2]);
        assert!(close(y.as_slice::<f32>(), &[27.5, 33.0], 1e-3));
    }

    #[test]
    fn delta_kernel_matches_ops_reference() {
        // 커널이 ops 참조 구현과 같은 값을 내는지 — GQA(Hv=2*Hk), 여러 스텝, 상태 감쇠 포함.
        let cfg = Config {
            hidden: 0, layers: 0, heads: 0, kv_heads: 0, head_dim: 0, rope_dims: 0, rope_theta: 0.0, eps: 0.0,
            full_attention_interval: 4, lin_k_heads: 2, lin_v_heads: 4, lin_k_dim: 128, lin_v_dim: 8,
            conv_kernel: 4, group_size: 64, bits: 8,
        };
        let len = 6;
        let wave = |count: i32, freq: f32, shape: &[i32]| {
            let steps = ops::arange::<_, f32>(None, count as f32, None).unwrap();
            ops::sin(ops::multiply(steps, Array::from_f32(freq)).unwrap()).unwrap().reshape(shape).unwrap()
        };
        // q/k를 작게 둬 상태가 폭주하지 않게 한다(실제 경로는 L2 정규화된 값이 들어온다).
        let small = |a: Array| ops::multiply(a, Array::from_f32(0.05)).unwrap();
        let q = small(wave(len * 2 * 128, 0.37, &[1, len, 2, 128]));
        let k = small(wave(len * 2 * 128, 0.11, &[1, len, 2, 128]));
        let v = wave(len * 4 * 8, 0.53, &[1, len, 4, 8]);
        let g = ops::add(
            ops::multiply(wave(len * 4, 0.9, &[1, len, 4]), Array::from_f32(0.1)).unwrap(),
            Array::from_f32(0.8),
        )
        .unwrap();
        let beta = ops::add(
            ops::multiply(wave(len * 4, 0.7, &[1, len, 4]), Array::from_f32(0.2)).unwrap(),
            Array::from_f32(0.5),
        )
        .unwrap();
        let expected = gated_delta_rule_ops(&q, &k, &v, &g, &beta, &cfg).unwrap().contiguous().unwrap();
        let got = gated_delta_rule(&q, &k, &v, &g, &beta, &cfg).unwrap().contiguous().unwrap();
        expected.eval().unwrap();
        got.eval().unwrap();
        assert_eq!(got.shape(), expected.shape());
        assert!(close(got.as_slice::<f32>(), expected.as_slice::<f32>(), 1e-3));
    }
}
