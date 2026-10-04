# MLX 로컬 백엔드 계획 (mlx-rs, 8비트)

## 목표
로컬 백엔드의 백본 추론을 candle CPU(GGUF Q6_K)에서 MLX GPU(Metal, 8비트 affine)로 바꾼다.
Rust 단일 바이너리를 유지하므로 `mlx-rs` 0.32를 쓴다. 시험(속도·parity)은 구현이 끝난 뒤에 한다.

## 범위
- 바꾸는 것: 백본 forward 한 곳. `local/mlx_backbone.rs`를 새로 만들고 `local/mod.rs`에서 엔진을 고른다.
- 그대로 두는 것: 토크나이저 조립(`tokenizer.rs`), 조인트 헤드(`joint_head.rs`, candle CPU F32), `postprocess.rs`, 프로토콜, daemon, MCP.
  헤드는 1024폭의 작은 모델이라 CPU에 둬도 부담이 작고, 이미 오라클과 대조된 코드를 건드리지 않는다.
- 엔진 선택: Cargo 기능 `mlx`(기본 꺼짐)로 컴파일 포함 여부를 정한다. 켜면 기본 엔진이 MLX이고
  `DECIDE_LOCAL_ENGINE=candle`로 되돌릴 수 있다. 기본 빌드와 CI는 Metal 툴체인이 필요 없다.

## 가중치
`mlx-community/clef-flash-8bit`에서 `model-0000{1,2}-of-00002.safetensors`, `model.safetensors.index.json`,
`config.json`, `joint_head.safetensors`, `tokenizer.json`을 쓴다. 비전 텐서(`vision_tower.*`)는 읽지 않는다.
체크포인트는 이미 mlx 형식이다(conv1d `[C,4,1]`, RMSNorm +1 반영, 모든 선형층과 임베딩이 8비트 affine, group 64).
`CLEF_WEIGHTS`는 이 파일들이 든 디렉터리를 가리킨다.

## 백본 구조 (Qwen3.5 text, 32층)
- 임베딩(양자화) → 층 반복 → 최종 RMSNorm. 3층 linear_attention 다음 1층 full_attention 패턴.
- full_attention: q_proj는 헤드당 256짜리 query와 gate를 같이 낸다. q/k RMSNorm, 부분 RoPE(64차원, 비전통식, theta 1e7), GQA(16/4), sigmoid 게이트.
- linear_attention(GatedDeltaNet): qkv/z/b/a 투영, 깊이별 causal conv1d(커널 4)+SiLU, q/k L2 정규화, 순차 델타 규칙, 게이트드 RMSNorm.
- MLP: SwiGLU. 모든 선형은 `quantized_matmul(transpose=true)`.
- 어휘 임베딩 행(joint head의 lexical 항)은 lm_head 8비트 행을 필요한 토큰만 dequantize해서 만든다.

## 순서
1. mlx-rs 빌드 확인(Metal Toolchain 설치 포함).
2. `mlx_backbone.rs`: 로더, 연산 조각, 층, forward, 헤드로 넘길 변환.
3. `local/mod.rs`에 엔진 선택과 가중치 해석을 연결.
4. 가중치 없이 도는 단위 테스트(양자화 matmul 대 dequantize, delta 규칙 한 스텝, conv 슬라이스 합, RoPE).
5. 기본 빌드가 안 깨지는지, `--features mlx` 빌드와 테스트가 통과하는지 확인.
6. (구현 뒤, 별도 단계) 실제 가중치로 parity와 지연을 잰다.

## 알려진 위험
- GatedDeltaNet을 순차 연산 루프로 구현하므로 긴 입력에서 그래프가 커진다. 느리면 청크 스캔이나 Metal 커널로 교체한다(측정 후 결정).
- 8비트 MLX 변환본은 Q6_K와 수치가 다르다. parity 합격 기준은 기존 `--features parity`와 오라클 비교다.
- `mlx-rs`는 비전공식 바인딩이다. 빌드에 cmake와 Metal Toolchain이 필요하다.
