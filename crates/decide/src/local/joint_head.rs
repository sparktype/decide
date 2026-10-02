// Clef-flash JointSchemaHead — EvidenceRoutingLayer × routing_layers +
// TransformerDecoderLayer × layers + residual scorer. Task 3의 Backbone이
// 내놓는 hidden_states 위에서 질문/옵션 스팬별 로짓을 뽑아낸다.
//
// candle-nn 0.9.2에는 `MultiheadAttention`도 `transformer::TransformerDecoderLayer`도
// 없다(두 심볼 모두 설치된 crate 소스에 없음을 직접 확인했다) — 그래서 이
// 파일 안에서 Linear Q/K/V/O 투영과 candle_nn::ops::softmax_last_dim으로
// scaled-dot-product attention을 직접 구성한 `MultiHeadAttention`과, 그걸
// 재사용하는 표준 pre-norm 디코더 레이어(self-attn → cross-attn →
// feedforward)를 `TransformerDecoderLayer`로 손으로 작성했다. 입출력 shape
// 계약은 모델 카드 기술과 동일하게 유지한다: `(batch, 1, width)` 쿼리가
// `(batch, seq_len, width)` 메모리에 attend해 `(batch, 1, width)`를 낸다.

use crate::local::tokenizer::{OptionSpan, QuestionSpan};
use candle_core::{DType, Result as CandleResult, Tensor, D};
use candle_nn::{layer_norm, linear_no_bias, Embedding, LayerNorm, Linear, Module, VarBuilder};

struct MultiHeadAttention {
    q_proj: Linear,
    k_proj: Linear,
    v_proj: Linear,
    out_proj: Linear,
    heads: usize,
    head_dim: usize,
}

impl MultiHeadAttention {
    fn new(width: usize, heads: usize, vb: VarBuilder) -> CandleResult<Self> {
        Ok(Self {
            q_proj: linear_no_bias(width, width, vb.pp("q_proj"))?,
            k_proj: linear_no_bias(width, width, vb.pp("k_proj"))?,
            v_proj: linear_no_bias(width, width, vb.pp("v_proj"))?,
            out_proj: linear_no_bias(width, width, vb.pp("out_proj"))?,
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
    attn: MultiHeadAttention,
    feedforward_norm: LayerNorm,
    ff1: Linear,
    ff2: Linear,
}

impl EvidenceRoutingLayer {
    fn new(width: usize, heads: usize, feedforward: usize, vb: VarBuilder) -> CandleResult<Self> {
        Ok(Self {
            query_norm: layer_norm(width, 1e-5, vb.pp("query_norm"))?,
            memory_norm: layer_norm(width, 1e-5, vb.pp("memory_norm"))?,
            attn: MultiHeadAttention::new(width, heads, vb.pp("attn"))?,
            feedforward_norm: layer_norm(width, 1e-5, vb.pp("feedforward_norm"))?,
            ff1: linear_no_bias(width, feedforward, vb.pp("ff1"))?,
            ff2: linear_no_bias(feedforward, width, vb.pp("ff2"))?,
        })
    }

    fn forward(&self, query: &Tensor, memory: &Tensor) -> CandleResult<Tensor> {
        let q = self.query_norm.forward(query)?;
        let m = self.memory_norm.forward(memory)?;
        let attended = self.attn.forward(&q, &m, &m)?;
        let query = (query + attended)?;
        let normed = self.feedforward_norm.forward(&query)?;
        let ff = self.ff2.forward(&self.ff1.forward(&normed)?.gelu()?)?;
        query + ff
    }
}

struct TransformerDecoderLayer {
    self_attn_norm: LayerNorm,
    self_attn: MultiHeadAttention,
    cross_attn_norm: LayerNorm,
    cross_attn: MultiHeadAttention,
    feedforward_norm: LayerNorm,
    ff1: Linear,
    ff2: Linear,
}

impl TransformerDecoderLayer {
    fn new(width: usize, heads: usize, feedforward: usize, vb: VarBuilder) -> CandleResult<Self> {
        Ok(Self {
            self_attn_norm: layer_norm(width, 1e-5, vb.pp("self_attn_norm"))?,
            self_attn: MultiHeadAttention::new(width, heads, vb.pp("self_attn"))?,
            cross_attn_norm: layer_norm(width, 1e-5, vb.pp("cross_attn_norm"))?,
            cross_attn: MultiHeadAttention::new(width, heads, vb.pp("cross_attn"))?,
            feedforward_norm: layer_norm(width, 1e-5, vb.pp("feedforward_norm"))?,
            ff1: linear_no_bias(width, feedforward, vb.pp("ff1"))?,
            ff2: linear_no_bias(feedforward, width, vb.pp("ff2"))?,
        })
    }

    fn forward(&self, query: &Tensor, memory: &Tensor, _mask: Option<&Tensor>) -> CandleResult<Tensor> {
        let normed = self.self_attn_norm.forward(query)?;
        let attended = self.self_attn.forward(&normed, &normed, &normed)?;
        let query = (query + attended)?;

        let normed = self.cross_attn_norm.forward(&query)?;
        let attended = self.cross_attn.forward(&normed, memory, memory)?;
        let query = (query + attended)?;

        let normed = self.feedforward_norm.forward(&query)?;
        let ff = self.ff2.forward(&self.ff1.forward(&normed)?.gelu()?)?;
        query + ff
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
    width: usize,
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
        let residual_scorer_1 = linear_no_bias(width * 4, width, vb.pp("residual_scorer_1"))?;
        let residual_scorer_2 = linear_no_bias(width, 1, vb.pp("residual_scorer_2"))?;

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

    fn span_mean(hidden_states: &Tensor, start: usize, end: usize) -> CandleResult<Tensor> {
        hidden_states
            .narrow(1, start, end.saturating_sub(start).max(1))?
            .mean(1)
    }

    pub fn score(
        &self,
        hidden_states: &Tensor,
        _input_ids: &Tensor,
        question_span: &QuestionSpan,
        option_spans: &[OptionSpan],
        token_offsets: &[(usize, usize)],
    ) -> Result<Vec<f32>, String> {
        let run = || -> CandleResult<Vec<f32>> {
            let normed = self.hidden_norm.forward(hidden_states)?;
            let memory = self.memory_projection.forward(&normed)?;

            let (q_start, q_end) =
                Self::char_span_to_token_span(token_offsets, question_span.start_char, question_span.end_char);
            let question_vec = Self::span_mean(&normed, q_start, q_end)?;
            let question_vec = self.question_projection.forward(&question_vec)?;

            let num_options = option_spans.len().max(1);
            let mut option_vecs = Vec::with_capacity(num_options);
            if option_spans.is_empty() {
                option_vecs.push(question_vec.clone());
            } else {
                for span in option_spans {
                    let (o_start, o_end) =
                        Self::char_span_to_token_span(token_offsets, span.start_char, span.end_char);
                    let option_context = Self::span_mean(&normed, o_start, o_end)?;
                    option_vecs.push(self.option_context_projection.forward(&option_context)?);
                }
            }

            let mut logits = Vec::with_capacity(num_options);
            for option_vec in &option_vecs {
                let query = (&question_vec + option_vec)?.unsqueeze(1)?;
                let mut routed = query;
                for layer in &self.evidence_layers {
                    routed = layer.forward(&routed, &memory)?;
                }
                let summary = self.option_summary_norm.forward(&routed)?;
                let mut decoded = summary;
                for layer in &self.decoder_layers {
                    decoded = layer.forward(&decoded, &memory, None)?;
                }
                let field = self.field_norm.forward(&decoded)?;
                let option_rep = self.option_norm.forward(option_vec)?.unsqueeze(1)?;
                let combined = Tensor::cat(&[&field, &option_rep, &field, &option_rep], D::Minus1)?;
                let combined = combined.reshape((1, self.width * 4))?;
                let hidden = self.residual_scorer_1.forward(&combined)?.gelu()?;
                let score = self.residual_scorer_2.forward(&hidden)?;
                logits.push(score.flatten_all()?.to_vec1::<f32>()?[0]);
            }
            Ok(logits)
        };
        run().map_err(|err| format!("joint_head 추론에 실패했습니다: {err}"))
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
            .score(&hidden, &input_ids, &question_span, &option_spans, &token_offsets)
            .unwrap();
        assert_eq!(logits.len(), 2);
    }

    #[test]
    fn noul_question_returns_single_logit() {
        let device = Device::Cpu;
        let head = random_head(&device);
        let seq_len = 10;
        let hidden = Tensor::randn(0f32, 1f32, (1, seq_len, 4096), &device).unwrap();
        let input_ids = Tensor::zeros((1, seq_len), DType::U32, &device).unwrap();
        let question_span = crate::local::tokenizer::QuestionSpan {
            label: "q".into(),
            start_char: 0,
            end_char: 5,
        };
        let token_offsets: Vec<(usize, usize)> = (0..seq_len).map(|i| (i, i + 1)).collect();
        let logits = head.score(&hidden, &input_ids, &question_span, &[], &token_offsets).unwrap();
        assert_eq!(logits.len(), 1);
    }
}
