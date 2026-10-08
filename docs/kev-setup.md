# Kev로 decide 돌리기

Kev는 Qwen3.5 위에 얹은 작은 판단 모델 묶음이다(`jaredpalmer/kev-*`, Apache-2.0). TypeSafe의 System One API(`POST /v1/systemone`)와 같은 형식을 말하는 로컬 서버(`kev.serve`)로 띄운다. `decide` 0.8.0부터 이 서버가 `local` 백엔드가 부르는 대상이다. `decide`는 모델을 서빙하지 않고 이 서버를 HTTP로 부르기만 하며, 서버는 `scripts/serve-local.sh`가 띄운다. 인터넷이 없는 사무실에서 쓰는 것이 주된 용도다.

## 언제 쓰나

같은 입력으로 0.7.0까지 있던 in-process Clef-flash 8비트와 Kev-4B를 나란히 잰 결과다(M1 Max 64GB, 정상 상태, 새 state). Clef-flash 경로는 0.8.0에서 지웠으므로 이 표는 전환의 근거로만 남긴다.

| 입력 | Clef-flash 8비트 | Kev-4B | 배수 |
| --- | --- | --- | --- |
| 긴 state(약 865토큰), 질문 4개 | 3767ms | 약 1040~1140ms | 약 3.5배 |
| 짧은 state, 질문 2개 | 1005ms | 약 180ms | 약 5.5배 |
| 짧은 state, 질문 1개 | 약 500ms | 약 160ms | 약 3배 |
| 같은 state를 다시 보냄 | 해당 없음 | 100~195ms | 서버가 state를 캐시 |

- 응답 속도가 우선이면 Kev-4B가 낫다. 서버 메모리는 약 5.3GB다.
- 정확도는 Kev가 낮을 가능성이 크다. Kev-4B는 Jev보다 낮다. 모델 카드(breadth-v1 test)는 0.690 대 0.757이고, Kev README의 chance-corrected 지수(14개 공개 데이터셋, 학습에 쓰이지 않은 출처)는 38.0 대 54.0이다. 둘은 척도가 달라 서로 비교하지 않고, 모두 Kev 쪽 자체 수치다. 이 저장소의 골든 5케이스에서는 Clef-flash 오라클과 판단이 3건 같았고, 1건은 반대였고, 1건(11개 숫자 옵션)은 확률이 거의 균등해 판단이 서지 않았다. 표본이 작아서 우열의 근거는 아니다. 중요한 판단에 쓰기 전에 실제 질문으로 일치율을 재야 한다.
- 확률이 덜 극단적이다(같은 `noul` 질문에서 Clef-flash 0.957, Kev 0.761). `noul`과 `confidence` 임계값은 Kev 기준으로 다시 정한다. 두 백엔드의 확률은 서로 보정되어 있지 않다.
- 사무실 M2 Pro 32GB는 이 Mac(M1 Max)보다 GPU 연산이 약 0.65배라서 지연이 약 1.5배가 된다고 추정한다(측정하지 않았다). Kev-4B(약 5GB)와 Clef-flash 8비트(약 10.7GB) 모두 32GB에 여유 있게 들어가고, Kev-27B(51GB)는 들어가지 않는다.

측정 기록과 결정은 [`docs/mlx-perf/context-notes.md`](mlx-perf/context-notes.md)에 있다.

## 호환 모델 목록

"검증됨"은 이 저장소에서 `decide`로 직접 돌려 본 것이고, "미검증"은 문서나 모델 카드로만 아는 것이다. 이 표에 없는 모델은 호환 여부를 모른다.

### Kev 서버로 쓰는 모델 (`KEV_MODEL`)

| 허깅페이스 ID | 베이스 | 상태 | 확인한 내용 |
| --- | --- | --- | --- |
| `jaredpalmer/kev-4b` | Qwen3.5-4B-Base | **검증됨** | `kev.serve`(MLX, bf16, `device: mps`)로 띄우고 `decide_many`가 끝까지 동작한다. 지연·판단 일치는 위 "언제 쓰나". 서버 메모리 약 5.3GB. Kev 저장소 커밋 `5e42a7a`, `uv sync --extra serve`. |
| `jaredpalmer/kev-0.8b` | Qwen3.5-0.8B-Base | 미검증 | README는 "모든 Apple Silicon에서 돈다"고 한다. 더 빠르지만 새 출처 정확도가 Kev-4B보다 낮다(README 표 0.648 대 0.817, 개발셋). |
| `jaredpalmer/kev-9b` | Qwen3.5-9B-Base | 미검증 | 32GB Mac 이상에서 돌 것으로 예상하지만 측정되지 않았다고 README가 밝힌다. 약 17GB. |
| `jaredpalmer/kev-27b` | Qwen3.8-27B | 미검증, 사무실(32GB)에는 불가 | 가중치 51GB. README는 96~128GB Mac을 예상한다. |

모두 Apache-2.0이다. `kev.serve --run <ID>`로 지정한다(위 "설정 절차").

서버가 `decide`와 호환되려면 `POST /v1/systemone`이 TypeSafe와 같은 요청·응답 형식을 말해야 한다. `decide`는 `model: "jev-latest"`를 보내고(Kev가 이 별칭을 받는다), 응답의 `answers`를 질문 id로 읽는다. Kev가 아닌 서버는 이 점을 직접 확인해야 하며, 이 저장소에서는 Kev 외의 서버를 시험하지 않았다.

### 0.7.0까지의 in-process 로컬 백엔드 (삭제됨)

`mlx-community/clef-flash-8bit`(기본), `clef-flash-4bit` 등 Clef-flash 구조를 `CLEF_WEIGHTS`/`DECIDE_LOCAL_REPO`/`[local].repo`로 읽던 경로는 0.8.0에서 지웠다. 그 검증 기록(parity 5/5, 웜 지연 150토큰 0.5초·860토큰 2.3초)은 `docs/mlx-backend/`에 남아 있다. Clef-flash가 다시 필요하면 같은 System One 형식의 서버로 따로 띄워 `[local].url`로 가리키면 된다(이 저장소는 그 서버를 제공하지 않는다).

### 시험하지 않은 후보

Von(`wfzyx/von-1.0`), Laya, Nimble, NeoHorse-Jev-4B, `autotrust/JEV-*`는 조사만 했고 `decide`에 붙여 보지 않았다. 이 모델들이 System One 형식의 서버를 제공하는지, 맥(MLX)에서 도는지는 확인하지 못했다(Von은 자체 SDK를 쓴다고 모델 카드에 적혀 있다). 조사 결과와 제외 이유는 [`docs/mlx-perf/context-notes.md`](mlx-perf/context-notes.md)에 있다.

## 설정 절차

### 1. 서버를 띄운다

Apple Silicon Mac에서는 MLX를 자동으로 쓴다. [`uv`](https://docs.astral.sh/uv/)가 필요하다.

```bash
scripts/serve-local.sh
```

스크립트가 하는 일은 다음과 같다.

- kev 저장소를 `~/.cache/decide/kev`에 받고 `KEV_REF`(기본 `5e42a7a`, 위에서 검증한 커밋)로 고정한다.
- `uv sync --extra serve` 뒤 `kev.serve --run $KEV_MODEL --host 127.0.0.1 --port $DECIDE_LOCAL_PORT`를 띄운다.
- `/v1/models`가 답할 때까지 기다린 뒤 noul과 choice 웜업 요청을 한 번씩 보낸다(Metal 커널 컴파일과 첫 호출 비용을 첫 실제 요청이 떠안지 않게).
- 포그라운드로 남고 Ctrl-C로 서버를 끈다. 이미 그 주소에서 서버가 돌고 있으면 아무것도 하지 않고 끝난다.

환경변수는 `KEV_MODEL`(기본 `jaredpalmer/kev-4b`, 다른 크기는 위 표), `DECIDE_LOCAL_PORT`(기본 8009), `KEV_REF`, `KEV_DIR`다.

- 첫 실행은 어댑터와 베이스 모델(Qwen3.5-4B)을 내려받는다. 시간과 디스크가 든다.
- 서버는 `127.0.0.1`에만 바인딩된다. 다른 기기가 붙게 하려면 스크립트를 쓰지 말고 `--host 0.0.0.0`과 `KEV_API_KEY`로 직접 띄운다. `decide`는 인증 헤더를 보내지 않으므로 키를 건 서버는 지금 부를 수 없다.

서버가 떴는지 본다.

```bash
curl -s http://127.0.0.1:8009/v1/models | head -c 400
```

`"backend":"mlx"`, `"device":"mps"`가 보이면 MLX로 돌고 있다.

### 2. decide가 서버를 가리키게 한다

기본 주소가 `http://127.0.0.1:8009/v1/systemone`이므로 포트를 안 바꿨다면 설정할 것이 없다. `TYPESAFE_API_KEY`가 환경에 있으면 백엔드를 안 골랐을 때 TypeSafe가 선택되니 `DECIDE_BACKEND=local`로 고정한다(`~/.claude/settings.json`의 `env` 또는 `config.toml`의 `backend = "local"`).

포트나 호스트를 바꿨으면 `~/.config/decide/config.toml`에 쓴다.

```toml
backend = "local"

[local]
url = "http://127.0.0.1:8010/v1/systemone"
```

같은 값을 환경변수 `DECIDE_LOCAL_URL`로도 줄 수 있다. 환경변수가 있으면 TOML 값은 무시한다. 이미 떠 있는 데몬과 MCP 연결은 옛 설정을 쥐고 있으니, 바꾼 뒤 `pkill -f "decide daemon"`으로 데몬을 한 번 끄고 Claude Code에서 `/mcp`로 `decide`를 다시 연결한다.

### 3. 붙었는지 확인한다

`decide_many`를 한 번 불러 `answers`와 `routing.backend == "local"`이 오는지 본다.

```bash
echo '{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"decide_many","arguments":{"state":"결제 서비스의 응답 지연이 관측되었습니다.","questions":{"urgent":{"type":"noul","instructions":"긴급한가?"}}}}}' \
  | DECIDE_BACKEND=local decide mcp
```

서버가 꺼져 있으면 `로컬 연결에 실패했습니다: ... Connection refused — scripts/serve-local.sh로 로컬 서버를 띄웠는지 확인하세요`로 도구 오류가 난다.

## 되돌리기

`local` 대신 TypeSafe를 쓰려면 `config.toml`의 `backend`를 `typesafe`로 바꾸거나 `DECIDE_BACKEND=typesafe`와 `TYPESAFE_API_KEY`를 준다. 데몬을 한 번 끈다. 서버는 스크립트를 실행한 터미널에서 Ctrl-C로 끈다.

## 운영할 때 알아 둘 점

- **서버 수명은 decide가 관리하지 않는다.** `decide daemon`과 `decide install`은 서버를 띄우지 않는다. 서버가 꺼져 있으면 모든 판단이 연결 실패다. 훅 게이트는 실패를 조용히 통과시키므로(fail-open) 서버가 죽어도 Bash 명령이 막히지 않지만, 판정도 일어나지 않는다. 재부팅 뒤 자동으로 띄우려면 `scripts/install-launchd.sh`로 launchd 사용자 에이전트(`dev.sparktype.kev`)에 등록한다 — `kev.serve`를 직접 실행하도록 plist를 쓰고(`serve-local.sh`는 거치지 않는다. launchd `KeepAlive`가 재시작할 때마다 clone/`uv sync`를 다시 하지 않도록), 로그인 때 `RunAtLoad`로 올라오고 비정상 종료 시 `KeepAlive`로 재시작한다. HMG 사내망처럼 SSL 인터셉트가 있으면 현재 셸의 `SSL_CERT_FILE`/`REQUESTS_CA_BUNDLE`/`CURL_CA_BUNDLE`/`UV_CERT`/`UV_SYSTEM_CERTS`/`NODE_EXTRA_CA_CERTS`를 읽어 plist에 심는다(launchd는 로그인 셸의 환경변수를 물려받지 않는다). 로그는 `~/Library/Logs/dev.sparktype.kev.log`, 상태는 `launchctl print gui/$(id -u)/dev.sparktype.kev`, 제거는 `scripts/install-launchd.sh --uninstall`. `KEV_MODEL`/`DECIDE_LOCAL_PORT`/`KEV_REF`/`KEV_DIR` 환경변수는 `serve-local.sh`와 같다.
- **모델 이름이 `jev-latest`로 나온다.** `routing.backend`는 `local`이지만 `routing.model`은 서버가 돌려준 값이고, Kev는 요청의 모델 별칭(`jev-latest`)을 그대로 돌려준다. 실제로는 로컬 Kev가 답한 것이다.
- **결과는 두 백엔드를 섞어 비교하지 않는다.** 확률 보정이 다르다.
- **같은 기기 안에서 끝난다.** 주소가 `127.0.0.1`이면 게이트가 보내는 명령과 state도 기기 밖으로 나가지 않는다.
- **state 길이.** Kev-0.8B, 4B, 9B는 8,192토큰까지 정확도를 확인했다(서버는 65,536토큰까지 받고 넘으면 422로 거절한다). 긴 문서에서 정확도가 떨어질 수 있다.
- **옵션 한도.** `choice`는 옵션 255개까지다. `decide`의 TypeSafe 백엔드 한도(`score` 10개)가 그대로 적용된다.

## 인터넷 없는 사무실에 옮기기

> 이 절차는 이 저장소에서 검증하지 않았다. 인터넷이 되는 기기에서 시험한 뒤 옮긴다.

필요한 것은 세 가지다.

1. **Kev 저장소와 Python 환경.** 인터넷이 되는 Mac에서 `uv sync --extra serve`까지 마친 `kev` 디렉터리를 같은 CPU 구조(arm64)·같은 macOS 버전대 Mac으로 같은 절대 경로에 복사하는 방식이 가장 단순하다(`uv`의 가상환경은 절대 경로를 품고 있어 경로가 바뀌면 깨질 수 있다).
2. **모델 가중치.** `kev.serve`가 받은 `jaredpalmer/kev-4b`와 베이스 모델 캐시를 `~/.cache/huggingface/hub`에서 옮긴다. 이 Mac에서는 `models--jaredpalmer--kev-4b`(어댑터)를 확인했지만 베이스 모델 가중치가 캐시의 어느 항목인지는 확인하지 못했다. `kev.serve`는 `--run`에 로컬 체크포인트 디렉터리도 받으므로, 어댑터는 그렇게 지정하는 방법도 있다.
3. **오프라인 모드.** 사무실에서는 `HF_HUB_OFFLINE=1`을 설정해 서버가 네트워크로 나가지 않게 한다.

`decide` 쪽에는 HuggingFace를 쓰는 부분이 없다. 오프라인 사무실에서는 서버만 위 세 가지를 갖추면 된다. `scripts/serve-local.sh`는 kev 저장소를 `git clone`하므로, 저장소를 미리 `~/.cache/decide/kev`(`KEV_DIR`)에 옮겨 두고 `KEV_REF`가 그 체크아웃에 있는 커밋이어야 한다(없으면 `git fetch`가 실패한다).
