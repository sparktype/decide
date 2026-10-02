// Clef-flash JointSchemaHead — EvidenceRoutingLayer × routing_layers +
// TransformerDecoderLayer × layers + prior/joint 잔차 스코어러. Task 3의
// Backbone이 내놓는 hidden_states 위에서 질문/옵션 스팬별 로짓을 뽑아낸다.
//
// Task 4에서는 이 구조를 모델 카드 설명(프로즈 요약)만으로 역설계했다 —
// 여섯 개 투영(memory/question/option_question/global/option_context/
// option_lexical_projection)과 type_embedding을 필드로는 만들었지만
// forward 로직은 question_projection·option_context_projection·
// memory_projection만 실제로 썼다. Task 7에서 Cloudflare/clef-flash
// HuggingFace 레포의 실제 소스
// (https://huggingface.co/Cloudflare/clef-flash/raw/main/joint_schema_model.py,
// `JointSchemaHead.forward()`)를 직접 받아 대조한 결과, 이전 구현은 두
// 종류의 문제가 있었다:
//
// 1. 네 개 필드(option_question_projection, global_projection,
//    option_lexical_projection, type_embedding)가 실제로는 전부 forward에서
//    쓰인다 — "학습 때만 쓰고 추론 때는 안 쓴다"는 가능성은 실제 소스를 보니
//    해당하지 않았다. 아래 `score()`가 이 네 필드를 모두 실제 조합대로
//    연결한다 — 자세한 매핑은 아래 각 단계 주석 참고.
// 2. safetensors의 실제 파라미터 이름이 Task 4가 지어낸 이름과 달랐다.
//    `evidence_layers.{i}.attn.q_proj.weight` 같은 이름은 존재하지 않고,
//    실제로는 PyTorch `nn.MultiheadAttention`이 쓰는 패킹된 이름
//    (`attention.in_proj_weight`/`in_proj_bias` + `attention.out_proj.*`,
//    디코더 레이어는 `self_attn`/`multihead_attn` 두 개)이다. 이 파일의
//    `safetensors_path`에서 직접 헤더를 읽어 전체 122개 키를 확인했다 — 그
//    결과를 아래 `PackedAttention::new`/`EvidenceRoutingLayer::new`/
//    `TransformerDecoderLayer::new`의 `vb.pp(...)` 경로에 반영했다. 또한
//    6개 투영과 type_embedding을 제외한 거의 모든 Linear/LayerNorm이
//    `bias=true`다(PyTorch 기본값) — Task 4는 전부 `linear_no_bias`로
//    가정했는데 틀렸다.
// 3. GELU는 PyTorch 기본(`approximate='none'`, 정확한 erf 기반)인데 Task 4는
//    candle의 tanh 근사(`Tensor::gelu`)를 썼다 — `gelu_erf()`로 바꿨다.
//
// candle-nn 0.9.2에는 `MultiheadAttention`도 `transformer::TransformerDecoderLayer`도
// 없다(두 심볼 모두 설치된 crate 소스에 없음을 직접 확인했다) — 그래서 이
// 파일 안에서 PyTorch의 패킹된 in_proj_weight/bias 레이아웃과 수학적으로
// 동일한 scaled-dot-product attention을 손으로 구성했다(`PackedAttention`).
use crate::local::tokenizer::{OptionSpan, QuestionSpan};
use candle_core::{DType, Result as CandleResult, Tensor, D};
use candle_nn::{layer_norm, linear, linear_no_bias, Embedding, LayerNorm, Linear, Module, VarBuilder};

/// PyTorch `nn.MultiheadAttention`과 동일한 패킹된 `in_proj_weight`
/// `(3*width, width)` / `in_proj_bias (3*width,)` 레이아웃. 행 `[0, width)`가
/// query, `[width, 2*width)`가 key, `[2*width, 3*width)`가 value 투영이다 —
/// PyTorch의 `_in_projection_packed`가 self-attention이든 cross-attention
/// (key is value)이든 수학적으로 이 분할과 동일한 결과를 낸다(공식
/// torch 소스로 직접 확인).
struct PackedAttention {
    q_proj: Linear,
    k_proj: Linear,
    v_proj: Linear,
    out_proj: Linear,
    heads: usize,
    head_dim: usize,
}

impl PackedAttention {
    fn new(width: usize, heads: usize, vb: VarBuilder) -> CandleResult<Self> {
        let in_proj_weight = vb.get((3 * width, width), "in_proj_weight")?;
        let in_proj_bias = vb.get(3 * width, "in_proj_bias")?;
        let slice = |offset: usize| -> CandleResult<Linear> {
            let w = in_proj_weight.narrow(0, offset, width)?.contiguous()?;
            let b = in_proj_bias.narrow(0, offset, width)?.contiguous()?;
            Ok(Linear::new(w, Some(b)))
        };
        Ok(Self {
            q_proj: slice(0)?,
            k_proj: slice(width)?,
            v_proj: slice(2 * width)?,
            out_proj: linear(width, width, vb.pp("out_proj"))?,
            heads,
            head_dim: width / heads,
        })
    }

    fn split_heads(&self, x: &Tensor, batch: usize, len: usize) -> CandleResult<Tensor> {
        x.reshape((batch, len, self.heads, self.head_dim))?
            .transpose(1, 2)?
            .contiguous()
    }

    fn forward(&self, query: &Tensor, key: &Tensor, value: &Tensor) -> CandleResult<Tensor> {
        let (b, lq, _) = query.dims3()?;
        let (_, lk, _) = key.dims3()?;

        let q = self.split_heads(&self.q_proj.forward(query)?, b, lq)?;
        let k = self.split_heads(&self.k_proj.forward(key)?, b, lk)?;
        let v = self.split_heads(&self.v_proj.forward(value)?, b, lk)?;

        let scale = 1.0 / (self.head_dim as f64).sqrt();
        let scores = (q.matmul(&k.transpose(2, 3)?.contiguous()?)? * scale)?;
        let probs = candle_nn::ops::softmax_last_dim(&scores)?;
        let ctx = probs.matmul(&v)?;

        let ctx = ctx
            .transpose(1, 2)?
            .contiguous()?
            .reshape((b, lq, self.heads * self.head_dim))?;
        self.out_proj.forward(&ctx)
    }
}

struct EvidenceRoutingLayer {
    query_norm: LayerNorm,
    memory_norm: LayerNorm,
    attention: PackedAttention,
    feedforward_norm: LayerNorm,
    ff1: Linear,
    ff2: Linear,
}

impl EvidenceRoutingLayer {
    fn new(width: usize, heads: usize, feedforward: usize, vb: VarBuilder) -> CandleResult<Self> {
        Ok(Self {
            query_norm: layer_norm(width, 1e-5, vb.pp("query_norm"))?,
            memory_norm: layer_norm(width, 1e-5, vb.pp("memory_norm"))?,
            attention: PackedAttention::new(width, heads, vb.pp("attention"))?,
            feedforward_norm: layer_norm(width, 1e-5, vb.pp("feedforward_norm"))?,
            ff1: linear(width, feedforward, vb.pp("feedforward").pp(0))?,
            ff2: linear(feedforward, width, vb.pp("feedforward").pp(3))?,
        })
    }

    fn forward(&self, query: &Tensor, memory: &Tensor) -> CandleResult<Tensor> {
        let normalized_queries = self.query_norm.forward(query)?;
        let normalized_memory = self.memory_norm.forward(memory)?;
        let routed = self
            .attention
            .forward(&normalized_queries, &normalized_memory, &normalized_memory)?;
        let queries = (query + routed)?;
        let ff_input = self.feedforward_norm.forward(&queries)?;
        let ff = self.ff2.forward(&self.ff1.forward(&ff_input)?.gelu_erf()?)?;
        queries + ff
    }
}

/// PyTorch `nn.TransformerDecoderLayer(norm_first=True, activation="gelu")`와
/// 동일한 pre-norm 구조: self-attn → cross-attn → feedforward, 각각 잔차 연결.
struct TransformerDecoderLayer {
    norm1: LayerNorm,
    self_attn: PackedAttention,
    norm2: LayerNorm,
    cross_attn: PackedAttention,
    norm3: LayerNorm,
    linear1: Linear,
    linear2: Linear,
}

impl TransformerDecoderLayer {
    fn new(width: usize, heads: usize, feedforward: usize, vb: VarBuilder) -> CandleResult<Self> {
        Ok(Self {
            norm1: layer_norm(width, 1e-5, vb.pp("norm1"))?,
            self_attn: PackedAttention::new(width, heads, vb.pp("self_attn"))?,
            norm2: layer_norm(width, 1e-5, vb.pp("norm2"))?,
            cross_attn: PackedAttention::new(width, heads, vb.pp("multihead_attn"))?,
            norm3: layer_norm(width, 1e-5, vb.pp("norm3"))?,
            linear1: linear(width, feedforward, vb.pp("linear1"))?,
            linear2: linear(feedforward, width, vb.pp("linear2"))?,
        })
    }

    fn forward(&self, tgt: &Tensor, memory: &Tensor) -> CandleResult<Tensor> {
        let normed = self.norm1.forward(tgt)?;
        let attended = self.self_attn.forward(&normed, &normed, &normed)?;
        let x = (tgt + attended)?;

        let normed = self.norm2.forward(&x)?;
        let attended = self.cross_attn.forward(&normed, memory, memory)?;
        let x = (&x + attended)?;

        let normed = self.norm3.forward(&x)?;
        let ff = self.linear2.forward(&self.linear1.forward(&normed)?.gelu_erf()?)?;
        &x + ff
    }
}

pub struct JointHead {
    hidden_norm: LayerNorm,
    memory_projection: Linear,
    question_projection: Linear,
    option_question_projection: Linear,
    global_projection: Linear,
    option_context_projection: Linear,
    option_lexical_projection: Linear,
    type_embedding: Embedding,
    evidence_layers: Vec<EvidenceRoutingLayer>,
    option_summary_norm: LayerNorm,
    decoder_layers: Vec<TransformerDecoderLayer>,
    field_norm: LayerNorm,
    option_norm: LayerNorm,
    residual_scorer_1: Linear,
    residual_scorer_2: Linear,
    /// `exp(clamp(prior_logit_scale, max=ln(100)))` — 추론 중 값이 바뀌지
    /// 않으므로 로드 시점에 스칼라로 미리 계산해 둔다(실제 forward에서도
    /// 매 호출마다 다시 계산할 필요가 없는 상수).
    prior_scale: f32,
    joint_scale: f32,
    /// `sigmoid(residual_gate)`.
    residual_gate: f32,
    width: usize,
}

fn scalar_scale(vb: &VarBuilder, name: &str, clamp_max: f64) -> CandleResult<f32> {
    let raw = vb.get((), name)?;
    let clamped = raw.clamp(f64::NEG_INFINITY, clamp_max)?;
    clamped.exp()?.to_dtype(DType::F32)?.to_scalar::<f32>()
}

fn scalar_sigmoid(vb: &VarBuilder, name: &str) -> CandleResult<f32> {
    let raw = vb.get((), name)?;
    let sig = candle_nn::ops::sigmoid(&raw.unsqueeze(0)?)?.squeeze(0)?;
    sig.to_dtype(DType::F32)?.to_scalar::<f32>()
}

impl JointHead {
    #[cfg(test)]
    pub fn new_for_test(
        vb: VarBuilder,
        hidden_size: usize,
        width: usize,
        routing_layers: usize,
        layers: usize,
        heads: usize,
        feedforward: usize,
    ) -> CandleResult<Self> {
        Self::new(vb, hidden_size, width, routing_layers, layers, heads, feedforward)
    }

    fn new(
        vb: VarBuilder,
        hidden_size: usize,
        width: usize,
        routing_layers: usize,
        layers: usize,
        heads: usize,
        feedforward: usize,
    ) -> CandleResult<Self> {
        let hidden_norm = layer_norm(hidden_size, 1e-5, vb.pp("hidden_norm"))?;
        let memory_projection = linear_no_bias(hidden_size, width, vb.pp("memory_projection"))?;
        let question_projection = linear_no_bias(hidden_size, width, vb.pp("question_projection"))?;
        let option_question_projection =
            linear_no_bias(hidden_size, width, vb.pp("option_question_projection"))?;
        let global_projection = linear_no_bias(hidden_size, width, vb.pp("global_projection"))?;
        let option_context_projection =
            linear_no_bias(hidden_size, width, vb.pp("option_context_projection"))?;
        let option_lexical_projection =
            linear_no_bias(hidden_size, width, vb.pp("option_lexical_projection"))?;
        let type_embedding = candle_nn::embedding(3, width, vb.pp("type_embedding"))?;

        let mut evidence_layers = Vec::with_capacity(routing_layers);
        let vb_evidence = vb.pp("evidence_layers");
        for i in 0..routing_layers {
            evidence_layers.push(EvidenceRoutingLayer::new(width, heads, feedforward, vb_evidence.pp(i))?);
        }

        let option_summary_norm = layer_norm(width, 1e-5, vb.pp("option_summary_norm"))?;

        let mut decoder_layers = Vec::with_capacity(layers);
        let vb_layers = vb.pp("layers");
        for i in 0..layers {
            decoder_layers.push(TransformerDecoderLayer::new(width, heads, feedforward, vb_layers.pp(i))?);
        }

        let field_norm = layer_norm(width, 1e-5, vb.pp("field_norm"))?;
        let option_norm = layer_norm(width, 1e-5, vb.pp("option_norm"))?;
        let residual_scorer_1 = linear(width * 4, width, vb.pp("residual_scorer").pp(0))?;
        let residual_scorer_2 = linear(width, 1, vb.pp("residual_scorer").pp(3))?;

        let hundred_ln = 100f64.ln();
        let prior_scale = scalar_scale(&vb, "prior_logit_scale", hundred_ln)?;
        let joint_scale = scalar_scale(&vb, "joint_logit_scale", hundred_ln)?;
        let residual_gate = scalar_sigmoid(&vb, "residual_gate")?;

        Ok(Self {
            hidden_norm,
            memory_projection,
            question_projection,
            option_question_projection,
            global_projection,
            option_context_projection,
            option_lexical_projection,
            type_embedding,
            evidence_layers,
            option_summary_norm,
            decoder_layers,
            field_norm,
            option_norm,
            residual_scorer_1,
            residual_scorer_2,
            prior_scale,
            joint_scale,
            residual_gate,
            width,
        })
    }

    pub fn load(safetensors_path: &std::path::Path, hidden_size: usize) -> Result<Self, String> {
        let device = candle_core::Device::Cpu;
        let vb = unsafe {
            VarBuilder::from_mmaped_safetensors(&[safetensors_path], DType::F32, &device)
                .map_err(|err| format!("joint_head.safetensors를 읽을 수 없습니다: {err}"))?
        };
        Self::new(vb, hidden_size, 1024, 2, 4, 16, 4096)
            .map_err(|err| format!("joint_head 레이어 구성에 실패했습니다: {err}"))
    }

    fn char_span_to_token_span(
        token_offsets: &[(usize, usize)],
        start_char: usize,
        end_char: usize,
    ) -> (usize, usize) {
        let start = token_offsets
            .iter()
            .position(|&(s, e)| s <= start_char && start_char < e)
            .unwrap_or(0);
        let end = token_offsets
            .iter()
            .rposition(|&(s, e)| s < end_char && end_char <= e)
            .map(|i| i + 1)
            .unwrap_or(token_offsets.len());
        (start, end.max(start + 1))
    }

    /// `(1, seq_len, hidden)` 텐서에서 토큰 구간 `[start, end)`의 평균을 낸
    /// `(1, hidden)` 텐서. Python의 `_mean_span`(`values[start:end].mean(dim=0)`)과
    /// 동치 — candle 쪽은 배치 차원을 유지한 채로 같은 연산을 한다.
    fn mean_span(hidden_states: &Tensor, start: usize, end: usize) -> CandleResult<Tensor> {
        hidden_states
            .narrow(1, start, end.saturating_sub(start).max(1))?
            .mean(1)
    }

    /// L2 정규화. PyTorch `F.normalize(x, dim=-1)` 기본값(eps=1e-12)과 동일.
    fn normalize_last_dim(x: &Tensor) -> CandleResult<Tensor> {
        let norm = x.sqr()?.sum_keepdim(D::Minus1)?.sqrt()?;
        let norm = norm.clamp(1e-12, f64::INFINITY)?;
        x.broadcast_div(&norm)
    }

    /// 마지막 축 기준 코사인 유사도. PyTorch `F.cosine_similarity(a, b, dim=-1)`
    /// 기본값(eps=1e-8)과 동일: `dot(a,b) / (|a|.clamp(eps) * |b|.clamp(eps))`.
    fn cosine_similarity_last_dim(a: &Tensor, b: &Tensor) -> CandleResult<Tensor> {
        let dot = (a * b)?.sum(D::Minus1)?;
        let norm_a = a.sqr()?.sum(D::Minus1)?.sqrt()?.clamp(1e-8, f64::INFINITY)?;
        let norm_b = b.sqr()?.sum(D::Minus1)?.sqrt()?.clamp(1e-8, f64::INFINITY)?;
        dot / (norm_a * norm_b)?
    }

    /// `question_type`은 Python `QUESTION_TYPES`와 같은 인코딩이다:
    /// noul=0, choice=1, score=2.
    ///
    /// `output_embeddings`는 백본의 출력 임베딩 행렬(`(vocab_size, hidden_size)`,
    /// GGUF의 `output.weight`를 역양자화한 것) — `option_lexical_projection`이
    /// 쓰는, 문맥화되지 않은 토큰 임베딩 평균(`output_embedding_weight[token_ids].mean(dim=0)`,
    /// 실제 소스 그대로)을 계산하는 데 쓴다.
    ///
    /// 반환값은 `(옵션 label, 로짓)` 쌍이다 — `option_spans`가 이미
    /// `tokenizer::option_entries()`가 정한 순서(choice는 알파벳 정렬,
    /// score는 원래 순서, noul은 true/false 고정)로 들어오고, 로짓은 그
    /// 순서 그대로 나온다. 리뷰에서 발견된 버그: 이전에는 `Vec<f32>`만
    /// 돌려줘서, 호출부(`postprocess::to_answer`)가 그 벡터를
    /// `Question::Choice.options`(호출자가 준, 정렬되지 않았을 수 있는
    /// 원래 순서)와 위치로 zip했다 — `option_spans`의 정렬 순서와 다르면
    /// 로짓이 잘못된 옵션 라벨에 붙는다. 이제 label을 로짓과 함께 묶어
    /// 반환해 이 암묵적 "같은 순서" 가정을 코드에서 없앤다.
    pub fn score(
        &self,
        hidden_states: &Tensor,
        input_ids: &Tensor,
        question_type: u32,
        question_span: &QuestionSpan,
        option_spans: &[OptionSpan],
        token_offsets: &[(usize, usize)],
        output_embeddings: &Tensor,
    ) -> Result<Vec<(String, f32)>, String> {
        if option_spans.is_empty() {
            return Err("joint_head 추론에는 옵션이 최소 1개 필요합니다".to_string());
        }
        let run = || -> CandleResult<Vec<f32>> {
            let device = hidden_states.device();
            let num_options = option_spans.len();
            let normed = self.hidden_norm.forward(hidden_states)?;
            let (_, seq_len, _) = normed.dims3()?;

            // memory = memory_projection(normalized_hidden) — 토큰 단위
            // 선형변환이라 배치 차원을 유지한 채 한 번에 계산해도 Python의
            // "unbatch한 뒤 projection, 다시 batch 복원"과 수학적으로 동일하다.
            let memory = self.memory_projection.forward(&normed)?;

            // global_vector = sequence_hidden[-1] (정규화된 hidden의 마지막 토큰).
            let global_vector_hidden = normed.narrow(1, seq_len - 1, 1)?.squeeze(1)?; // (1, hidden)

            let (q_start, q_end) =
                Self::char_span_to_token_span(token_offsets, question_span.start_char, question_span.end_char);
            let question_hidden = Self::mean_span(&normed, q_start, q_end)?; // (1, hidden)
            let base_field = self.question_projection.forward(&question_hidden)?; // (1, width)
            let option_question_term = self.option_question_projection.forward(&question_hidden)?; // (1, width)

            // 옵션별 문맥 벡터(hidden 공간, 평균 pooled)와 어휘 벡터(토큰
            // 임베딩 평균, 문맥화되지 않음 — 실제 소스의 `output_embedding_weight[token_ids].mean(dim=0)`).
            let mut context_rows = Vec::with_capacity(num_options);
            let mut lexical_rows = Vec::with_capacity(num_options);
            for span in option_spans {
                let (o_start, o_end) =
                    Self::char_span_to_token_span(token_offsets, span.start_char, span.end_char);
                context_rows.push(Self::mean_span(&normed, o_start, o_end)?.squeeze(0)?);

                let token_ids = input_ids
                    .narrow(1, o_start, o_end.saturating_sub(o_start).max(1))?
                    .squeeze(0)?
                    .to_dtype(DType::U32)?;
                let lexical = output_embeddings.index_select(&token_ids, 0)?.mean(0)?;
                lexical_rows.push(lexical);
            }
            let context_vectors = Tensor::stack(&context_rows, 0)?; // (num_options, hidden)
            let lexical_hidden = Tensor::stack(&lexical_rows, 0)?; // (num_options, hidden)

            let option_context_term = self.option_context_projection.forward(&context_vectors)?; // (num_options, width)
            let option_lexical_term = self.option_lexical_projection.forward(&lexical_hidden)?; // (num_options, width)

            // option_queries = option_context_projection(context) +
            // option_lexical_projection(lexical) + option_question_projection(question).unsqueeze(0)
            let option_queries =
                option_context_term.broadcast_add(&option_lexical_term)?.broadcast_add(&option_question_term)?;

            // EvidenceRoutingLayer × routing_layers. (batch=1, num_options, width) 쿼리가
            // (batch=1, seq_len, width) 메모리에 attend한다.
            let mut routed = option_queries.unsqueeze(0)?;
            for layer in &self.evidence_layers {
                routed = layer.forward(&routed, &memory)?;
            }
            let routed_options = routed.squeeze(0)?; // (num_options, width)

            // option_summary: field에 대한 옵션들의 attention 가중 평균.
            let field_vec = base_field.squeeze(0)?; // (width,)
            let scores = routed_options.matmul(&field_vec.unsqueeze(1)?)?.squeeze(1)?; // (num_options,)
            let scale = 1.0 / (self.width as f64).sqrt();
            let routing_weights = candle_nn::ops::softmax(&(scores * scale)?, D::Minus1)?; // (num_options,)
            let option_summary = routing_weights
                .unsqueeze(1)?
                .broadcast_mul(&routed_options)?
                .sum(0)?
                .unsqueeze(0)?; // (1, width)
            let option_summary = self.option_summary_norm.forward(&option_summary)?;

            let global_term = self.global_projection.forward(&global_vector_hidden)?; // (1, width)
            let type_ids = Tensor::new(&[question_type], device)?;
            let type_term = self.type_embedding.forward(&type_ids)?; // (1, width)

            let fields = base_field
                .broadcast_add(&option_summary)?
                .broadcast_add(&global_term)?
                .broadcast_add(&type_term)?; // (1, width)

            let mut decoded = fields.unsqueeze(0)?; // (1, 1, width)
            for layer in &self.decoder_layers {
                decoded = layer.forward(&decoded, &memory)?;
            }
            let field_final = self.field_norm.forward(&decoded.squeeze(0)?)?; // (1, width)

            // prior: 문맥화되지 않은 옵션 어휘 벡터와 (질문+전역) 앵커의 코사인 유사도.
            let anchor = Self::normalize_last_dim(&(question_hidden.broadcast_add(&global_vector_hidden))?)?; // (1, hidden)
            let lexical_anchor = Self::normalize_last_dim(&lexical_hidden)?; // (num_options, hidden)
            let prior = lexical_anchor
                .matmul(&anchor.t()?)? // (num_options, 1)
                .squeeze(1)?; // (num_options,)
            let prior = (prior * self.prior_scale as f64)?;

            let options_normed = self.option_norm.forward(&routed_options)?; // (num_options, width)
            let repeated_field = field_final.broadcast_as((num_options, self.width))?;
            let cosine = Self::cosine_similarity_last_dim(&repeated_field, &options_normed)?; // (num_options,)

            let features = Tensor::cat(
                &[
                    &repeated_field,
                    &options_normed,
                    &(&repeated_field * &options_normed)?,
                    &(&repeated_field - &options_normed)?.abs()?,
                ],
                D::Minus1,
            )?; // (num_options, width*4)
            let residual = self
                .residual_scorer_2
                .forward(&self.residual_scorer_1.forward(&features)?.gelu_erf()?)?
                .squeeze(1)?; // (num_options,)

            let joint = ((cosine * self.joint_scale as f64)? + residual)?;
            let logits = (prior + (joint * self.residual_gate as f64)?)?;
            logits.to_vec1::<f32>()
        };
        let logits = run().map_err(|err| format!("joint_head 추론에 실패했습니다: {err}"))?;
        Ok(option_spans
            .iter()
            .map(|span| span.label.clone())
            .zip(logits)
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use candle_core::{DType, Device, Tensor};

    fn random_head(device: &Device) -> JointHead {
        let varmap = candle_nn::VarMap::new();
        let vb = VarBuilder::from_varmap(&varmap, DType::F32, device);
        JointHead::new_for_test(vb, 4096, 1024, 2, 4, 16, 4096).unwrap()
    }

    #[test]
    fn score_returns_one_logit_per_option() {
        let device = Device::Cpu;
        let head = random_head(&device);
        let seq_len = 20;
        let hidden = Tensor::randn(0f32, 1f32, (1, seq_len, 4096), &device).unwrap();
        let input_ids = Tensor::zeros((1, seq_len), DType::U32, &device).unwrap();
        let output_embeddings = Tensor::randn(0f32, 1f32, (32, 4096), &device).unwrap();
        let question_span = crate::local::tokenizer::QuestionSpan {
            label: "q".into(),
            start_char: 0,
            end_char: 5,
        };
        let option_spans = vec![
            crate::local::tokenizer::OptionSpan { label: "a".into(), start_char: 6, end_char: 7 },
            crate::local::tokenizer::OptionSpan { label: "b".into(), start_char: 8, end_char: 9 },
        ];
        let token_offsets: Vec<(usize, usize)> = (0..seq_len).map(|i| (i, i + 1)).collect();
        let logits = head
            .score(&hidden, &input_ids, 1, &question_span, &option_spans, &token_offsets, &output_embeddings)
            .unwrap();
        assert_eq!(logits.len(), 2);
        // 리뷰에서 발견된 버그 재발 방지: score()는 입력 option_spans의
        // 라벨·순서를 그대로 돌려줘야 한다(positional하게만 맞는 게 아니라).
        assert_eq!(logits[0].0, "a");
        assert_eq!(logits[1].0, "b");
    }

    #[test]
    fn noul_question_returns_two_logits_for_true_and_false() {
        // 실제 모델은 noul도 choice/score와 마찬가지로 항상 "true"/"false"
        // 두 옵션을 스키마에 넣는다(Task 2/7에서 tokenizer.rs를 실제
        // encode_record와 맞추면서 확인) — "noul은 옵션이 0개"라는 Task 2의
        // 가정은 실제 소스와 맞지 않았다. 호출부(local::infer)가 이 두
        // 로짓의 log-odds(= logits[0] - logits[1])를 postprocess::to_answer가
        // 기대하는 단일 시그모이드 입력으로 변환한다.
        let device = Device::Cpu;
        let head = random_head(&device);
        let seq_len = 10;
        let hidden = Tensor::randn(0f32, 1f32, (1, seq_len, 4096), &device).unwrap();
        let input_ids = Tensor::zeros((1, seq_len), DType::U32, &device).unwrap();
        let output_embeddings = Tensor::randn(0f32, 1f32, (32, 4096), &device).unwrap();
        let question_span = crate::local::tokenizer::QuestionSpan {
            label: "q".into(),
            start_char: 0,
            end_char: 5,
        };
        let option_spans = vec![
            crate::local::tokenizer::OptionSpan { label: "true".into(), start_char: 6, end_char: 7 },
            crate::local::tokenizer::OptionSpan { label: "false".into(), start_char: 8, end_char: 9 },
        ];
        let token_offsets: Vec<(usize, usize)> = (0..seq_len).map(|i| (i, i + 1)).collect();
        let logits = head
            .score(&hidden, &input_ids, 0, &question_span, &option_spans, &token_offsets, &output_embeddings)
            .unwrap();
        assert_eq!(logits.len(), 2);
        assert_eq!(logits[0].0, "true");
        assert_eq!(logits[1].0, "false");
    }

    #[test]
    fn score_rejects_empty_option_spans() {
        let device = Device::Cpu;
        let head = random_head(&device);
        let seq_len = 10;
        let hidden = Tensor::randn(0f32, 1f32, (1, seq_len, 4096), &device).unwrap();
        let input_ids = Tensor::zeros((1, seq_len), DType::U32, &device).unwrap();
        let output_embeddings = Tensor::randn(0f32, 1f32, (32, 4096), &device).unwrap();
        let question_span = crate::local::tokenizer::QuestionSpan {
            label: "q".into(),
            start_char: 0,
            end_char: 5,
        };
        let token_offsets: Vec<(usize, usize)> = (0..seq_len).map(|i| (i, i + 1)).collect();
        let err = head
            .score(&hidden, &input_ids, 0, &question_span, &[], &token_offsets, &output_embeddings)
            .unwrap_err();
        assert!(err.contains("옵션이 최소 1개"));
    }
}
