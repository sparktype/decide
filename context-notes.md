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
