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

## 2026-10-04 (실측)
- 사실: 8비트 MLX parity는 골든 5건 모두 오라클과 일치했고 최대 로짓 차이는 0.084~0.095다. Q6_K는 같은 5건 중 11옵션 near-tie 1건이 불일치였는데 이번엔 일치했다.
- 사실: 처음 ops 순차 루프는 짧은 입력 약 1~2초, 907토큰 6.5초였다. mlx-lm의 기본 Metal 커널을 `mlx-sys`(`mlx_fast_metal_kernel_*`)로 직접 호출하도록 바꾸자 각각 약 0.6초, 2.8~3.7초로 줄었다. mlx-rs에는 이 래퍼가 없어서 `mlx-sys`를 직접 의존성으로 추가했다.
- 사실: 구간 계측 결과 백본이 약 90%(토큰당 약 3.6ms, 선형)이고 헤드(candle CPU)는 40~130ms다. 9B×2×토큰 FLOP로 환산하면 약 5 TFLOP/s라 M1 Max 실효 한계에 가깝다. 헤드를 GPU로 옮겨도 이득은 작다.
- 사실: 같은 입력의 candle CPU(Q6_K) 기준선은 짧은 입력 30~48초, 907토큰 125초였다(모델 로드는 첫 호출에 포함). MLX가 약 45~50배 빠르다.
- 참고: 테스트 도중 델타 커널 비교 테스트가 한 번 실패했는데 원인은 합성 입력이 정규화되지 않아 값이 수십만으로 폭주해 절대 오차 기준이 깨진 것이었다(값은 부동소수점 정밀도까지 일치). q/k 스케일을 줄여 고쳤다.
- 남은 일: (1) 릴리스 파이프라인(GitHub Actions)에서 mlx 빌드를 포함할지 결정 — 러너에 Metal Toolchain이 필요하다. (2) 기본 엔진을 mlx로 배포할지. (3) show.rs의 local score legend 형태 문제는 기존 한계 그대로다.

## 2026-10-04 (이전 코드 정리)
- 결정: 사용자가 mlx를 기본 엔진으로 정한 뒤 이전 코드를 모두 걷어내라고 해서, candle GGUF 백본 경로를 삭제했다. `local/backbone.rs`(약 990줄), GGUF 다운로드·해석, `Engine` 분기, `DECIDE_LOCAL_ENGINE`, `mlx`·`default` Cargo 기능이 사라졌다. `mlx-rs`/`mlx-sys`는 일반 의존성이다.
- 결정: candle은 조인트 헤드 때문에 계속 쓴다. `candle-core/nn`은 그대로다.
- 결정: ops 델타 루프는 테스트 모듈로 옮겨 Metal 커널의 참조 구현으로만 남겼다.
- 결정: 가중치 해석은 `resolve_pinned`(CLEF_WEIGHTS, 없으면 하드 에러)와 `download_weights`(HF 캐시)로 나눴다. 옛 가중치 테스트를 `resolve_pinned_*` 테스트 3개와 `pinned_weights_dir` 테스트로 바꿨다.
- 결과: 정리 후에도 parity 5/5, 로짓 최대 차이 0.0948로 같았고 `cargo test`는 82개가 통과했다. 이전 CPU 경로가 필요하면 커밋 d81f1da 이전 이력에서 꺼낸다.
- 결과: 릴리스 워크플로는 Metal Toolchain과 cmake 단계를 항상 거친다.
