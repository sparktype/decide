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
