#!/usr/bin/env bash
# Kev MLX 서버를 launchd 사용자 에이전트로 등록해 로그인 시 자동으로 띄운다.
# plist는 scripts/serve-local.sh를 거치지 않고 kev.serve를 직접 실행한다(KeepAlive가
# 매번 처음부터 git clone/uv sync를 다시 하지 않도록). 이 스크립트가 그 준비(클론,
# KEV_REF 체크아웃, uv sync)를 먼저 한 번 해 둔다.
#
# 사용: scripts/install-launchd.sh            # 준비하고 설치하고 지금 바로 띄운다
#       scripts/install-launchd.sh --uninstall # 내리고 plist를 지운다
#
# 환경변수는 serve-local.sh와 같다(KEV_MODEL, DECIDE_LOCAL_PORT, KEV_REF, KEV_DIR).
# 이 스크립트를 실행할 때 설정한 값이 plist에 그대로 적힌다.
# HMG 사내망 SSL 인터셉트가 있는 환경이면 현재 셸의 SSL_CERT_FILE/REQUESTS_CA_BUNDLE/
# CURL_CA_BUNDLE/UV_CERT/UV_SYSTEM_CERTS/NODE_EXTRA_CA_CERTS를 함께 plist에 심는다
# (launchd는 로그인 셸의 환경변수를 물려받지 않는다).
set -euo pipefail

LABEL=dev.sparktype.kev
PLIST="$HOME/Library/LaunchAgents/$LABEL.plist"
UID_GUI="gui/$(id -u)"

MODEL=${KEV_MODEL:-jaredpalmer/kev-4b}
PORT=${DECIDE_LOCAL_PORT:-8009}
REF=${KEV_REF:-5e42a7a03f28134853dd3ff77461457e921e5ec1}
DIR=${KEV_DIR:-$HOME/.cache/decide/kev}
URL=http://127.0.0.1:$PORT

if [ "${1:-}" = "--uninstall" ]; then
  launchctl bootout "$UID_GUI/$LABEL" 2>/dev/null || true
  rm -f "$PLIST"
  echo "제거했습니다: $PLIST" >&2
  exit 0
fi

UV=$(command -v uv) || { echo "uv가 필요합니다: brew install uv" >&2; exit 1; }

[ -d "$DIR/.git" ] || git clone -q https://github.com/jaredpalmer/kev.git "$DIR"
git -C "$DIR" cat-file -e "$REF^{commit}" 2>/dev/null || git -C "$DIR" fetch -q origin
git -C "$DIR" checkout -q "$REF"
( cd "$DIR" && "$UV" sync -q --extra serve )

cert_env_xml() {
  for name in SSL_CERT_FILE REQUESTS_CA_BUNDLE CURL_CA_BUNDLE UV_CERT UV_SYSTEM_CERTS NODE_EXTRA_CA_CERTS; do
    value="${!name:-}"
    [ -n "$value" ] && printf '\t\t\t<key>%s</key>\n\t\t\t<string>%s</string>\n' "$name" "$value"
  done
}

cat > "$PLIST" <<PLIST_EOF
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
	<key>Label</key>
	<string>$LABEL</string>
	<key>ProgramArguments</key>
	<array>
		<string>$UV</string>
		<string>run</string>
		<string>--extra</string>
		<string>serve</string>
		<string>python</string>
		<string>-m</string>
		<string>kev.serve</string>
		<string>--run</string>
		<string>$MODEL</string>
		<string>--host</string>
		<string>127.0.0.1</string>
		<string>--port</string>
		<string>$PORT</string>
	</array>
	<key>EnvironmentVariables</key>
	<dict>
		<key>PATH</key>
		<string>/opt/homebrew/bin:/usr/bin:/bin:/usr/sbin:/sbin</string>
$(cert_env_xml)	</dict>
	<key>WorkingDirectory</key>
	<string>$DIR</string>
	<key>RunAtLoad</key>
	<true/>
	<key>KeepAlive</key>
	<dict>
		<key>SuccessfulExit</key>
		<false/>
	</dict>
	<key>StandardOutPath</key>
	<string>$HOME/Library/Logs/$LABEL.log</string>
	<key>StandardErrorPath</key>
	<string>$HOME/Library/Logs/$LABEL.log</string>
</dict>
</plist>
PLIST_EOF

launchctl bootout "$UID_GUI/$LABEL" 2>/dev/null || true
launchctl bootstrap "$UID_GUI" "$PLIST"
launchctl enable "$UID_GUI/$LABEL"

echo "등록했습니다: $PLIST" >&2

# 첫 기동은 모델을 내려받느라 오래 걸린다.
for _ in $(seq 1 1800); do
  curl -sf -m 2 "$URL/v1/models" >/dev/null && break
  sleep 1
done

# 커널 컴파일과 첫 호출 비용을 첫 실제 요청이 떠안지 않게 noul과 choice를 한 번씩 보낸다.
ask() {
  curl -sf -m 120 "$URL/v1/systemone" -H 'content-type: application/json' -d "$1" >/dev/null ||
    echo "웜업 요청이 실패했습니다(서버는 계속 돕니다): $1" >&2
}
ask '{"model":"jev-latest","state":"warmup","questions":{"q":{"type":"noul","instructions":"참인가?"}}}'
ask '{"model":"jev-latest","state":"warmup","questions":{"q":{"type":"choice","instructions":"어느 쪽인가?","criteria":{"a":"a","b":"b"}}}}'

echo "준비됨: $URL/v1/systemone ($MODEL)" >&2
echo "로그: tail -f $HOME/Library/Logs/$LABEL.log" >&2
echo "상태: launchctl print $UID_GUI/$LABEL" >&2
echo "제거: scripts/install-launchd.sh --uninstall" >&2
