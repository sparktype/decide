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
- [x] 실제 가중치 parity (5/5 일치, 최대 로짓 차이 0.095)
- [x] 지연 측정 (MLX 0.6~2.8초 대 candle CPU 30~125초)
- [x] 델타 스캔을 Metal 커널로 교체(ops 루프 대비 약 2배)

## 이전 코드 정리 (MLX를 기본이자 유일한 엔진으로)
- [x] `local/backbone.rs`(candle GGUF 백본) 삭제
- [x] `local/mod.rs`에서 엔진 분기, GGUF 가중치 해석·다운로드, `DECIDE_LOCAL_ENGINE`, `#[cfg(feature = "mlx")]` 제거
- [x] Cargo: `mlx-rs`/`mlx-sys`를 일반 의존성으로, `mlx`·`default` 기능 제거(`parity`만 유지)
- [x] ops 델타 루프를 테스트 전용 참조 구현으로 이동
- [x] 옛 가중치 해석 테스트를 MLX 가중치 해석 테스트로 교체
- [x] 문서(CLAUDE.md, README, 워크플로 주석, 테스트 주석) 갱신
- [x] 빌드·전체 테스트·parity 재확인
