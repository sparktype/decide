# MLX 백엔드 컨텍스트 노트

## 2026-10-04
- 결정: 백본만 MLX로 바꾸고 조인트 헤드는 candle CPU에 둔다. 이유는 헤드가 작고 이미 검증됐기 때문이다. 은닉 상태(L×4096)를 GPU→CPU로 한 번 복사하는 비용만 든다.
- 결정: `mlx` Cargo 기능은 기본 꺼짐. 이유는 CI(macos-14)와 기본 빌드가 Metal Toolchain 없이 돌아야 하기 때문이다.
- 사실: `mlx-community/clef-flash-8bit`는 이미 mlx 형식이다(conv1d `[8192,4,1]`, norm 값이 약 1로 +1 반영됨, 양자화 텐서는 U32 weight + BF16 scales/biases). 따로 sanitize할 필요가 없다.
- 사실: 모든 선형이 양자화돼 있다(in_proj_a/b 출력 32도 포함). lm_head와 embed_tokens도 8비트다.
- 사실: 이 환경에는 Metal Toolchain이 없어서 `xcodebuild -downloadComponent MetalToolchain`으로 설치했다(838MB).
- 결정: 깊이별 conv1d는 mlx-rs `conv1d`의 groups 지원이 문서상 불확실해서 커널 4짜리 슬라이스 합으로 직접 구현한다.
- 참고: 포팅 원본은 mlx-lm 0.32.0 `models/qwen3_5.py`, `qwen3_next.py`, `gated_delta.py`(ops 경로)다. 오라클 포트는 HF 저장소의 `clef_mlx.py`.
- 결정: 헤드가 어휘 임베딩을 `input_ids`로 `index_select`하므로, MLX 경로는 옵션 구간 토큰만 dequantize한 압축 테이블과 그 기준으로 다시 매긴 `input_ids`를 넘긴다. 헤드와 그 테스트는 그대로다. 대신 `JointHead::char_span_to_token_span`만 `pub(super)`로 열었다.
- 사실: mlx-rs 0.32 빌드(MLX C++ + Metal 커널)는 처음에 약 3분 걸렸다. 연산 API는 전부 있었다(quantized_matmul, dequantize, fast::rope/rms_norm/sdpa, load_safetensors).
- 사실: 기본 스위트에서 `claude::tests::install_twice_changes_nothing_the_second_time`가 한 번 실패했다. `temp_dir()`이 PID+나노초만 써서 병렬 테스트끼리 같은 디렉터리를 받는 기존 플레이크로 보이고(단독·재실행은 통과), 이번 변경과 무관해 건드리지 않았다.
- 미검증: 실제 가중치 forward. 순서는 (1) `DECIDE_BACKEND=local cargo test --features "mlx parity"` 또는 수동 호출로 오라클 대조, (2) 지연 측정(순차 델타 루프가 병목인지), (3) 필요 시 청크 스캔/Metal 커널.
