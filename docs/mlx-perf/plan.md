# 로컬 MLX 백엔드 응답 성능 개선 계획

## 목표
로컬 백엔드(clef-flash 8bit)의 응답 지연을 줄인다. 먼저 어디서 시간이 드는지 재고, 비용이 작고 확실한 두 가지를 적용한다.

## 범위 (이번 작업)
1. 계측 — `DECIDE_LOCAL_TIMING=1`일 때 추론 한 번의 단계별 시간을 stderr에 한 줄로 찍는다. 기본은 꺼짐이고 출력 형식은 바뀌지 않는다.
2. 데몬 선로딩 — `decide daemon` 시작 직후 백그라운드 스레드로 모델을 올리고 더미 추론을 한 번 돌려, 첫 요청이 로딩·페이지인·커널 컴파일을 떠안지 않게 한다.
3. candle `accelerate` — 조인트 헤드의 CPU GEMM을 Accelerate(AMX)로 돌린다. 계측 전후 비교로 효과가 있을 때만 남긴다.

## 범위 밖 (측정 결과를 보고 다음에 결정)
- `decide_many`의 state 접두 재사용(레이어별 캐시 상태 필요).
- 헤드를 MLX로 이전, 추론마다 `clear_cache` 완화, 4bit 전환, chunkwise delta 커널.

## 성공 기준
- 계측 출력이 단계(토큰화/백본 eval/호스트 복사/헤드 텐서/헤드)별로 나온다.
- 선로딩 후 데몬 첫 요청 지연이 두 번째 요청과 비슷해진다.
- `accelerate` 전후의 헤드 구간 시간을 같은 입력으로 비교해 기록한다.
- 기본 테스트 스위트와 `--features parity`(5/5 일치)가 그대로 통과한다.

## 2차 범위: state 접두 재사용 (2026-10-06)
`decide_many`가 질문마다 `local::infer`를 불러 state를 매번 prefill한다. 백본이 전부 causal이고 `segments()`가 state를 앞, 질문·옵션을 뒤에 두므로, 앞 세 텍스트 세그먼트(시스템 프롬프트, state, `SCHEMA FIELDS`)의 백본 상태를 한 번만 만들고 질문별로는 접미만 이어 계산한다.

### 설계
- 접두 = 첫 세 Text 세그먼트의 토큰. 세그먼트를 따로 토큰화하므로 질문이 달라도 접두 토큰이 같다.
- 레이어별 캐시: 어텐션 레이어는 (K, V) — rope·norm 이후 `[1, kv_heads, P, head_dim]`. gated-delta 레이어는 conv 상태(마지막 K-1개 qkv 입력)와 delta 상태(`[1, Hv, Dv, Dk]` f32).
- 접미 forward: 어텐션은 rope offset=P, K/V를 접두와 이어 붙여 causal sdpa(쿼리 길이 ≠ 키 길이). gated-delta는 conv 앞 패딩을 0 대신 접두 conv 상태로, 커널 `state_in`을 0 대신 접두 delta 상태로 준다.
- 헤드는 전체 시퀀스 은닉 상태를 쓰므로 접두 은닉 상태(host f32)를 보관해 접미 뒤에 붙여 넘긴다. 헤드와 `head_inputs`의 출력 형태는 그대로다.
- 사용처는 `decide_many`에서 질문이 2개 이상일 때만 `local::infer_many`. 단일 질문 `decide`는 그대로.
- 안전장치: 접두 토큰이 캐시와 다르면 접두 없이 전체 계산으로 되돌린다.

### 성공 기준
- 접두 사용 결과가 미사용 결과와 같다(로짓 차이가 양자화 노이즈 수준, 판단 동일).
- 질문 N개일 때 prefill 시간이 대략 1배 + N×접미로 준다.
- 기본 스위트·parity 통과, 단일 질문 경로는 동작·출력 불변.
