# decide 로컬 백엔드: Jev-Style v3 서버 호출

2026-09-29 설계(`2026-09-29-decide-rust-runtime-design.md`)의 로컬 런타임 절(ONNX/Candle,
Laya 게시 필드 일치, `--features parity`)을 이 문서가 대체한다. 그 문서의 나머지(TypeSafe
백엔드, MCP, 데몬, Homebrew)는 그대로다.

## 결정

로컬 모델은 Laya가 아니라 `chaoliangUNSW/Jev-Style-2B-Decision-v3-MLX`(8bit, 2.0GB,
Apache-2.0)다. 로컬 백엔드는 모델을 Rust에서 돌리지 않고 `jev-style serve`가 여는
`POST http://127.0.0.1:8765/v1/systemone`을 호출한다.

## 왜 이 방식인가

- 이 모델은 Gated DeltaNet 18층과 블록 단위 양방향 어텐션 6층이다. stock mlx-lm이나
  llama.cpp는 결정 점수를 내지 못하고, 공식 런타임이 mlx-lm 0.31.3 소스 해시를 검사하며
  q/k l2norm eps와 RMSNorm `(1+w)` fp32 사이드카를 보정한다.
- `mlx-rs` 0.32.0의 `mlx-lm`은 Llama와 Qwen3만 지원한다. Qwen3.5 PR(#335, #351, #356)은
  모두 머지되지 않았다. `mlx-c`에는 `mlx_fast_metal_kernel`이 있지만 `mlx-rs`에 안전한
  래퍼가 없다. 직접 이식하려면 GDN 레이어, 블록 어텐션, 보정, 프롬프트 렌더링(약 1000줄),
  토크나이저를 옮겨야 한다.
- 2026-09-30 실측에서 서버는 TypeSafe와 같은 요청 본문을 받았고 응답이 `model`과
  `answers.q`를 갖춰 `typesafe::map_response`가 그대로 읽는다. 새 모델 코드가 필요 없다.

## 대가

로컬 백엔드를 쓰려면 Python 서버가 필요하다. `pip install "jev-style[mlx]"`(mlx-lm 0.31.3
고정)와 `jev-style serve --release 2b --precision 8bit`다. 2026-09-29 설계의 "설치에
Python 없음"은 로컬 백엔드에 한해 포기한다. TypeSafe 백엔드는 지금처럼 Python이 없다.

## 동작

| 항목 | 내용 |
| --- | --- |
| 선택 | 변경 없음. `DECIDE_BACKEND`, 없으면 `TYPESAFE_API_KEY` 유무로 고른다. |
| 주소 | `DECIDE_LOCAL_URL`, 기본 `http://127.0.0.1:8765/v1/systemone`. |
| 인증 | 보내지 않는다. 서버는 `auth=off`이고 127.0.0.1에만 바인딩한다. |
| 요청 본문 | `typesafe::request_body`를 그대로 쓴다. |
| 한도 | 클라이언트는 로컬에서 옵션·등급 개수를 검사하지 않는다. 모델 런타임은 25,600토큰 안에서 개수 제한이 없지만, `jev-style serve` API는 choice가 255개를 넘으면 422(`choice needs 1..255 options`)로 거절한다(2026-09-30 실측). 그 메시지가 그대로 전달된다. score 등급 상한은 확인하지 못했다. |
| 재시도 | `typesafe::execute`를 공유한다. 429와 529 한 번 재시도는 로컬에서도 그대로 돈다. |
| 응답 | `answer`는 서버의 `answers.q`다. `routing`은 `{"backend": "local", "model": <서버 model>}`이다. |
| 오류 | 연결 실패는 "로컬 연결에 실패했습니다: …. jev-style serve가 실행 중인지 확인하세요"다. 다른 백엔드로 넘어가지 않는다. |
| 캐시 | 데몬 LRU는 요청과 해석된 백엔드가 키라 그대로 동작한다. |

Laya `system_one`의 `action` 키는 더 이상 나오지 않는다. 로컬 답은 TypeSafe 답과 같은
모양이다(`choice`/`score`/`noul`, `probabilities`, `confidence`, score의 `legend`).
두 백엔드의 확률은 여전히 서로 맞추지 않는다.

## 구성 변경

- `local.rs`는 NOT_READY 상수 대신 기본 주소와 오류 문구를 둔다.
- `typesafe.rs`의 `LiveTransport`는 주소와 선택적 키를 받는다. 오류 문구의 "TypeSafe"는
  백엔드 이름을 받아 채운다.
- `backend.rs`는 `Backend::Local`에서 NOT_READY 대신 같은 경로를 타고 주소와 라벨만 바꾼다.
- 삭제 없음. Python 패키지는 아래 검증 뒤에 지운다.

## 테스트

기본 `cargo test`는 가중치, 네트워크, API 키 없이 돈다.

- 로컬 선택 시 요청이 로컬 주소로 가고 `Authorization`이 없다(스크립트 transport로 검사).
- 로컬 응답이 `routing.backend == "local"`로 매핑된다. 기록한 응답 JSON을 쓴다.
- 로컬에서는 클라이언트가 한도를 검사하지 않아 256개 choice가 호출까지 간다. TypeSafe는 호출 전에 막는다.
- 로컬 연결 실패가 TypeSafe로 넘어가지 않고 안내 문구를 낸다.
- 기존 `typesafe_error_does_not_call_local` 계열은 유지한다.

## 실행 검증 뒤 정리

다음이 모두 통과하면 같은 변경에서 이전 Python 구현을 지운다.

1. `cargo test`가 통과한다.
2. 키가 있는 환경에서 `decide` 도구 호출이 `routing.backend == "typesafe"`로 온다.
3. `jev-style serve`가 떠 있을 때 `DECIDE_BACKEND=local` 호출이 `routing.backend == "local"`로
   오고 한글 state가 맞게 판단된다.
4. 데몬 소켓에 JSON 한 줄을 보내면 같은 껍데기의 답이 온다.

지우는 것과 남기는 것은 2026-09-29 문서의 같은 절을 따른다. 다만 Laya 픽스처와 ONNX
스크립트는 만들지 않았으므로 지울 것이 없고, "로컬 회귀 JSON 픽스처"도 없다.
README와 `CLAUDE.md`의 로컬 백엔드 절은 이 문서 기준으로 고친다.

## 범위 밖

- Rust 안에서 MLX로 모델 돌리기. `mlx-rs`에 Qwen3.5가 머지되면 다시 본다.
- `decide`가 `jev-style serve`를 대신 띄우거나 종료하기.
- Laya, OpenJev 27GB 모델, Jeff 계열 모델. 한국어 근거가 없거나 라이선스가 맞지 않는다.
- 한국어 정확도 벤치마크. 지금 근거는 8문항 스모크뿐이다.

## 알려진 위험

- 한국어는 스모크 8문항(8/8)만 확인했다. 2B v3 카드는 한국어를 명시하지 않는다.
- 서버 기동 후 모델 로딩에 1~8초, 첫 호출에 1.7초가 걸렸다. 호출 타임아웃은 30초다.
- 첫 실행은 가중치를 받는다. `HF_HUB_OFFLINE=1`이면 미리 받아 둬야 한다.
