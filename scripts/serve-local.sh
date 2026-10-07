#!/usr/bin/env bash
# decide의 local 백엔드가 부르는 Kev 서버를 띄우고 웜업하는 스크립트 (decide는 모델을 서빙하지 않는다)
#
# 사용: scripts/serve-local.sh            # 포그라운드로 서버를 띄운다. Ctrl-C로 끈다
#       KEV_MODEL=jaredpalmer/kev-0.8b scripts/serve-local.sh
#
# 환경변수: KEV_MODEL(기본 jaredpalmer/kev-4b), DECIDE_LOCAL_PORT(기본 8009),
#           KEV_REF(kev 커밋, 기본은 docs/kev-setup.md에서 검증한 것), KEV_DIR(kev 체크아웃 위치)
set -euo pipefail

MODEL=${KEV_MODEL:-jaredpalmer/kev-4b}
PORT=${DECIDE_LOCAL_PORT:-8009}
REF=${KEV_REF:-5e42a7a03f28134853dd3ff77461457e921e5ec1}
DIR=${KEV_DIR:-$HOME/.cache/decide/kev}
URL=http://127.0.0.1:$PORT

command -v uv >/dev/null || { echo "uv가 필요합니다: brew install uv" >&2; exit 1; }

if curl -sf -m 2 "$URL/v1/models" >/dev/null; then
  echo "이미 $URL 에서 서버가 돌고 있습니다." >&2
  exit 0
fi

[ -d "$DIR/.git" ] || git clone -q https://github.com/jaredpalmer/kev.git "$DIR"
git -C "$DIR" cat-file -e "$REF^{commit}" 2>/dev/null || git -C "$DIR" fetch -q origin
git -C "$DIR" checkout -q "$REF"
cd "$DIR"
uv sync -q --extra serve

# 127.0.0.1에만 바인딩한다. 모델은 이 프로세스에 상주하고, 같은 state는 서버가 캐시한다.
uv run --extra serve python -m kev.serve --run "$MODEL" --host 127.0.0.1 --port "$PORT" &
server=$!
trap 'kill "$server" 2>/dev/null || true' EXIT INT TERM

# 첫 실행은 모델을 내려받느라 오래 걸린다.
for _ in $(seq 1 1800); do
  curl -sf -m 2 "$URL/v1/models" >/dev/null && break
  kill -0 "$server" 2>/dev/null || { echo "서버가 시작 중에 끝났습니다." >&2; exit 1; }
  sleep 1
done

# 커널 컴파일과 첫 호출 비용을 첫 실제 요청이 떠안지 않게 noul과 choice를 한 번씩 보낸다.
# 요청 모양은 decide가 보내는 것과 같다(choice의 선택지는 criteria). 웜업이 실패해도 서버는 그대로 둔다.
ask() {
  curl -sf -m 120 "$URL/v1/systemone" -H 'content-type: application/json' -d "$1" >/dev/null ||
    echo "웜업 요청이 실패했습니다(서버는 계속 돕니다): $1" >&2
}
ask '{"model":"jev-latest","state":"warmup","questions":{"q":{"type":"noul","instructions":"참인가?"}}}'
ask '{"model":"jev-latest","state":"warmup","questions":{"q":{"type":"choice","instructions":"어느 쪽인가?","criteria":{"a":"a","b":"b"}}}}'

echo "준비됨: $URL/v1/systemone ($MODEL)  — decide는 기본으로 이 주소를 부른다(DECIDE_BACKEND=local)." >&2
wait "$server"
