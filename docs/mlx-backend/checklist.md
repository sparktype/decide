# MLX 백엔드 체크리스트

- [x] Metal Toolchain 설치 (`xcodebuild -downloadComponent MetalToolchain`)
- [x] Cargo 기능 `mlx` 추가
- [x] mlx-rs 컴파일 확인
- [x] `local/mlx_backbone.rs` 로더(safetensors, 양자화 텐서 묶음)
- [x] 연산: 양자화 선형, RMSNorm, 부분 RoPE, GQA 어텐션(게이트), GatedDeltaNet, SwiGLU
- [x] forward → 헤드용 candle Tensor 변환, lexical 행 dequantize
- [x] `local/mod.rs` 엔진 선택(`DECIDE_LOCAL_ENGINE`), MLX 가중치 해석
- [x] 가중치 없는 단위 테스트
- [x] 기본 빌드/테스트 통과
- [x] `--features mlx` 빌드/테스트 통과
- [x] 문서(CLAUDE.md의 로컬 백엔드 설명) 갱신
- [ ] (구현 뒤) 실제 가중치 parity, 지연 측정
