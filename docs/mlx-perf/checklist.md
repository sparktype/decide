# 로컬 MLX 성능 개선 체크리스트

- [x] 베이스라인 빌드·기본 테스트 통과 확인
- [x] 계측 모듈(`local/timing.rs`) 추가, 추론 경로에 lap 삽입
- [x] 실제 가중치로 베이스라인 측정(짧은 입력, 긴 입력, 콜드/웜)
- [x] 데몬 선로딩(`local::warmup`, `serve_default`에서 백그라운드 호출)
- [x] 선로딩 전후 첫 요청 지연 측정
- [x] candle `accelerate` 기능 켜기, 빌드 확인
- [x] accelerate 전후 헤드 구간 비교
- [x] 기본 테스트 + parity 재확인
- [x] CLAUDE.md의 로컬 백엔드 설명에 계측 환경변수·선로딩 반영

## 2차: state 접두 재사용
- [x] 접두 세그먼트 수 상수와 `encode_with_prefix`(토큰화), 두 질문이 접두를 공유하는 단위 테스트
- [x] 단위 테스트(가중치 없음): causal sdpa가 쿼리≠키 길이에서 분할 계산과 같은 값, delta 커널 상태 이어받기, conv 상태 이어받기
- [x] 백본: delta 커널이 상태를 입력·출력, conv 상태, 어텐션 KV 캐시, `PrefixState`, `prefill_prefix`, `hidden_states_with`
- [x] 실제 가중치: 전체 forward와 접두+접미 은닉 상태 비교
- [x] `local::infer_many`와 `decide_many` 연결(질문 2개 이상)
- [x] 실제 가중치: `infer_many` 대 `infer` 판단 일치와 지연 비교
- [x] 기본 스위트 + parity, CLAUDE.md 갱신, 커밋
- [x] MLX 스트림 스레드 제한 발견 → 전용 `decide-local` 스레드(`on_worker`)로 모든 로컬 작업을 라우팅, 다중 스레드 회귀 테스트
