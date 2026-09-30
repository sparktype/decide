# 컨텍스트 노트

결정과 그 이유를 시간순으로 덧붙인다. 다음 세션이 다시 유도하지 않도록 남긴다.

## 2026-09-30

- **전환 방향 확인.** 사용자가 "swift → rust"라고 했으나 저장소에 Swift 파일은 없었고
  Python의 착오로 확인됨. 전환은 Python → Rust.
- **Laya 포팅 중단.** 4단계(Laya 시퀀스/후처리 포팅)를 시작하려던 중 사용자가 로컬 모델을
  OpenJev 계열 MLX로 정했다. Laya 설계(ONNX/Candle, 게시 필드 일치)는 대부분 쓸모가 없어져
  새 설계로 대체. Laya 가중치는 HF 캐시에 남아 있음.
- **모델 선정.** 후보를 HF에서 검색해 비교.
  - `mstrasser/Jeff-Qwen3.5-2B`: Apache-2.0, 4.4GB, 자체 패널 82.0%. 영어 전용이라 탈락.
  - `openjev/openjev-MLX`(27GB)/`-4bit`(15GB): 공식, CC BY-NC라 배포 제약.
  - `chaoliangUNSW/Jev-Style-2B-Decision-v3-MLX`: Apache-2.0, 8bit 2.0GB, 25,600토큰,
    옵션 개수 제한 없음, JevBench 73.6%(Jev 86.6%). **선택.**
  - 정확도 수치는 벤치마크가 서로 달라 직접 비교하지 않았다. 처음 "v1이 더 좋다"고
    비교한 것은 틀렸고 사용자가 지적해 정정.
- **한국어.** 2B v3 카드는 한국어를 명시하지 않음(영어 77.7%, 중국어 17.0%, MASSIVE 9개
  언어 미명시). 0.8B v3만 `ko` 명시. 2B 8bit로 한국어 8문항 스모크 실행해 8/8.
  표본이 작아 정확도 수치로 쓰지 않는다.
- **런타임 조사.** `mlx-rs` 0.32.0은 Llama/Qwen3만 지원, Qwen3.5 PR 3개 미머지. 이 모델은
  Gated DeltaNet 18층 + 블록 양방향 어텐션 6층이고 공식 런타임이 mlx-lm 0.31.3 해시 검사와
  수치 보정을 한다. Rust 직접 이식(A)은 작업량이 커서 보류.
- **B 채택.** 로컬 백엔드는 `jev-style serve`(127.0.0.1:8765)를 호출한다. 요청 본문은
  TypeSafe와 같고 응답은 `model`/`answers.q`를 갖춰 `typesafe::map_response`가 그대로 읽는다.
  대가는 로컬 백엔드에 한해 Python이 필요해지는 것.
- **서버 실측값.** 워밍업 후 호출 45~150ms, 첫 호출 1.7초. 서버는 `auth=off`. 오류는
  `{"error":{"code","message"}}` 형태라 `http_error`가 문자열을 못 뽑고 원문 본문을 쓴다.
  (고칠지 구현 때 결정)
- **환경 주의.** `HF_HUB_OFFLINE=1`이 설정돼 있다. 다운로드는 사용자 승인 후 명령 단위로만
  해제했다. 환경 변수 확인 출력에 `HF_TOKEN`이 노출돼 사용자에게 알렸다.
- **scratchpad 잔여물.** `jv/`(venv), `mlx-rs/`(clone), `ko_test.py`. 세션 임시 디렉터리라
  정리 불필요. 2B v3 8bit 가중치(약 2GB)는 HF 캐시에 남아 있다.
- **구현 (feat/local-jev-style-backend).** `execute`/`map_response`/`http_error`가 라벨("TypeSafe"/
  "로컬")을 받고, `LiveTransport`는 `typesafe(key)`/`local(url)` 생성자로 주소와 선택적 키를 갖는다.
  `backend::live_transport(env)`가 프로세스 시작 시 백엔드에 맞는 전송을 고른다. `Env`는 바꾸지
  않고 `DECIDE_LOCAL_URL`은 `local::url()`이 읽는다(`Env` 리터럴 8곳 수정을 피함).
  로컬 연결 실패 안내 문구는 전송 계층(`key.is_none()`)에서 붙인다.
- **실측으로 설계 가정 정정.** `jev-style serve` API는 choice 255개 초과에 422를 돌려준다(모델
  런타임은 무제한). 클라이언트는 로컬 한도를 검사하지 않고 서버 메시지를 그대로 전달한다. 설계
  문서 수정. 서버 오류는 `{"error":{"message"}}` 중첩이라 `http_error`가 원문 JSON을 그대로
  보여 준다(읽을 만해서 그대로 둠).
- **fmt.** baseline이 `cargo fmt --check` 미준수라 `cargo fmt`가 무관한 코드까지 바꿨다.
  `tests/cli.rs`와 `daemon.rs`의 서식만 바뀐 hunk는 되돌렸다. 저장소 전체 fmt 정리는 별도 변경으로.
- **실행 검증 진행.** 데몬 소켓(임시 HOME, `DECIDE_BACKEND=local`, 실서버)에서 choice 답, 동일
  요청 캐시 적중, noul, 검증 오류 뒤 생존을 확인. TypeSafe 실호출은 사용자가 직접 실행해 통과
  (jev-1.13.0, noul 0.84, 528ms). 실행 검증 4개 모두 완료, Python 삭제 게이트 해제.

## 2026-09-30 (하니스 기능 추가)

- **요청.** Obsidian 노트 "Jev 에이전트 하니스 활용 영상 요약"을 바탕으로 decide에 기능 추가.
  사용자가 후보 4개(다중 질문, PreToolUse 위험 게이트, 인젝션 선검사, 리랭킹·캐스케이드)를 모두
  선택하고, 이어서 "스킬을 카테고리로 먼저 질문해 맞는 스킬만 로드"하는 기능의 검토를 추가 요청.
- **분해.** 독립 부분이 섞여 있어 기능별로 설계→계획→구현을 따로 돈다. 순서는 1 다중 질문,
  2 인젝션 선검사, 3 위험 게이트 훅, 4 스킬 캐스케이드, 5 RAG 리랭킹(보류). 사용자 승인.
- **인터페이스.** 다중 질문은 새 도구 `decide_many`(기존 `decide` 불변)로 결정. 기존 호출자와
  Stop 훅 보호가 이유. 전부 성공/전부 실패, 질문 상한은 백엔드에 맡김.
- **스킬 캐스케이드 조사 결과.** (공식 문서 확인) 스킬 이름은 항상 목록에 실리고 설명은 컨텍스트의
  1% 예산과 스킬당 1,536자 안에서만 실린다. `skillOverrides`로 name-only/숨김 가능하고 숨긴 스킬은
  Claude가 Skill 도구로 못 부른다. `UserPromptSubmit` 훅은 `additionalContext`를 주입할 수 있다.
  이 머신은 고유 스킬 48개(설명 약 1.4만 자, 추정 4.6천 토큰)이고 이번 세션은 예산 초과가 아니라
  절약 효과는 작다. 그래서 핵심 가치는 토큰 절약보다 매칭 품질이라 보고, 아무것도 숨기지 않는
  S1 힌트 모드부터 하고 측정 뒤 S2(숨기고 본문 주입)를 판단하기로 했다.
- **기존 자산.** `routing-skills-with-jev` 스킬은 `decide`가 아니라 `mcp__evaluate__evaluate`
  (다른 Jev 판단 서버, `min_confidence`/`__uncertain__` 지원)를 쓴다. 프로세스 스킬 5개만 다룬다.
  캐스케이드 훅은 이를 일반화하는 모양이 될 수 있다. `decide`와 `evaluate`의 중복은 이후 정리 대상.
- **주의.** `docs/jev-agent-harness-video.md`는 로컬 브랜치 `chore/formula-0.0.4-checksum`의
  커밋 `c69f53f`에만 있고 `main`에 없다. 사용자가 만든 것으로 보여 건드리지 않았다.

## 2026-09-30 (decide_many 구현)

- **구현 완료(Native 실행).** 계획의 6개 작업을 TDD로 수행. 단위 테스트 42개와 통합 테스트 전부 통과.
  `decide`의 동작과 기존 테스트는 수정 없이 통과.
- **실측으로 전제 정정.** 영상의 "한 state에 질문 여러 개를 던져도 응답 시간은 거의 늘지 않는다"는 로컬
  Jev-Style 서버에서 성립하지 않았다. 웜업 후 같은 질문 3개를 다중 1회 134ms, 단일 3회 합 139ms로
  질문당 약 45ms씩 선형으로 든다. 첫 호출은 웜업 때문에 363ms였다. 이 기능의 이점은 모델 지연이
  아니라 에이전트의 도구 호출(턴) 횟수 감소다. README와 설계 문서에 반영. TypeSafe는 미측정(키 필요).
- **한글 다중 질문 품질.** 같은 state("배송이 2주째 안 와요. 환불해 주세요. 정말 화가 납니다.")에서
  intent=환불(conf 0.98), urgent noul 0.44, anger score 1.55/2(conf 0.39). intent는 명확하고 나머지는
  애매한 질문이라 확신이 낮다. 정확도 평가가 아니라 한 번의 스모크다.
- **계획 결함 1건.** Task 4 Step 3의 `use` 블록이 이미 있는 `use crate::typesafe::Transport;`를 중복해 컴파일
  오류(E0252). 중복 한 줄 제거. 계획 문서는 고치지 않았다(실행 기록은 ledger에 있다).
- **사고: 공유 체크아웃의 브랜치 전환.** 작업 중 다른 세션이 같은 체크아웃의 브랜치를 `docs/readme-banner`로
  바꿔, Task 2 커밋이 그 브랜치에 들어갔다(9c61c8c). 전용 워크트리 `../decide-many`로 옮기고 내 브랜치에서
  관련 없는 README 배너 커밋을 rebase로 뺐다. `docs/readme-banner`의 9c61c8c는 건드리지 않았다.
