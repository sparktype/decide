# decide: 로컬 판단 모델 기반 에이전트 사용성 도구

## 배경

TypeSafe AI가 2026년 9월 출시한 "Jev"는 텍스트 생성 대신 선택지별 확률(choice) /
순서형 점수(score) / 참거짓(boolean, 원문 표기 "noul")을 단일 순전파로 반환하는
비자기회귀 의사결정 모델이다. API 전용이며 공개 가중치가 없어 로컬 실행이 불가능하다.

`Laya`(Convai Innovations, PyPI `laya`)는 이 방식의 오픈 가중치 대안이다.
공식 Python SDK는 `torch>=2.0`, `transformers>=4.48`, `huggingface_hub`,
`safetensors`, `numpy`에 의존하며, 세 개의 체크포인트(영어 전용 `laya`/ModernBERT-large
421M, 다국어 `laya-multilingual`/mmBERT-base 322M, `laya-typed-decisions`)를
언어에 따라 자동 선택하는 `Router` 클래스를 권장 진입점으로 제공한다.

**중요한 제약**: `Router()`를 기본값(지연 로딩, `max_loaded=1`)으로 쓰면 언어가
바뀔 때마다 체크포인트를 다시 빌드하며 CPU 기준 약 7~10초가 걸린다. 반드시
`Router(preload=True)`로 모든 체크포인트를 상주시켜야 하며, 이 경우 지연시간은
GPU 32.8ms, **CPU 193~464ms** 수준(공식 벤치마크 실측치)이다. 이 프로젝트가 다루는
Mac 환경은 CPU(또는 PyTorch MPS) 추론이므로, "150ms 판단"은 GPU 기준 수치이고 로컬
환경에서는 요청당 수백 ms를 기대해야 한다 — 그래도 LLM 전체 호출보다는 훨씬 빠르다.

이 프로젝트(`decide`)는 이 로컬 판단 모델을 Claude Code 에이전트 워크플로우에
연결해, 에이전트가 개방형 추론 대신 빠르고 보정된(calibrated) 판단을 활용할 수
있게 하는 도구를 만든다.

**이름에 대해**: "Jev"는 TypeSafe AI의 제품명이므로 이 도구에는 사용하지 않는다.
프로젝트 이름은 `decide`다. 폴더는 `decide-mcp`다.

**전송 방식 결정**: 최초 설계는 HTTP 데몬 + CLI였으나, "stdio로 통신하면 어떨지"라는
요청을 검토한 결과 **MCP(Model Context Protocol) 서버로 전환**하기로 했다. 이유는
아래 "검토한 대안과 선택" 참고.

## 목표 / 범위

**1단계(이번 스펙 범위)**
- `decide` MCP 서버: `laya.Router(preload=True)`를 프로세스 시작 시 1회 로드해
  상주시키고, `decide` tool 하나를 stdio로 노출
- 프로젝트 `.mcp.json`에 `decide`를 등록해 Claude Code가 세션당 1번 서버 프로세스를
  스폰하고 세션 내내 stdio 파이프를 유지하도록 함
- `decide` Claude Code Skill: 에이전트가 언제/어떻게 `decide` tool을
  호출해야 하는지, 실패 시 어떻게 폴백해야 하는지 안내

**2단계(별도 브레인스토밍, 이번 스펙 범위 아님)**
- PreToolUse Hook: Bash/Edit 실행 전 위험도를 `decide` tool로 판단해 저위험 자동 승인
- 라우팅/오케스트레이션: 여러 스킬/서브에이전트 중 어디로 보낼지 결정하는 데 활용

성공 기준: Claude Code 세션에서 `decide` tool을 호출해 choice/score/noul 판단을
받고, 그 결과를 근거로 대화를 이어갈 수 있다. 세션 중 첫 호출 이후에는 모델이 이미
상주해 있으므로 매 호출이 LLM 전체 호출보다 체감상 뚜렷이 빠르다.

## 검토한 대안과 선택

| 방식 | 설명 | 채택 |
|---|---|---|
| **A. MCP 서버(stdio) (채택)** | `laya` Router를 감싼 MCP 서버를 Claude Code가 세션당 1회 스폰, stdio로 tool 호출 | 채택 |
| B. HTTP 데몬 + CLI (초기안) | FastAPI 데몬 + `guru` CLI가 HTTP로 통신, CLI가 데몬 자동 스폰 | 기각(대체) — 포트 관리·데몬 자동 스폰·헬스체크·상태 파일을 전부 직접 구현해야 했음. MCP는 이 전체 생명주기 관리를 Claude Code(MCP host)가 대신 해준다 |
| C. 순수 stdio 파이프(비-MCP) | 데몬 프로세스의 stdin/stdout을 CLI가 직접 붙잡는 방식 | 기각 — stdio는 스폰 시점에만 연결되는 1:1 파이프라, 매번 새로 실행되는 독립적인 CLI 프로세스가 이미 떠 있는 데몬의 stdio에 나중에 재연결할 방법이 없음. 데몬 1개를 여러 호출이 공유하려면 소켓(HTTP 또는 유닉스 도메인 소켓)이나 MCP처럼 host가 프로세스를 직접 소유하는 모델이 필요 |
| D. Ollama로 서빙 | 이미 설치된 Ollama(arm64) 재사용 | 기각 — Ollama는 causal-LM 채팅/임베딩용 GGUF 서빙 대상. Laya의 choice/score/noul 판단 헤드 출력은 Ollama API가 노출하지 않음 |

## 아키텍처

```
Claude Code (MCP host)
        │ 세션 시작 시 1회 스폰, stdio 파이프 유지
        ▼
   decide MCP 서버 (상주, laya.Router(preload=True) 로드)
        │ tool: decide(state, type, instructions, options?, criteria?)
        ▼
   결과 반환 (JSON-RPC over stdio, MCP 프로토콜이 처리)
```

데몬 자동 스폰, 포트 선택, 헬스체크, 상태 파일(`daemon.json`) 같은 생명주기
관리는 전부 사라진다 — Claude Code가 `.mcp.json` 설정에 따라 서버 프로세스를
직접 스폰하고 세션이 끝나면 종료시키기 때문이다.

### 컴포넌트

1. **`decide` MCP 서버** (Python, `mcp` SDK — PyPI `mcp` 2.x, `mcp.server.MCPServer`)
   - `decide` tool 하나를 노출:
     - 파라미터: `state: str`, `type: Literal["choice","score","noul"]`,
       `instructions: str`, `options: list[str] | None`(choice용),
       `criteria: list[str] | None`(score용, 등급을 낮은 순서대로)
     - 내부적으로 `router.predict(state, {"q": {"type":..., "instructions":..., "criteria"?:...}})` 호출
       (`criteria`는 choice일 때 `{옵션: 옵션}` dict로, score일 때 순서 리스트 그대로 변환)
     - 반환: `{"answer": <result["answers"]["q"]>, "routing": <result["routing"]>, "latency_ms": float}`
       - `answer`는 타입별로 모양이 다르다: choice → `{"choice": str, "confidence": float, ...}`,
         score → `{"score": float, "confidence": float, ...}`,
         noul → `{"noul": float}` (0.0~1.0 확률 — boolean이 아니다)
     - 입력 검증 실패(옵션/등급 1개 이하, noul에 options/criteria 부여 등)는
       `ToolError`로 반환해 Claude에게 명확한 이유가 대화에 그대로 보이게 한다
   - `laya.Router(preload=True)`는 모듈 전역에 1회만 생성해 재사용(첫 tool 호출 시
     지연 로딩 — 세션 시작과 동시에 무거운 모델을 올리지 않고, 실제 필요할 때 로드)
   - 프로세스 시작/종료는 전적으로 Claude Code(MCP host)가 `.mcp.json` 설정에 따라 관리

2. **`.mcp.json`** (프로젝트 루트)
   - Claude Code가 세션 시작 시 `decide` MCP 서버를 stdio로 스폰하도록 등록
   - `{"mcpServers": {"decide": {"command": "decide-mcp", "args": []}}}` 형태
     (`decide-mcp`는 `pip install -e .`로 설치되는 콘솔 스크립트)

3. **`decide` Skill** (`.claude/skills/decide/SKILL.md`)
   - 언제 쓰는지: 여러 선택지 중 고르기, 참거짓에 가까운 확률 판단, 순서형 점수
     매기기처럼 빠르고 보정된 판단이 필요할 때 전체 추론 대신 `decide` tool 호출
   - 언제 안 쓰는지: 개방형 추론·생성·설명이 필요한 작업에는 사용 안 함
   - tool 호출 예시와 결과(특히 `noul`이 boolean이 아니라 확률이라는 점)를 대화에
     반영하는 방법을 안내
   - tool 호출이 에러를 반환하면(모델 로드 실패, 입력 검증 실패 등) 작업을 막지
     말고 평소처럼 직접 추론해서 계속 진행하도록 안내

## 데이터 흐름

1. Claude Code가 세션 시작 시 `.mcp.json`을 읽고 `decide` MCP 서버를 stdio
   subprocess로 스폰(최초 tool 호출 전까지 모델은 아직 로드되지 않음)
2. Skill 지시에 따라 Claude가 `decide` tool을 호출(JSON-RPC over stdio, MCP
   프로토콜이 직렬화/역직렬화 처리)
3. 서버가 첫 호출이면 `laya.Router(preload=True)`를 로드(수백 ms~수 초, 이후
   호출부터는 재사용), 판단 수행 후 구조화된 결과 반환
4. Claude가 tool 결과(`answer`/`routing`/`latency_ms`)를 대화 맥락에서 바로 사용

가중치 다운로드/캐싱은 `laya` SDK가 자체 처리하므로 별도 로직을 만들지 않는다.

## 에러 처리

- **입력 검증 실패**(choice 옵션 1개 이하, score 등급 1개 이하, noul에 options/criteria
  부여 등) → tool 함수가 `ValueError`를 던지고, MCP 래퍼가 이를 `ToolError`로
  변환해 반환한다. MCP 프로토콜상 tool 에러는 예외로 클라이언트를 죽이지 않고
  `is_error=True`인 정상 결과로 돌아온다 — Claude는 이 결과를 보고 대화를 이어간다
- **모델 로드 실패**(최초 실행 시 네트워크 없어 가중치 다운로드 불가 등) → 예외가
  MCP 에러 결과로 그대로 전달된다. Skill은 이 경우 "판단 불가 → 평소처럼 직접
  추론"으로 폴백하도록 안내(작업을 막지 않음)
- **원칙(2단계 훅 설계에 적용될 제약, 지금은 코드 없음)**: 안전 게이팅에 판단 모델을
  쓸 경우, 실패 시 반드시 "자동 승인"이 아니라 "평소 권한 프롬프트로 폴백"해야
  한다. 판단 실패가 곧 자동 실행으로 이어지면 안 된다.

## 테스트

- **단위 테스트**: `decide` tool의 실제 로직(`_decide_impl`)은 `predict_fn`을
  주입받는 순수 함수로 분리해, 실제 Laya 모델 없이 검증/변환/응답 조립 로직을
  pytest로 검증한다.
- **스모크 테스트**(`test_smoke.py`, pytest 불필요, `python test_smoke.py`로 직접
  실행): `mcp.Client` + `StdioServerParameters`로 서버를 실제 subprocess로 띄우고,
  `decide` tool을 `type="noul", state="2+2=4", instructions="이 문장이 수학적으로
  참인가?"`로 호출해 `result.is_error`가 False이고 `answer["noul"]`이 0.0~1.0
  사이 float이며 0.5보다 큼을 assert로 확인한다.

## 향후 확장 (이번 스펙 범위 아님)

- PreToolUse Hook 기반 위험도 자동 게이팅 — 1단계로 지연시간·정확도가 검증된 뒤
  별도 브레인스토밍/스펙으로 진행
- 여러 스킬/서브에이전트 라우팅 결정에 활용
