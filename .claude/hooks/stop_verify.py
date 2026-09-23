#!/usr/bin/env python3
# Stop 훅: 상주 guru 데몬(decide)에게 이 세션의 코드 변경이 테스트로 검증됐는지 물어, 근거가 약하면 완료를 막는다
import json
import socket
import subprocess
import sys
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parents[2]
SOCKET_PATH = Path.home() / ".cache" / "guru" / "decide.sock"
TEST_MARKERS = ("pytest", "npm test", "npm run test", "cargo test", "go test")
THRESHOLD = 0.4  # ponytail: 고정 임계값, 오탐 잦으면 조정
CONNECT_TIMEOUT_S = 0.5
REQUEST_TIMEOUT_S = 5.0


def read_transcript_tail(path_str: str, max_chars: int = 4000) -> str:
    if not path_str:
        return ""
    path = Path(path_str)
    if not path.exists():
        return ""
    return path.read_text(errors="ignore")[-max_chars:]


def mentions_test_run(transcript: str) -> bool:
    return any(marker in transcript for marker in TEST_MARKERS)


def git_diff_stat() -> str:
    staged = subprocess.run(
        ["git", "-C", str(REPO_ROOT), "diff", "--cached", "--stat"],
        capture_output=True, text=True, timeout=5,
    ).stdout
    unstaged = subprocess.run(
        ["git", "-C", str(REPO_ROOT), "diff", "--stat"],
        capture_output=True, text=True, timeout=5,
    ).stdout
    return (staged + unstaged).strip()


def should_block(confidence: float) -> bool:
    return confidence < THRESHOLD


def ensure_daemon_started() -> None:
    subprocess.Popen(
        [sys.executable, "-m", "guru.decide_daemon"],
        stdout=subprocess.DEVNULL,
        stderr=subprocess.DEVNULL,
        start_new_session=True,
    )


def ask_daemon(state: str) -> dict | None:
    request = json.dumps({
        "state": state,
        "type": "noul",
        "instructions": "이 세션에서 변경한 코드가 테스트로 검증되었는가",
    }) + "\n"

    try:
        with socket.socket(socket.AF_UNIX, socket.SOCK_STREAM) as sock:
            sock.settimeout(CONNECT_TIMEOUT_S)
            sock.connect(str(SOCKET_PATH))
            sock.settimeout(REQUEST_TIMEOUT_S)
            sock.sendall(request.encode())
            data = sock.makefile("rb").readline()
    except OSError:
        ensure_daemon_started()  # 다음 번 호출을 위해 백그라운드로 띄워두고, 이번엔 그냥 통과시킨다
        return None

    if not data:
        return None
    try:
        return json.loads(data)
    except json.JSONDecodeError:
        return None


def main() -> None:
    try:
        payload = json.load(sys.stdin)
    except Exception:
        return

    if payload.get("stop_hook_active"):
        return  # 이미 한 번 막았다면 무한루프 방지

    diff_stat = git_diff_stat()
    if not diff_stat:
        return  # 변경사항 없으면 검증할 것도 없음

    ran_tests = mentions_test_run(read_transcript_tail(payload.get("transcript_path", "")))
    state = (
        f"변경된 파일:\n{diff_stat}\n\n"
        f"세션 로그에서 테스트 실행 명령 흔적: {'있음' if ran_tests else '없음'}"
    )

    resp = ask_daemon(state)
    if not resp or "error" in resp:
        return  # guru 데몬 사용 불가하면 막지 않는다

    confidence = resp["answer"]["noul"]
    if should_block(confidence):
        print(json.dumps({
            "decision": "block",
            "reason": (
                f"코드 변경이 테스트로 검증됐다는 근거가 약합니다(신뢰도 {confidence:.2f}). "
                "테스트를 실행해 결과를 확인한 뒤 다시 완료하세요."
            ),
        }))


if __name__ == "__main__":
    main()
