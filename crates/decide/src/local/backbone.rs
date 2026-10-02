// Clef-flash(Qwen3.5 계열) GGUF 백본 — PR #3396(woodRock/candle,
// quantized_qwen3_5.rs, 커밋 e9e4929f3e31b6183cc17e7be0596929c6725257)을
// 벤더링해 hidden_states 추출 경로로 적응시킨다.
//
// 원본은 candle-transformers의 비공개 유틸(with_tracing::QMatMul,
// utils::repeat_kv, quantized_nn::RmsNorm)에 의존하지만, candle-transformers
// 전체를 의존성으로 끌어오지 않기로 했으므로(Task 1) 그 유틸들을
// `vendored_utils` 서브모듈로 이 파일 안에 복제한다. QMatMul은
// candle-core 0.9가 이미 공개 재노출(`candle_core::quantized::QMatMul`)하므로
// 래퍼를 복제하지 않고 그대로 쓴다.

mod vendored_utils {
    use candle_core::quantized::QTensor;
    use candle_core::{Module, Result, Tensor};

    // candle-transformers src/utils.rs::repeat_kv 그대로.
    pub fn repeat_kv(xs: Tensor, n_rep: usize) -> Result<Tensor> {
        if n_rep == 1 {
            Ok(xs)
        } else {
            let (b_sz, n_kv_head, seq_len, head_dim) = xs.dims4()?;
            Tensor::cat(&vec![&xs; n_rep], 2)?.reshape((b_sz, n_kv_head * n_rep, seq_len, head_dim))
        }
    }

    // candle-transformers src/quantized_nn.rs::RmsNorm 그대로
    // (from_qtensor 생성자가 필요해 그대로 복제).
    //
    // Task 7 조사 노트: HF `transformers`의 `Qwen3_5RMSNorm.forward`는
    // `_norm(x) * (1.0 + self.weight)`를 계산하는데, 이 GGUF(llama.cpp류
    // 변환 도구)는 그 `1.0 + weight`를 변환 시점에 미리 더해 저장해둔다 —
    // 실측으로 확인: `blk.0.attn_norm.weight[:8]`이 Python
    // `(1 + layers[0].input_layernorm.weight)[:8]`와 소수 몇 자리까지
    // 정확히 일치한다. 그래서 이 생성자에서 다시 1.0을 더하면 안 된다 —
    // 처음에 "GGUF가 raw weight를 저장했을 것"이라고 가정하고 +1.0을
    // 추가했다가, 실측 로짓이 기대값의 정확히 ~1.9~2.0배로 벌어지는 것을
    // 보고(= (1+w)를 두 번 곱한 것과 동치) 되돌렸다.
    #[derive(Debug, Clone)]
    pub struct RmsNorm {
        weight: Tensor,
        eps: f64,
    }

    impl RmsNorm {
        pub fn from_qtensor(weight: QTensor, eps: f64) -> Result<Self> {
            let weight = weight.dequantize(&weight.device())?;
            Ok(Self { weight, eps })
        }
    }

    impl Module for RmsNorm {
        fn forward(&self, x: &Tensor) -> Result<Tensor> {
            candle_nn::ops::rms_norm(x, &self.weight, self.eps as f32)
        }
    }
}

use candle_core::quantized::{gguf_file, QMatMul, QTensor};
use candle_core::{DType, Device, Module, Result as CandleResult, Tensor, D};
use candle_nn::{kv_cache::ConcatKvCache, Activation, Embedding};
use std::io::{Read, Seek};
use std::sync::Arc;
use vendored_utils::{repeat_kv, RmsNorm};

struct Gguf<R: Read + Seek> {
    ct: gguf_file::Content,
    reader: R,
    device: Device,
}

impl<R: Read + Seek> Gguf<R> {
    fn new(ct: gguf_file::Content, reader: R, device: Device) -> Self {
        Self { ct, reader, device }
    }

    fn qmatmul(&mut self, name: &str) -> CandleResult<(QMatMul, usize)> {
        let ws = self.ct.tensor(&mut self.reader, name, &self.device)?;
        let out_dim = ws.shape().dims()[0];
        Ok((QMatMul::from_qtensor(ws)?, out_dim))
    }

    fn rms_norm(&mut self, name: &str, eps: f64) -> CandleResult<RmsNorm> {
        let ws = self.ct.tensor(&mut self.reader, name, &self.device)?;
        RmsNorm::from_qtensor(ws, eps)
    }

    fn metadata(&self) -> &std::collections::HashMap<String, gguf_file::Value> {
        &self.ct.metadata
    }

    fn tensor(&mut self, name: &str) -> CandleResult<QTensor> {
        self.ct.tensor(&mut self.reader, name, &self.device)
    }
}

#[derive(Debug, Clone)]
struct MlpWeights {
    gate_proj: QMatMul,
    up_proj: QMatMul,
    down_proj: QMatMul,
    act_fn: Activation,
}

impl MlpWeights {
    fn new<R: Read + Seek>(gg: &mut Gguf<R>, prefix: &str) -> CandleResult<Self> {
        let (gate_proj, _) = gg.qmatmul(&format!("{prefix}.ffn_gate.weight"))?;
        let (up_proj, _) = gg.qmatmul(&format!("{prefix}.ffn_up.weight"))?;
        let (down_proj, _) = gg.qmatmul(&format!("{prefix}.ffn_down.weight"))?;
        Ok(Self {
            gate_proj,
            up_proj,
            down_proj,
            act_fn: Activation::Silu,
        })
    }
}

impl Module for MlpWeights {
    fn forward(&self, x: &Tensor) -> CandleResult<Tensor> {
        let gate = self.gate_proj.forward(x)?.apply(&self.act_fn)?;
        let up = self.up_proj.forward(x)?;
        let gated = (gate * up)?;
        self.down_proj.forward(&gated)
    }
}

/// Qwen3.5는 head_dim(256) 전체가 아니라 그 일부(`partial_rotary_factor=0.25` →
/// `rotary_dim=64`)에만 RoPE를 적용하고 나머지 192차원은 그대로 통과시킨다
/// (HF `transformers`의 `models/qwen3_5/modeling_qwen3_5.py::apply_rotary_pos_emb`가
/// `rotary_dim = cos.shape[-1]`로 앞쪽만 잘라 회전시키고 `q_pass`/`k_pass`를
/// 그대로 이어붙이는 것으로 직접 확인했다). GGUF 메타데이터의
/// `qwen3(5).rope.dimension_count`가 이 64를 그대로 담고 있다. Task 3가
/// 벤더링한 PR #3396 원본은 이 구분이 없어 `head_dim` 전체(256)를 회전시켰다
/// — Task 7의 parity 비교에서 이 차이가 로짓을 크게 틀어지게 하는 원인
/// 중 하나였다(실측: 수정 전 최대 오차 5.0, 수정 후 — 아래 report 참고).
#[derive(Debug, Clone)]
struct RotaryEmbedding {
    sin: Tensor,
    cos: Tensor,
    rotary_dim: usize,
}

impl RotaryEmbedding {
    fn new(
        dtype: DType,
        rotary_dim: usize,
        max_position_embeddings: usize,
        rope_theta: f64,
        dev: &Device,
    ) -> CandleResult<Self> {
        let dim = rotary_dim;
        let max_seq_len = max_position_embeddings;
        let inv_freq: Vec<_> = (0..dim)
            .step_by(2)
            .map(|i| 1f32 / rope_theta.powf(i as f64 / dim as f64) as f32)
            .collect();
        let inv_freq_len = inv_freq.len();
        let inv_freq = Tensor::from_vec(inv_freq, (1, inv_freq_len), dev)?.to_dtype(dtype)?;
        let t = Tensor::arange(0u32, max_seq_len as u32, dev)?
            .to_dtype(dtype)?
            .reshape((max_seq_len, 1))?;
        let freqs = t.matmul(&inv_freq)?;
        Ok(Self {
            sin: freqs.sin()?,
            cos: freqs.cos()?,
            rotary_dim,
        })
    }

    fn apply(&self, q: &Tensor, k: &Tensor, offset: usize) -> CandleResult<(Tensor, Tensor)> {
        let (_, _, seq_len, head_dim) = q.dims4()?;
        let cos = self.cos.narrow(0, offset, seq_len)?.to_dtype(q.dtype())?;
        let sin = self.sin.narrow(0, offset, seq_len)?.to_dtype(q.dtype())?;
        if self.rotary_dim >= head_dim {
            let q_embed = candle_nn::rotary_emb::rope(&q.contiguous()?, &cos, &sin)?;
            let k_embed = candle_nn::rotary_emb::rope(&k.contiguous()?, &cos, &sin)?;
            return Ok((q_embed, k_embed));
        }
        let pass_dim = head_dim - self.rotary_dim;
        let q_rot = q.narrow(D::Minus1, 0, self.rotary_dim)?.contiguous()?;
        let q_pass = q.narrow(D::Minus1, self.rotary_dim, pass_dim)?;
        let k_rot = k.narrow(D::Minus1, 0, self.rotary_dim)?.contiguous()?;
        let k_pass = k.narrow(D::Minus1, self.rotary_dim, pass_dim)?;
        let q_embed = candle_nn::rotary_emb::rope(&q_rot, &cos, &sin)?;
        let k_embed = candle_nn::rotary_emb::rope(&k_rot, &cos, &sin)?;
        let q_embed = Tensor::cat(&[&q_embed, &q_pass], D::Minus1)?;
        let k_embed = Tensor::cat(&[&k_embed, &k_pass], D::Minus1)?;
        Ok((q_embed, k_embed))
    }
}

#[derive(Debug, Clone)]
struct AttentionWeights {
    q_proj: QMatMul,
    k_proj: QMatMul,
    v_proj: QMatMul,
    o_proj: QMatMul,
    q_norm: RmsNorm,
    k_norm: RmsNorm,
    num_heads: usize,
    num_kv_heads: usize,
    num_kv_groups: usize,
    head_dim: usize,
    rotary_emb: Arc<RotaryEmbedding>,
    kv_cache: ConcatKvCache,
}

impl AttentionWeights {
    #[allow(clippy::too_many_arguments)]
    fn new<R: Read + Seek>(
        gg: &mut Gguf<R>,
        head_dim: usize,
        rms_norm_eps: f64,
        rotary_emb: Arc<RotaryEmbedding>,
        prefix: &str,
    ) -> CandleResult<Self> {
        let (q_proj, q_out) = gg.qmatmul(&format!("{prefix}.attn_q.weight"))?;
        let (k_proj, k_out) = gg.qmatmul(&format!("{prefix}.attn_k.weight"))?;
        let (v_proj, _) = gg.qmatmul(&format!("{prefix}.attn_v.weight"))?;
        let (o_proj, _) = gg.qmatmul(&format!("{prefix}.attn_output.weight"))?;

        let num_heads = q_out / (head_dim * 2); // Q + Gate
        let num_kv_heads = k_out / head_dim;
        let num_kv_groups = num_heads / num_kv_heads;

        let q_norm = gg.rms_norm(&format!("{prefix}.attn_q_norm.weight"), rms_norm_eps)?;
        let k_norm = gg.rms_norm(&format!("{prefix}.attn_k_norm.weight"), rms_norm_eps)?;

        Ok(Self {
            q_proj,
            k_proj,
            v_proj,
            o_proj,
            q_norm,
            k_norm,
            num_heads,
            num_kv_heads,
            num_kv_groups,
            head_dim,
            rotary_emb,
            kv_cache: ConcatKvCache::new(2),
        })
    }

    fn forward(
        &mut self,
        x: &Tensor,
        attn_mask: Option<&Tensor>,
        offset: usize,
    ) -> CandleResult<Tensor> {
        let (b, l, _) = x.dims3()?;

        let q_out = self.q_proj.forward(x)?;
        let q_out = q_out.reshape((b, l, self.num_heads, self.head_dim * 2))?;

        let q = q_out.narrow(D::Minus1, 0, self.head_dim)?.transpose(1, 2)?;
        let gate = q_out.narrow(D::Minus1, self.head_dim, self.head_dim)?;

        let k = self.k_proj.forward(x)?;
        let v = self.v_proj.forward(x)?;

        let k = k
            .reshape((b, l, self.num_kv_heads, self.head_dim))?
            .transpose(1, 2)?;
        let v = v
            .reshape((b, l, self.num_kv_heads, self.head_dim))?
            .transpose(1, 2)?;

        let q_flat = q.flatten(0, 2)?;
        let k_flat = k.flatten(0, 2)?;

        let q_flat = self.q_norm.forward(&q_flat)?;
        let k_flat = self.k_norm.forward(&k_flat)?;
        let q = q_flat.reshape((b, self.num_heads, l, self.head_dim))?;
        let k = k_flat.reshape((b, self.num_kv_heads, l, self.head_dim))?;

        let (q, k) = self.rotary_emb.apply(&q, &k, offset)?;

        let (k, v) = self.kv_cache.append(&k, &v)?;

        let k = repeat_kv(k, self.num_kv_groups)?.contiguous()?;
        let v = repeat_kv(v, self.num_kv_groups)?.contiguous()?;

        let scale = 1.0 / (self.head_dim as f64).sqrt();
        let mut scores = (q.matmul(&k.transpose(2, 3)?)? * scale)?;
        if let Some(m) = attn_mask {
            let m_dtype = m.dtype();
            let scores_dtype = scores.dtype();
            let mask = if m_dtype != scores_dtype {
                m.to_dtype(scores_dtype)?
            } else {
                m.clone()
            };
            scores = scores.broadcast_add(&mask)?;
        }
        let probs = candle_nn::ops::softmax_last_dim(&scores)?;
        let ctx = probs.matmul(&v)?;

        let ctx = ctx.transpose(1, 2)?;
        // sigmoid 게이트 적용
        let gate_sigmoid =
            candle_nn::ops::sigmoid(&gate.to_dtype(DType::F32)?)?.to_dtype(ctx.dtype())?;
        let ctx = ctx.broadcast_mul(&gate_sigmoid)?;

        let reshaped_ctx = ctx.reshape((b, l, self.num_heads * self.head_dim))?;
        self.o_proj.forward(&reshaped_ctx)
    }

    fn clear_kv_cache(&mut self) {
        self.kv_cache.reset();
    }
}

#[derive(Debug, Clone)]
struct GatedDeltaNetWeights {
    num_v_heads: usize,
    num_k_heads: usize,
    head_k_dim: usize,
    head_v_dim: usize,
    key_dim: usize,
    value_dim: usize,
    conv_dim: usize,
    conv_kernel_size: usize,

    // Task 7: 아래 네 투영과 out_proj는 `permute_v_heads_to_hf_order`로 헤드
    // 순서를 보정해야 해서(정의부 주석 참고) 양자화된 QMatMul이 아니라
    // 역양자화한 평문 Tensor로 들고 있다 — 로드 시 1회만 치환하면 되므로
    // 매 forward 호출마다 다시 계산할 필요가 없다.
    in_proj_qkv_weight: Tensor,
    in_proj_z_weight: Tensor,
    in_proj_b_weight: Tensor,
    in_proj_a_weight: Tensor,
    out_proj_weight: Tensor,

    conv1d_weight: Tensor,
    dt_bias_f32: Tensor,
    neg_a_f32: Tensor,
    norm_weight: Tensor,
    norm_eps: f64,

    conv_state: Option<Tensor>,
    recurrent_state: Option<Tensor>,
}

impl GatedDeltaNetWeights {
    #[allow(clippy::too_many_arguments)]
    fn new<R: Read + Seek>(
        gg: &mut Gguf<R>,
        num_v_heads: usize,
        num_k_heads: usize,
        head_k_dim: usize,
        head_v_dim: usize,
        conv_kernel_size: usize,
        rms_norm_eps: f64,
        prefix: &str,
    ) -> CandleResult<Self> {
        let key_dim = head_k_dim * num_k_heads;
        let value_dim = head_v_dim * num_v_heads;
        let conv_dim = key_dim * 2 + value_dim;
        let device = gg.device.clone();

        // attn_qkv.weight: (key_dim*2 + value_dim, hidden). q/k 구간(각
        // num_k_heads 단위)은 GGUF와 HF 순서가 같지만 v 구간(num_v_heads
        // 단위)은 치환이 필요하다 — Task 7에서 코사인 유사도로 직접 확인.
        let qkv_raw = gg.tensor(&format!("{prefix}.attn_qkv.weight"))?.dequantize(&device)?;
        let q_part = qkv_raw.narrow(0, 0, key_dim)?;
        let k_part = qkv_raw.narrow(0, key_dim, key_dim)?;
        let v_part = qkv_raw.narrow(0, key_dim * 2, value_dim)?;
        let v_part = permute_v_heads_to_hf_order(&v_part, num_v_heads, num_k_heads, head_v_dim)?;
        let in_proj_qkv_weight = Tensor::cat(&[&q_part, &k_part, &v_part], 0)?;

        let z_raw = gg.tensor(&format!("{prefix}.attn_gate.weight"))?.dequantize(&device)?;
        let in_proj_z_weight = permute_v_heads_to_hf_order(&z_raw, num_v_heads, num_k_heads, head_v_dim)?;

        let b_raw = gg.tensor(&format!("{prefix}.ssm_beta.weight"))?.dequantize(&device)?;
        let in_proj_b_weight = permute_v_heads_to_hf_order(&b_raw, num_v_heads, num_k_heads, 1)?;

        let a_raw = gg.tensor(&format!("{prefix}.ssm_alpha.weight"))?.dequantize(&device)?;
        let in_proj_a_weight = permute_v_heads_to_hf_order(&a_raw, num_v_heads, num_k_heads, 1)?;

        // ssm_out.weight: (hidden, value_dim) — 치환은 입력 쪽인 dim 1(열)에
        // 적용해야 한다. permute_v_heads_to_hf_order는 dim 0 기준이므로
        // 전치해서 적용한 뒤 다시 전치한다.
        let out_raw = gg.tensor(&format!("{prefix}.ssm_out.weight"))?.dequantize(&device)?;
        let out_proj_weight =
            permute_v_heads_to_hf_order(&out_raw.t()?.contiguous()?, num_v_heads, num_k_heads, head_v_dim)?
                .t()?
                .contiguous()?;

        // ssm_conv1d.weight: (conv_dim, kernel_size). q/k 채널 구간은 그대로,
        // v 채널 구간(마지막 value_dim개)은 attn_qkv.weight의 v 구간과 같은
        // 치환이 필요하다 — Task 7에서 직접 확인(코사인 유사도 ~1.0).
        let conv1d_raw = gg.tensor(&format!("{prefix}.ssm_conv1d.weight"))?.dequantize(&gg.device)?;
        let conv1d_qk = conv1d_raw.narrow(0, 0, key_dim * 2)?;
        let conv1d_v = conv1d_raw.narrow(0, key_dim * 2, value_dim)?;
        let conv1d_v = permute_v_heads_to_hf_order(&conv1d_v, num_v_heads, num_k_heads, head_v_dim)?;
        let conv1d_weight = Tensor::cat(&[&conv1d_qk, &conv1d_v], 0)?.unsqueeze(1)?;

        let a_log = gg.tensor(&format!("{prefix}.ssm_a"))?.dequantize(&gg.device)?;
        let a_log = permute_v_heads_to_hf_order(&a_log, num_v_heads, num_k_heads, 1)?;
        let dt_bias = gg
            .tensor(&format!("{prefix}.ssm_dt.bias"))?
            .dequantize(&gg.device)?;
        let dt_bias = permute_v_heads_to_hf_order(&dt_bias, num_v_heads, num_k_heads, 1)?;

        let dt_bias_f32 = dt_bias.to_dtype(DType::F32)?;
        let neg_a_f32 = (&a_log.to_dtype(DType::F32)?.exp()? * -1.0)?;

        let norm_weight = gg
            .tensor(&format!("{prefix}.ssm_norm.weight"))?
            .dequantize(&gg.device)?;

        Ok(Self {
            num_v_heads,
            num_k_heads,
            head_k_dim,
            head_v_dim,
            key_dim,
            value_dim,
            conv_dim,
            conv_kernel_size,
            in_proj_qkv_weight,
            in_proj_z_weight,
            in_proj_b_weight,
            in_proj_a_weight,
            out_proj_weight,
            conv1d_weight,
            dt_bias_f32,
            neg_a_f32,
            norm_weight,
            norm_eps: rms_norm_eps,
            conv_state: None,
            recurrent_state: None,
        })
    }

    fn l2_norm(xs: &Tensor) -> CandleResult<Tensor> {
        let eps = 1e-6;
        let norm = (xs.sqr()?.sum_keepdim(D::Minus1)? + eps)?.sqrt()?;
        xs.broadcast_div(&norm)
    }

    fn rms_norm_gated(&self, x: &Tensor, g: &Tensor) -> CandleResult<Tensor> {
        let x_dtype = x.dtype();
        let x_f32 = x.to_dtype(DType::F32)?;
        let g_f32 = g.to_dtype(DType::F32)?;

        let gate = g_f32.silu()?;
        let norm_x = (x_f32.sqr()?.mean_keepdim(D::Minus1)? + self.norm_eps)?.sqrt()?;
        let x_normed = x_f32.broadcast_div(&norm_x)?;
        (x_normed * gate)?
            .broadcast_mul(&self.norm_weight.to_dtype(DType::F32)?)?
            .to_dtype(x_dtype)
    }

    #[allow(clippy::too_many_arguments)]
    fn torch_recurrent_gated_delta_rule(
        &self,
        query: &Tensor,
        key: &Tensor,
        value: &Tensor,
        g: &Tensor,
        beta: &Tensor,
        initial_state: Option<&Tensor>,
    ) -> CandleResult<(Tensor, Tensor)> {
        let query = Self::l2_norm(query)?;
        let key = Self::l2_norm(key)?;

        let query = query.transpose(1, 2)?;
        let key = key.transpose(1, 2)?;
        let value = value.transpose(1, 2)?;
        let beta = beta.transpose(1, 2)?;
        let g = g.transpose(1, 2)?;

        let (batch_size, num_heads, seq_len, k_head_dim) = key.dims4()?;
        let v_head_dim = value.dim(3)?;

        let scale = 1.0 / (query.dim(3)? as f64).sqrt();
        let query = (query * scale)?;

        let mut core_attn_out_vec = Vec::with_capacity(seq_len);

        let mut last_recurrent_state = match initial_state {
            Some(state) => state.to_dtype(DType::F32)?,
            None => Tensor::zeros(
                (batch_size, num_heads, k_head_dim, v_head_dim),
                DType::F32,
                query.device(),
            )?,
        };

        let query_f32 = query.to_dtype(DType::F32)?;
        let key_f32 = key.to_dtype(DType::F32)?;
        let value_f32 = value.to_dtype(DType::F32)?;
        let g_exp = g.to_dtype(DType::F32)?.exp()?.unsqueeze(3)?;
        let beta_f32 = beta.to_dtype(DType::F32)?.unsqueeze(3)?;

        for i in 0..seq_len {
            let q_t = query_f32.narrow(2, i, 1)?;
            let k_t = key_f32.narrow(2, i, 1)?;
            let v_t = value_f32.narrow(2, i, 1)?;
            let g_t = g_exp.narrow(2, i, 1)?;
            let beta_t = beta_f32.narrow(2, i, 1)?;

            last_recurrent_state = last_recurrent_state.broadcast_mul(&g_t)?;

            let kv_mem = k_t.matmul(&last_recurrent_state)?;
            let delta = (&v_t - &kv_mem)?.broadcast_mul(&beta_t)?;

            let delta_k = k_t.transpose(2, 3)?.matmul(&delta)?;
            last_recurrent_state = (&last_recurrent_state + &delta_k)?;

            let out_t = q_t.matmul(&last_recurrent_state)?;
            core_attn_out_vec.push(out_t);
        }

        let core_attn_out = Tensor::cat(&core_attn_out_vec, 2)?;
        let core_attn_out = core_attn_out.transpose(1, 2)?;
        Ok((core_attn_out, last_recurrent_state))
    }

    fn forward(&mut self, hidden_states: &Tensor) -> CandleResult<Tensor> {
        let (batch_size, seq_len, _) = hidden_states.dims3()?;
        let initial_dtype = hidden_states.dtype();

        let mixed_qkv = hidden_states.broadcast_matmul(&self.in_proj_qkv_weight.t()?)?.transpose(1, 2)?;

        let z = hidden_states.broadcast_matmul(&self.in_proj_z_weight.t()?)?;
        let z = z.reshape((batch_size, seq_len, (), self.head_v_dim))?;

        let b = hidden_states.broadcast_matmul(&self.in_proj_b_weight.t()?)?;
        let a = hidden_states.broadcast_matmul(&self.in_proj_a_weight.t()?)?;

        let use_precomputed_states = self.conv_state.is_some() && seq_len == 1;

        let mixed_qkv = if use_precomputed_states {
            let conv_state = self.conv_state.as_mut().unwrap();
            let conv_state_data = Tensor::cat(&[conv_state.as_ref(), &mixed_qkv], 2)?;
            *conv_state = conv_state_data.narrow(2, 1, self.conv_kernel_size - 1)?;
            let out = conv_state_data.conv1d(&self.conv1d_weight, 0, 1, 1, self.conv_dim)?;
            candle_nn::ops::silu(&out)?
        } else {
            let pad = self.conv_kernel_size - 1;
            let padding = Tensor::zeros(
                (batch_size, self.conv_dim, pad),
                mixed_qkv.dtype(),
                mixed_qkv.device(),
            )?;
            let padded_qkv = Tensor::cat(&[&padding, &mixed_qkv], 2)?;
            self.conv_state = Some(padded_qkv.narrow(2, seq_len, pad)?);
            let out = padded_qkv.conv1d(&self.conv1d_weight, 0, 1, 1, self.conv_dim)?;
            candle_nn::ops::silu(&out)?
        };

        let mixed_qkv = mixed_qkv.transpose(1, 2)?;

        let q = mixed_qkv.narrow(D::Minus1, 0, self.key_dim)?;
        let k = mixed_qkv.narrow(D::Minus1, self.key_dim, self.key_dim)?;
        let v = mixed_qkv.narrow(D::Minus1, self.key_dim * 2, self.value_dim)?;

        let q = q.reshape((batch_size, seq_len, (), self.head_k_dim))?;
        let k = k.reshape((batch_size, seq_len, (), self.head_k_dim))?;
        let v = v.reshape((batch_size, seq_len, (), self.head_v_dim))?;

        let beta = candle_nn::ops::sigmoid(&b)?;
        let g = {
            let a_f32 = a.to_dtype(DType::F32)?;
            let a_plus_dt = a_f32.broadcast_add(&self.dt_bias_f32)?;
            let softplus = (a_plus_dt.exp()? + 1.0)?.log()?;
            self.neg_a_f32.broadcast_mul(&softplus)?
        };

        let repeat_n = self.num_v_heads / self.num_k_heads;
        let q = repeat_interleave(&q, repeat_n, 2)?;
        let k = repeat_interleave(&k, repeat_n, 2)?;

        let initial_state = if use_precomputed_states {
            self.recurrent_state.as_ref()
        } else {
            None
        };

        let (core_attn_out, new_state) =
            self.torch_recurrent_gated_delta_rule(&q, &k, &v, &g, &beta, initial_state)?;
        self.recurrent_state = Some(new_state);

        let core_attn_out = core_attn_out.to_dtype(initial_dtype)?;
        let core_attn_out =
            core_attn_out.reshape((batch_size, seq_len, self.num_v_heads, self.head_v_dim))?;
        let core_attn_out = core_attn_out.reshape(((), self.head_v_dim))?;
        let z_flat = z.reshape(((), self.head_v_dim))?;
        let core_attn_out = self.rms_norm_gated(&core_attn_out, &z_flat)?;
        let core_attn_out =
            core_attn_out.reshape((batch_size, seq_len, self.num_v_heads * self.head_v_dim))?;

        core_attn_out.broadcast_matmul(&self.out_proj_weight.t()?)
    }

    fn clear_kv_cache(&mut self) {
        self.conv_state = None;
        self.recurrent_state = None;
    }
}

/// Clef-flash의 GGUF 변환(llama.cpp류 도구)은 GatedDeltaNet의 `num_v_heads`(32)
/// 단위로 쌓인 텐서(`attn_gate.weight`, `attn_qkv.weight`의 v 구간,
/// `ssm_conv1d.weight`의 v 구간, `ssm_out.weight`의 입력(열) 쪽 v 구간,
/// `ssm_beta.weight`, `ssm_alpha.weight`, `ssm_dt.bias`, `ssm_a`)를 HF
/// `transformers`와 다른 헤드
/// 순서로 저장한다. HF는 자연 순서(0..32)로 두지만, GGUF는
/// `(num_v_heads/num_k_heads)`개 그룹으로 먼저 나눠(이 모델은 32/16=2그룹)
/// "그룹 0의 모든 헤드 다음 그룹 1의 모든 헤드" 순서로 재배열한다 — 즉
/// HF 헤드 i(0-based)는 GGUF 헤드 `(i % repeat) * num_k_heads + i / repeat`에
/// 저장되어 있다. Task 7에서 실제 safetensors와 GGUF를 코사인 유사도로
/// 직접 대조해 이 치환을 역설계했다(`ssm_beta.weight`의 HF 32행과 GGUF
/// 32행 사이 최적 매칭이 정확히 이 패턴이었다 — 다른 어떤 텐서명 혼선이나
/// 전치 문제가 아니라 이 치환 하나였다). q/k(`num_k_heads`=16 단위) 텐서와
/// `ssm_norm.weight`(헤드 공유, `head_v_dim`=128 크기)는 이 문제가 없다 —
/// 둘 다 직접 대조로 확인했다.
///
/// `tensor`는 dim 0이 `num_v_heads * unit`인 2D 이상 텐서(또는 1D 벡터,
/// unit=1)다. 반환값은 dim 0을 HF 순서로 재배열한 텐서.
fn permute_v_heads_to_hf_order(
    tensor: &Tensor,
    num_v_heads: usize,
    num_k_heads: usize,
    unit: usize,
) -> CandleResult<Tensor> {
    let repeat = num_v_heads / num_k_heads;
    let mut indices = Vec::with_capacity(num_v_heads * unit);
    for hf_head in 0..num_v_heads {
        let gguf_head = (hf_head % repeat) * num_k_heads + hf_head / repeat;
        for sub in 0..unit {
            indices.push((gguf_head * unit + sub) as u32);
        }
    }
    let index_tensor = Tensor::from_vec(indices, (num_v_heads * unit,), tensor.device())?;
    tensor.index_select(&index_tensor, 0)
}

fn repeat_interleave(img: &Tensor, repeats: usize, dim: usize) -> CandleResult<Tensor> {
    if repeats == 1 {
        return Ok(img.clone());
    }
    let mut dims = img.dims().to_vec();
    dims[dim] *= repeats;
    let final_dims = dims.clone();
    let img = img.unsqueeze(dim + 1)?;
    let mut expand_dims = img.dims().to_vec();
    expand_dims[dim + 1] = repeats;
    let expanded = img.expand(expand_dims.as_slice())?;
    expanded.reshape(final_dims.as_slice())
}

#[derive(Debug, Clone)]
enum TokenMixer {
    FullAttention(AttentionWeights),
    LinearAttention(GatedDeltaNetWeights),
}

#[derive(Debug, Clone)]
struct LayerWeights {
    token_mixer: TokenMixer,
    mlp: MlpWeights,
    input_layernorm: RmsNorm,
    post_attention_layernorm: RmsNorm,
}

impl LayerWeights {
    #[allow(clippy::too_many_arguments)]
    fn new<R: Read + Seek>(
        gg: &mut Gguf<R>,
        layer_idx: usize,
        head_dim: usize,
        rms_norm_eps: f64,
        rotary: Arc<RotaryEmbedding>,
        full_attention_interval: usize,
        // 선형 어텐션(GatedDeltaNet) 전용 파라미터
        linear_num_key_heads: usize,
        linear_num_value_heads: usize,
        linear_key_head_dim: usize,
        linear_value_head_dim: usize,
        linear_conv_kernel_dim: usize,
    ) -> CandleResult<Self> {
        let prefix = format!("blk.{layer_idx}");
        let is_full_attention = (layer_idx + 1) % full_attention_interval == 0;

        let token_mixer = if is_full_attention {
            TokenMixer::FullAttention(AttentionWeights::new(
                gg,
                head_dim,
                rms_norm_eps,
                rotary,
                &prefix,
            )?)
        } else {
            TokenMixer::LinearAttention(GatedDeltaNetWeights::new(
                gg,
                linear_num_value_heads,
                linear_num_key_heads,
                linear_key_head_dim,
                linear_value_head_dim,
                linear_conv_kernel_dim,
                rms_norm_eps,
                &prefix,
            )?)
        };

        let mlp = MlpWeights::new(gg, &prefix)?;
        let input_layernorm = gg.rms_norm(&format!("{prefix}.attn_norm.weight"), rms_norm_eps)?;
        let post_attention_layernorm =
            gg.rms_norm(&format!("{prefix}.post_attention_norm.weight"), rms_norm_eps)?;

        Ok(Self {
            token_mixer,
            mlp,
            input_layernorm,
            post_attention_layernorm,
        })
    }

    fn forward(&mut self, x: &Tensor, mask: Option<&Tensor>, offset: usize) -> CandleResult<Tensor> {
        let residual = x;
        let x = self.input_layernorm.forward(x)?;
        let x = match &mut self.token_mixer {
            TokenMixer::FullAttention(attn) => attn.forward(&x, mask, offset)?,
            TokenMixer::LinearAttention(attn) => attn.forward(&x)?,
        };
        let x = (residual + x)?;

        let residual = &x;
        let x = self.post_attention_layernorm.forward(&x)?;
        let x = x.apply(&self.mlp)?;
        x + residual
    }

    fn clear_kv_cache(&mut self) {
        match &mut self.token_mixer {
            TokenMixer::FullAttention(attn) => attn.clear_kv_cache(),
            TokenMixer::LinearAttention(attn) => attn.clear_kv_cache(),
        }
    }
}

// PR #3396 원본의 ModelWeights와 동형. embed_tokens/layers/norm은 같은 파일 안
// Backbone::hidden_states에서 모듈 내부 접근으로 직접 쓴다(원본은 전부
// private). lm_head는 이 경로(hidden_states 추출)에서 쓰지 않지만 필드로는
// 유지한다 — 추후 다른 경로가 필요해지면 재사용할 수 있고, GGUF에 없을 수도
// 있는 output.weight 로드 실패 시 token_embd.weight로 대체하는 원본 로직도
// 그대로 둔다. output_embeddings는 Task 7에서 joint_head의
// option_lexical_projection이 필요로 하는 역양자화된 출력 임베딩 행렬이다.
#[derive(Debug, Clone)]
struct ModelWeights {
    embed_tokens: Embedding,
    layers: Vec<LayerWeights>,
    norm: RmsNorm,
    #[allow(dead_code)]
    lm_head: QMatMul,
    output_embeddings: Tensor,
    device: Device,
    dtype: DType,
    hidden_size: usize,
}

impl ModelWeights {
    fn from_gguf<R: Read + Seek>(
        ct: gguf_file::Content,
        reader: &mut R,
        device: &Device,
    ) -> CandleResult<Self> {
        let mut gg = Gguf::new(ct, reader, device.clone());
        // 실제 Clef-flash GGUF(prithivMLmods/clef-flash-GGUF)의 메타데이터
        // 키는 전부 `qwen35.*`로만 온다(`qwen3.*`는 등장하지 않음) — 헤더
        // 20MB를 받아 직접 확인했다(Task 3 Step 4). 그래도 `qwen3.`을 먼저
        // 찾고 없으면 `qwen35.`로 치환하는 PR #3396의 폴백은 그대로 둔다 —
        // 매 키 조회마다 1회의 추가 HashMap 조회만 더할 뿐 동작은 올바르다.
        let md_get = |s: &str| match gg.metadata().get(s) {
            Some(v) => Ok(v),
            None => {
                let s35 = s.replace("qwen3.", "qwen35.");
                match gg.metadata().get(&s35) {
                    Some(v) => Ok(v),
                    None => candle_core::bail!("cannot find {s} or {s35} in metadata"),
                }
            }
        };

        let key_length = md_get("qwen3.attention.key_length")?.to_u32()? as usize;

        let head_dim = key_length;
        let num_layers = md_get("qwen3.block_count")?.to_u32()? as usize;
        let hidden_size = md_get("qwen3.embedding_length")?.to_u32()? as usize;
        let max_position_embeddings = md_get("qwen3.context_length")?.to_u32()? as usize;
        let rms_norm_eps = md_get("qwen3.attention.layer_norm_rms_epsilon")?.to_f32()? as f64;
        let rope_freq_base = md_get("qwen3.rope.freq_base")?.to_f32()? as f64;
        // Qwen3.5는 head_dim(256) 전체가 아니라 이 값(64, partial_rotary_factor=0.25
        // 적용 결과)만큼만 회전시킨다 — RotaryEmbedding 정의부 주석 참고.
        let rope_dimension_count = md_get("qwen3.rope.dimension_count")?.to_u32()? as usize;

        let full_attention_interval = md_get("qwen3.full_attention_interval")?.to_u32()? as usize;

        // 선형 어텐션(GatedDeltaNet) 전용 메타데이터
        let linear_num_key_heads = md_get("qwen3.ssm.group_count")?.to_u32()? as usize;
        let ssm_inner_size = md_get("qwen3.ssm.inner_size")?.to_u32()? as usize;
        let linear_key_head_dim = md_get("qwen3.ssm.state_size")?.to_u32()? as usize;
        let linear_value_head_dim = linear_key_head_dim;
        let linear_num_value_heads = ssm_inner_size / linear_value_head_dim;
        let linear_conv_kernel_dim = md_get("qwen3.ssm.conv_kernel")?.to_u32()? as usize;

        let dtype = match gg.metadata().get("general.dtype") {
            Some(v) => match v.to_u32() {
                Ok(0) => DType::F32,
                Ok(1) => DType::F16,
                _ => DType::F16,
            },
            None => DType::F16,
        };

        let embed_tensor = gg.tensor("token_embd.weight")?;
        let embed_tokens = Embedding::new(embed_tensor.dequantize(device)?, hidden_size);

        let rotary = Arc::new(RotaryEmbedding::new(
            dtype,
            rope_dimension_count,
            max_position_embeddings,
            rope_freq_base,
            device,
        )?);

        let mut layers = Vec::with_capacity(num_layers);
        for i in 0..num_layers {
            layers.push(LayerWeights::new(
                &mut gg,
                i,
                head_dim,
                rms_norm_eps,
                rotary.clone(),
                full_attention_interval,
                linear_num_key_heads,
                linear_num_value_heads,
                linear_key_head_dim,
                linear_value_head_dim,
                linear_conv_kernel_dim,
            )?);
        }

        let norm = gg.rms_norm("output_norm.weight", rms_norm_eps)?;
        let lm_head_tensor = match gg.tensor("output.weight") {
            Ok(tensor) => tensor,
            Err(_) => gg.tensor("token_embd.weight")?,
        };
        // joint_head의 option_lexical_projection은 역양자화된 `(vocab, hidden)`
        // 출력 임베딩 행렬에서 토큰별 행을 직접 모아 평균을 낸다(실제 Python
        // 소스의 `output_embedding_weight[token_ids].mean(dim=0)`) — QMatMul의
        // 전치 matmul 경로가 아니라 `index_select`로 바로 접근해야 하므로,
        // lm_head(QMatMul)와는 별개로 평문 Tensor를 하나 더 들고 있는다.
        let output_embeddings = lm_head_tensor.dequantize(device)?;
        let lm_head = QMatMul::from_qtensor(lm_head_tensor)?;

        Ok(Self {
            embed_tokens,
            layers,
            norm,
            lm_head,
            output_embeddings,
            device: device.clone(),
            dtype,
            hidden_size,
        })
    }

    fn causal_mask(&self, b: usize, tgt: usize, offset: usize) -> CandleResult<Tensor> {
        let minf = f32::NEG_INFINITY;
        let mask: Vec<_> = (0..tgt)
            .flat_map(|i| {
                (0..(tgt + offset)).map(move |j| if j <= i + offset { 0. } else { minf })
            })
            .collect();
        Tensor::from_slice(&mask, (b, 1, tgt, tgt + offset), &self.device)?.to_dtype(self.dtype)
    }
}

/// Clef-flash(Qwen3.5 계열) GGUF 백본. PR #3396의 `ModelWeights::forward`는
/// norm 통과 뒤 마지막 토큰만 narrow해 `lm_head`에 통과시키지만, joint_head는
/// 전체 시퀀스 hidden_states가 필요하므로 그 narrow 이전 단계에서 멈추는
/// `hidden_states`를 쓴다.
pub struct Backbone {
    inner: ModelWeights,
}

impl Backbone {
    pub fn from_gguf_path(path: &std::path::Path) -> Result<Self, String> {
        let mut file = std::fs::File::open(path)
            .map_err(|err| format!("GGUF 파일을 열 수 없습니다: {err}"))?;
        let content = gguf_file::Content::read(&mut file)
            .map_err(|err| format!("GGUF 헤더를 읽을 수 없습니다: {err}"))?;
        let device = Device::Cpu;
        let inner = ModelWeights::from_gguf(content, &mut file, &device)
            .map_err(|err| format!("Clef-flash 백본 로드에 실패했습니다: {err}"))?;
        Ok(Self { inner })
    }

    pub fn hidden_size(&self) -> usize {
        self.inner.hidden_size
    }

    /// 역양자화된 `(vocab_size, hidden_size)` 출력 임베딩 행렬. joint_head의
    /// option_lexical_projection이 토큰별 문맥화되지 않은 임베딩을 평균 내는
    /// 데 쓴다(실제 Python 소스 `output_embedding_weight[token_ids].mean(dim=0)`).
    pub fn output_embeddings(&self) -> &Tensor {
        &self.inner.output_embeddings
    }

    pub fn hidden_states(&mut self, input_ids: &Tensor) -> Result<Tensor, String> {
        // Task 7: `Runtime`(local/mod.rs)이 `Backbone`을 `OnceLock` 뒤에서 재사용하므로,
        // 레이어별 KV 캐시(ConcatKvCache/GatedDeltaNet 상태)를 지우지 않으면
        // 두 번째 호출의 시퀀스가 첫 번째 호출의 캐시 위에 이어붙어 길이가
        // 어긋난다(실측: parity 테스트에서 두 번째 입력이 `[1,16,126,273]`처럼
        // 이전 호출 길이 147과 섞인 273으로 깨짐). 이 메서드가 매번 완전히
        // 새로운(멀티턴 생성이 아닌) 시퀀스를 받으므로, 매 호출 시작에서
        // 모든 레이어의 캐시를 리셋한다.
        for layer in &mut self.inner.layers {
            layer.clear_kv_cache();
        }
        let (b, l) = input_ids.dims2().map_err(|err| err.to_string())?;
        let mut h = self
            .inner
            .embed_tokens
            .forward(input_ids)
            .map_err(|err| err.to_string())?;
        let causal_mask = if l == 1 {
            None
        } else {
            Some(self.inner.causal_mask(b, l, 0).map_err(|err| err.to_string())?)
        };
        for layer in &mut self.inner.layers {
            h = layer
                .forward(&h, causal_mask.as_ref(), 0)
                .map_err(|err| err.to_string())?;
        }
        self.inner.norm.forward(&h).map_err(|err| err.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use candle_core::{DType, Device, Tensor};

    #[test]
    fn rotary_embedding_apply_preserves_shape() {
        let device = Device::Cpu;
        let rotary = RotaryEmbedding::new(DType::F32, 128, 4096, 10_000_000.0, &device).unwrap();
        let q = Tensor::zeros((1, 2, 3, 128), DType::F32, &device).unwrap();
        let k = Tensor::zeros((1, 2, 3, 128), DType::F32, &device).unwrap();
        let (q2, k2) = rotary.apply(&q, &k, 0).unwrap();
        assert_eq!(q2.dims(), q.dims());
        assert_eq!(k2.dims(), k.dims());
    }

    #[test]
    fn partial_rotary_embedding_leaves_the_tail_dims_untouched() {
        // Qwen3.5의 partial_rotary_factor=0.25 케이스: head_dim=256인데
        // rotary_dim=64만 회전시키고 나머지 192차원은 입력 그대로 통과해야
        // 한다 — RotaryEmbedding 정의부 주석 참고.
        let device = Device::Cpu;
        let rotary_dim = 64;
        let head_dim = 256;
        let rotary = RotaryEmbedding::new(DType::F32, rotary_dim, 4096, 10_000_000.0, &device).unwrap();
        let q = Tensor::randn(0f32, 1f32, (1, 2, 3, head_dim), &device).unwrap();
        let k = Tensor::randn(0f32, 1f32, (1, 2, 3, head_dim), &device).unwrap();
        let (q2, k2) = rotary.apply(&q, &k, 0).unwrap();
        assert_eq!(q2.dims(), q.dims());
        assert_eq!(k2.dims(), k.dims());

        let pass_dim = head_dim - rotary_dim;
        let q_pass_before = q.narrow(3, rotary_dim, pass_dim).unwrap().flatten_all().unwrap().to_vec1::<f32>().unwrap();
        let q_pass_after = q2.narrow(3, rotary_dim, pass_dim).unwrap().flatten_all().unwrap().to_vec1::<f32>().unwrap();
        assert_eq!(q_pass_before, q_pass_after, "회전 적용 뒤에도 뒷부분(pass-through)은 입력과 같아야 한다");

        let q_rot_before = q.narrow(3, 0, rotary_dim).unwrap().flatten_all().unwrap().to_vec1::<f32>().unwrap();
        let q_rot_after = q2.narrow(3, 0, rotary_dim).unwrap().flatten_all().unwrap().to_vec1::<f32>().unwrap();
        assert_ne!(q_rot_before, q_rot_after, "앞부분(rotary)은 회전으로 값이 바뀌어야 한다");
    }

    #[test]
    fn repeat_kv_identity_when_groups_is_one() {
        let device = Device::Cpu;
        let x = Tensor::zeros((1, 4, 3, 8), DType::F32, &device).unwrap();
        let out = repeat_kv(x.clone(), 1).unwrap();
        assert_eq!(out.dims(), x.dims());
    }
}
