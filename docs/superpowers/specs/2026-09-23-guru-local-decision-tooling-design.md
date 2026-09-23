# guru: 로컬 판단 모델 기반 에이전트 사용성 도구

## 배경

TypeSafe AI가 2026년 9월 출시한 "Jev"는 텍스트 생성 대신 선택지별 확률(choice) /
순서형 점수(score) / 참거짓(boolean, 원문 표기 "noul")을 단일 순전파로 반환하는
비자기회귀 의사결정 모델이다. API 전용이며 공개 가중치가 없어 로컬 실행이 불가능하다.

`Laya`(Convai Innovations, PyPI `laya`)는 이 방식의 오픈 가중치 대안이다.
ModernBERT-large(421M, context 512) 기반, ONNX Runtime으로 구동되며 PyTorch 없이도
동작한다. 가중치(~1.7GB, fp32 ONNX)는 최초 실행 시 Hugging Face에서 내려받아
`~/.cache`에 캐싱된다. 응답 지연은 수백 ms 수준으로, 매 호출 LLM 추론보다 훨씬 싸고
빠르게 "구조화된 판단"을 낼 수 있다.

이 프로젝트(`guru`)는 이 로컬 판단 모델을 Claude Code 에이전트 워크플로우에
Skill/Hook 형태로 연결해, 에이전트가 개방형 추론 대신 빠르고 보정된(calibrated)
판단을 활용할 수 있게 하는 도구를 만든다.

**이름에 대해**: "Jev"는 TypeSafe AI의 제품명이므로 이 도구에는 사용하지 않는다.
CLI/데몬/스킬 이름은 모두 `guru`로 통일한다.

## 목표 / 범위

**1단계(이번 스펙 범위)**
- `guru` 로컬 데몬: Laya 모델을 상주시켜 HTTP로 판단 요청을 처리
- `guru` CLI: 데몬을 자동 기동하고 판단 결과를 JSON으로 반환하는 얇은 클라이언트
- `guru:decide` Claude Code Skill: 에이전트가 판단이 필요할 때 CLI를 호출하도록 안내

**2단계(별도 브레인스토밍, 이번 스펙 범위 아님)**
- PreToolUse Hook: Bash/Edit 실행 전 위험도를 `guru decide`로 판단해 저위험 자동 승인
- 라우팅/오케스트레이션: 여러 스킬/서브에이전트 중 어디로 보낼지 결정하는 데 활용

성공 기준: Claude Code 세션에서 `guru decide`를 호출해 choice/score/boolean 판단을
받고, 그 결과를 근거로 대화를 이어갈 수 있다. 데몬이 뜬 상태에서 판단 1회 왕복이
LLM 전체 호출보다 체감상 뚜렷이 빠르다.

## 검토한 대안과 선택

| 방식 | 설명 | 채택 |
|---|---|---|
| **A. 로컬 데몬 + 얇은 CLI (채택)** | `laya` SDK를 감싼 상주 프로세스가 모델을 1회 로드, HTTP로 판단 처리 | 채택 |
| B. 데몬 없이 매 호출 subprocess | 호출마다 Python 스크립트로 모델 로드 | 기각 — 421M 모델 로드 비용이 매번 발생, "빠른 판단"이라는 핵심 가치 상실. 특히 2단계 훅은 도구 호출마다 실행되므로 치명적 |
| C. Ollama로 서빙 | 이미 설치된 Ollama(arm64) 재사용 | 기각 — Ollama는 causal-LM 채팅/임베딩용 GGUF 서빙 대상. Laya의 choice/score/boolean 판단 헤드 출력은 Ollama API가 노출하지 않음 |

## 아키텍처

```
Claude (guru:decide Skill)
        │ subprocess
        ▼
   guru CLI  ──HTTP(127.0.0.1)──▶  guru daemon (상주, laya 모델 로드)
        ▲                                │
        └── 데몬 미기동 시 자동 스폰 ──────┘
```

### 컴포넌트

1. **`guru serve`** (Python, FastAPI + uvicorn)
   - `127.0.0.1`에만 바인딩 (외부 노출 없음)
   - 시작 시 `laya` SDK로 모델 1회 로드 후 프로세스 상주
   - `POST /decide`
     - 요청: `{"type": "choice"|"score"|"noul", "question": str, "options"?: [str], "scale"?: [int, int]}`
     - 응답: `{"answer": ..., "probs": [...], "confidence": float, "latency_ms": float}`
   - `GET /health` — 상태 확인용
   - 기동 시 실제 바인딩 포트와 PID를 `~/.cache/guru/daemon.json`에 기록 (기본 포트 사용 불가 시 빈 포트로 폴백)

2. **`guru` CLI** (Python entry point, HTTP 클라이언트)
   - `guru decide --type boolean --question "..."`
   - `guru decide --type choice --question "..." --options "a,b,c"`
   - `guru decide --type score --question "..." --scale 1,5`
   - `guru serve` / `guru stop` / `guru status` (수동 제어용)
   - 기본 출력: JSON(기계 소비용). `--pretty` 옵션으로 사람이 읽기 좋은 포맷
   - `decide` 호출 시 데몬 미기동이면 자동으로 `guru serve`를 백그라운드 스폰하고
     `/health`가 응답할 때까지 폴링(최대 ~5초) 후 원 요청 재시도

3. **`guru:decide` Skill** (SKILL.md)
   - 언제 쓰는지: 여러 선택지 중 고르기, 참거짓 판단, 순서형 점수 매기기처럼
     빠르고 보정된 판단이 필요할 때 전체 추론 대신 CLI 호출
   - 언제 안 쓰는지: 개방형 추론·생성·설명이 필요한 작업에는 사용 안 함
   - CLI 호출 예시와 JSON 결과를 대화에 반영하는 방법을 안내

## 데이터 흐름

1. Skill 지시에 따라 Claude가 `guru decide ...`를 subprocess로 실행
2. CLI가 `~/.cache/guru/daemon.json`을 읽어 데몬 주소 확인. 없으면 `guru serve` 스폰 후 대기
3. CLI → 데몬 `/decide` HTTP 요청
4. 데몬이 로드된 Laya 모델로 단일 순전파 수행, 결과 반환
5. CLI가 JSON을 stdout에 출력, Claude가 결과를 파싱해 이어서 사용

가중치 다운로드/캐싱은 `laya` SDK가 자체 처리하므로 별도 로직을 만들지 않는다.

## 에러 처리

- **데몬 기동 실패** (예: 최초 실행 시 네트워크 없어 가중치 다운로드 불가) → CLI는
  stderr에 원인을 명시하고 non-zero exit. Skill은 이 경우 "판단 불가 → 평소처럼
  직접 추론"으로 폴백하도록 안내(작업을 막지 않음)
- **포트 충돌** → `guru serve`가 기본 포트를 못 열면 빈 포트로 폴백하고 실제 포트를
  `daemon.json`에 기록. CLI는 항상 이 파일을 읽어 접속하므로 영향 없음
- **타임아웃** → CLI는 고정 타임아웃(10초) 후 에러 반환
- **원칙(2단계 훅 설계에 적용될 제약, 지금은 코드 없음)**: 안전 게이팅에 판단 모델을
  쓸 경우, 실패/타임아웃 시 반드시 "자동 승인"이 아니라 "평소 권한 프롬프트로 폴백"
  해야 한다. 판단 실패가 곧 자동 실행으로 이어지면 안 된다.

## 테스트

프레임워크 없이 최소 1개의 실행 가능한 스모크 테스트:

- `test_smoke.py` — `guru serve`를 띄운 뒤 `guru decide --type boolean --question
  "2+2=4?"`를 호출해 `answer == True`이고 `probs` 합이 1에 근접함을 assert로 확인.
  `python test_smoke.py`로 직접 실행 가능(pytest 불필요).

## 향후 확장 (이번 스펙 범위 아님)

- PreToolUse Hook 기반 위험도 자동 게이팅 — 1단계로 지연시간·정확도가 검증된 뒤
  별도 브레인스토밍/스펙으로 진행
- 여러 스킬/서브에이전트 라우팅 결정에 활용
