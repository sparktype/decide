# Kev로 decide 돌리기

Kev는 Qwen3.5 위에 얹은 작은 판단 모델 묶음이다(`jaredpalmer/kev-*`, Apache-2.0). TypeSafe의 System One API(`POST /v1/systemone`)와 같은 형식을 말하는 로컬 서버(`kev.serve`)로 띄운다. `decide`는 TypeSafe 백엔드의 주소만 이 서버로 바꾸면 되므로 코드를 바꿀 필요가 없다. 인터넷이 없는 사무실에서 쓰는 것이 주된 용도다.

## 언제 쓰나

같은 입력으로 이 저장소의 로컬 백엔드(Clef-flash 8비트)와 Kev-4B를 나란히 잰 결과다(M1 Max 64GB, 정상 상태, 새 state).

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

### Kev 서버로 쓰는 모델 (TypeSafe 백엔드의 `url`)

| 허깅페이스 ID | 베이스 | 상태 | 확인한 내용 |
| --- | --- | --- | --- |
| `jaredpalmer/kev-4b` | Qwen3.5-4B-Base | **검증됨** | `kev.serve`(MLX, bf16, `device: mps`)로 띄우고 `decide_many`가 끝까지 동작한다. 지연·판단 일치는 위 "언제 쓰나". 서버 메모리 약 5.3GB. Kev 저장소 커밋 `5e42a7a`, `uv sync --extra serve`. |
| `jaredpalmer/kev-0.8b` | Qwen3.5-0.8B-Base | 미검증 | README는 "모든 Apple Silicon에서 돈다"고 한다. 더 빠르지만 새 출처 정확도가 Kev-4B보다 낮다(README 표 0.648 대 0.817, 개발셋). |
| `jaredpalmer/kev-9b` | Qwen3.5-9B-Base | 미검증 | 32GB Mac 이상에서 돌 것으로 예상하지만 측정되지 않았다고 README가 밝힌다. 약 17GB. |
| `jaredpalmer/kev-27b` | Qwen3.8-27B | 미검증, 사무실(32GB)에는 불가 | 가중치 51GB. README는 96~128GB Mac을 예상한다. |

모두 Apache-2.0이다. `kev.serve --run <ID>`로 지정한다(위 "설정 절차").

서버가 `decide`와 호환되려면 `POST /v1/systemone`이 TypeSafe와 같은 요청·응답 형식을 말해야 한다. `decide`는 `model: "jev-latest"`를 보내고(Kev가 이 별칭을 받는다), 응답의 `answers`를 질문 id로 읽는다. Kev가 아닌 서버는 이 점을 직접 확인해야 하며, 이 저장소에서는 Kev 외의 서버를 시험하지 않았다.

### 로컬 백엔드로 쓰는 모델 (`DECIDE_LOCAL_REPO` / `[local].repo` / `CLEF_WEIGHTS`)

로컬 백엔드는 Clef-flash 구조(Qwen3.5-9B 하이브리드 백본 + `joint_head.safetensors`)만 읽는다. 저장소(또는 `CLEF_WEIGHTS` 디렉터리)에 `config.json`, `model.safetensors.index.json`, 샤드, `joint_head.safetensors`가 있어야 하고, 양자화는 한 가지 비트 수의 affine(group 64)이어야 한다.

| 허깅페이스 ID | 상태 | 확인한 내용 |
| --- | --- | --- |
| `mlx-community/clef-flash-8bit` | **검증됨(기본)** | parity 5/5 일치, 로짓 최대 차이 0.095. 웜 지연 150토큰 0.5초, 860토큰 2.3초(M1 Max). 약 10.7GB. |
| `mlx-community/clef-flash-4bit` | **검증됨** | 로딩 정상, parity 판단 5/5 일치지만 로짓 최대 차이 1.157로 커진다. 속도는 8비트와 같고 디스크는 약 6GB. `decide_many` 접두 재사용도 동작한다. |
| `mlx-community/clef-4bit`, `clef-8bit` (Clef 27B) | 미검증 | 백본은 설정 파일 기반이라 로딩될 가능성이 있으나 `joint_head` 차원이 다를 수 있다. 4비트 약 15GB, 8비트 약 28GB라 32GB Mac에는 4비트만 맞다. |
| `TrevorJS/clef-flash-mlx-4bit`, `-8bit`, `aufklarer/Clef-flash-9B-MLX-4bit` 등 | 미검증 | 샤드 구성과 `joint_head` 유무를 확인하지 않았다. |

토크나이저는 어느 쪽이든 `Cloudflare/clef-flash`의 `tokenizer.json`을 HF 캐시에서 읽는다.

### 시험하지 않은 후보

Von(`wfzyx/von-1.0`), Laya, Nimble, NeoHorse-Jev-4B, `autotrust/JEV-*`는 조사만 했고 `decide`에 붙여 보지 않았다. 이 모델들이 System One 형식의 서버를 제공하는지, 맥(MLX)에서 도는지는 확인하지 못했다(Von은 자체 SDK를 쓴다고 모델 카드에 적혀 있다). 조사 결과와 제외 이유는 [`docs/mlx-perf/context-notes.md`](mlx-perf/context-notes.md)에 있다.

## 설정 절차

### 1. Kev 서버를 띄운다

Apple Silicon Mac에서는 MLX를 자동으로 쓴다. [`uv`](https://docs.astral.sh/uv/)가 필요하다.

```bash
git clone https://github.com/jaredpalmer/kev.git
cd kev
uv sync --extra serve
uv run --extra serve python -m kev.serve --run jaredpalmer/kev-4b --port 8009
```

- 첫 실행은 어댑터와 베이스 모델(Qwen3.5-4B)을 내려받는다. 시간과 디스크가 든다.
- 서버는 `127.0.0.1`에만 바인딩된다. 다른 기기가 붙게 하려면 `--host 0.0.0.0`을 쓰되 `KEV_API_KEY`로 인증을 건다.
- 포트 8009는 `decide` 데몬의 48080과 겹치지 않는다.
- 다른 크기는 `--run jaredpalmer/kev-0.8b`(더 빠름, 정확도 낮음), `kev-9b`(느림, 약 17GB)처럼 지정한다. Kev-27B는 51GB라 32GB Mac에는 맞지 않는다.

서버가 떴는지 본다.

```bash
curl -s http://127.0.0.1:8009/v1/models | head -c 400
```

`"backend":"mlx"`, `"device":"mps"`가 보이면 MLX로 돌고 있다.

### 2. decide가 Kev를 가리키게 한다

`decide` 0.7.0 이상이 필요하다. 이전 버전은 `url`을 모르고 무시하므로, `api_key`만 보고 진짜 TypeSafe 서버에 접속해 실패한다(훅 게이트는 조용히 통과하고 MCP 도구는 오류가 난다). 먼저 `decide --version`으로 확인한다.

`~/.config/decide/config.toml`에 쓴다.

```toml
backend = "typesafe"

[typesafe]
api_key = "local"
url = "http://127.0.0.1:8009/v1/systemone"
```

- `api_key`는 비어 있지 않은 아무 값이면 된다. Kev는 기본으로 인증을 요구하지 않는다(`KEV_API_KEY`를 걸었다면 그 값을 쓴다).
- 같은 값을 환경변수로도 줄 수 있다. `DECIDE_BACKEND=typesafe`, `TYPESAFE_API_KEY=local`, `DECIDE_TYPESAFE_URL=http://127.0.0.1:8009/v1/systemone`. 환경변수가 있으면 같은 키의 TOML 값은 무시한다.
- 이미 떠 있는 데몬과 MCP 연결은 옛 설정을 쥐고 있다. 바꾼 뒤 `pkill -f "decide daemon"`으로 데몬을 한 번 끄고, Claude Code에서 `/mcp`로 `decide`를 다시 연결한다.

### 3. 붙었는지 확인한다

`decide_many`를 한 번 불러 `answers`가 오는지 본다. 설정 파일 없이 환경변수만으로 시험하려면 다음처럼 한다.

```bash
echo '{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"decide_many","arguments":{"state":"결제 서비스의 응답 지연이 관측되었습니다.","questions":{"urgent":{"type":"noul","instructions":"긴급한가?"}}}}}' \
  | DECIDE_BACKEND=typesafe TYPESAFE_API_KEY=local DECIDE_TYPESAFE_URL=http://127.0.0.1:8009/v1/systemone decide mcp
```

서버 주소가 틀리거나 서버가 꺼져 있으면 `TypeSafe 연결에 실패했습니다: ... Connection refused`로 도구 오류가 난다.

## 되돌리기

`config.toml`에서 `backend`, `[typesafe]` 블록을 지우거나 `backend = "local"`로 바꾼다. 환경변수로 줬다면 해당 변수를 지운다. 데몬을 한 번 끈다.

## 운영할 때 알아 둘 점

- **서버 수명은 decide가 관리하지 않는다.** `decide daemon`과 `decide install`은 Kev 서버를 띄우지 않는다. 서버가 꺼져 있으면 모든 판단이 연결 실패다. 훅 게이트는 실패를 조용히 통과시키므로(fail-open) 서버가 죽어도 Bash 명령이 막히지 않지만, 판정도 일어나지 않는다. 재부팅 뒤 자동 실행은 launchd 등으로 직접 구성해야 하고, 이 저장소는 그 설정을 제공하지 않는다.
- **표시가 TypeSafe로 나온다.** 응답의 `routing.backend`는 `typesafe`, `routing.model`은 `jev-latest`다. Kev가 모델 별칭을 그대로 돌려주기 때문이고, 실제로는 로컬 Kev가 답한 것이다.
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

`decide` 쪽에도 같은 점이 있다. 로컬 백엔드(Clef-flash)를 쓰는 경우 `CLEF_WEIGHTS`로 가중치를 고정해도 토크나이저(`Cloudflare/clef-flash`)는 HF 캐시를 거친다. 캐시가 미리 채워져 있으면 네트워크 없이 동작하고, 비어 있으면 실패한다. Kev만 쓰면 `decide`는 HF를 쓰지 않는다.
