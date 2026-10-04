// 게이트가 상주 decide 데몬에 소켓으로 질문하고, 데몬을 띄우고, 감사 로그를 쓰는 입출력
use serde_json::Value;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

#[derive(Debug, PartialEq)]
pub enum AskError {
    /// 소켓이 없거나 연결이 거부됐다(데몬이 꺼져 있다).
    Down,
    /// 데몬이 다른 버전이라 stale로 답했다(곧 종료한다).
    Stale,
    /// 제한 시간 안에 답하지 않았다.
    Timeout,
    /// 데몬이 오류로 답했다.
    Backend(String),
    /// 답이 JSON이 아니거나 읽을 수 없다.
    Invalid(String),
}

/// 데몬 소켓에 요청 한 줄을 보내고 답 한 줄을 JSON으로 읽는다.
pub fn ask(socket: &Path, request: &Value, timeout: Duration) -> Result<Value, AskError> {
    let mut stream = UnixStream::connect(socket).map_err(|_| AskError::Down)?;
    let _ = stream.set_write_timeout(Some(timeout));
    let _ = stream.set_read_timeout(Some(timeout));
    let mut line = request.to_string();
    line.push('\n');
    stream
        .write_all(line.as_bytes())
        .map_err(|err| AskError::Invalid(format!("요청을 보내지 못했습니다: {err}")))?;
    let mut reply = String::new();
    match BufReader::new(stream).read_line(&mut reply) {
        Ok(0) => return Err(AskError::Invalid("데몬이 답 없이 연결을 닫았습니다".to_string())),
        Ok(_) => {}
        Err(err) if matches!(err.kind(), std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut) => {
            return Err(AskError::Timeout);
        }
        Err(err) => return Err(AskError::Invalid(err.to_string())),
    }
    let value: Value =
        serde_json::from_str(reply.trim()).map_err(|err| AskError::Invalid(err.to_string()))?;
    if value.get("stale").and_then(Value::as_bool) == Some(true) {
        return Err(AskError::Stale);
    }
    if let Some(message) = value.get("error").and_then(Value::as_str) {
        return Err(AskError::Backend(message.to_string()));
    }
    Ok(value)
}

/// 연결이 더 이상 안 될 때까지(stale 데몬이 종료할 때까지) 최대 `max`만큼 기다린다. 사라졌으면 true.
pub fn wait_gone(socket: &Path, max: Duration) -> bool {
    let deadline = Instant::now() + max;
    loop {
        if UnixStream::connect(socket).is_err() {
            return true;
        }
        if Instant::now() >= deadline {
            return false;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// 질문하고, 데몬이 없거나 옛 버전이면 새 데몬을 띄운다. 이번 요청은 답을 못 받으므로 `Err(이유)`로 돌려준다
/// (호출자는 판정 없이 통과시킨다). 질문에 성공하면 `Ok(답)`이다. 옛 버전 데몬은 stale 응답 뒤 스스로 끝나므로
/// 소켓이 닫히기를 기다린 뒤 띄워야 새 데몬이 "이미 실행 중"으로 물러나지 않는다.
pub fn ask_or_start(
    socket: &Path,
    request: &Value,
    timeout: Duration,
    spawn: &mut dyn FnMut(),
) -> Result<Value, String> {
    match ask(socket, request, timeout) {
        Ok(answer) => Ok(answer),
        Err(AskError::Down) => {
            spawn();
            Err("데몬이 꺼져 있어 새로 시작합니다".to_string())
        }
        Err(AskError::Stale) => {
            wait_gone(socket, Duration::from_secs(1));
            spawn();
            Err("데몬 버전이 달라 새로 시작합니다".to_string())
        }
        Err(AskError::Timeout) => Err(format!("데몬이 {}ms 안에 답하지 않았습니다", timeout.as_millis())),
        Err(AskError::Backend(message)) => Err(format!("백엔드 오류: {message}")),
        Err(AskError::Invalid(message)) => Err(format!("데몬 답을 읽지 못했습니다: {message}")),
    }
}

/// 이 실행 파일의 `daemon` 서브커맨드를 입출력 없이 별도 프로세스 그룹으로 띄운다. 환경은 그대로 물려준다.
pub fn spawn_daemon() -> std::io::Result<()> {
    use std::os::unix::process::CommandExt;
    use std::process::{Command, Stdio};
    Command::new(std::env::current_exe()?)
        .arg("daemon")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .process_group(0)
        .spawn()
        .map(|_| ())
}

/// 감사 로그에 JSON 한 줄을 덧붙인다(디렉터리가 없으면 만든다).
pub fn append_audit(path: &Path, record: &Value) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let mut file = std::fs::OpenOptions::new().create(true).append(true).open(path)?;
    writeln!(file, "{record}")
}

/// 감사 로그 경로 `~/.cache/decide/gate.log`. HOME이 없거나 비면 `None`이다.
pub fn audit_path(home: Option<&str>) -> Option<PathBuf> {
    home.filter(|home| !home.is_empty())
        .map(|home| Path::new(home).join(".cache/decide/gate.log"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::os::unix::net::UnixListener;

    /// 유닉스 소켓 경로는 macOS에서 약 104바이트가 한계라 이름을 짧게 한다(레이블은 테스트마다 다르다).
    fn temp_dir(label: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("dgc-{label}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// 한 번만 받아 `reply`로 답하는 가짜 데몬. 받은 요청 줄을 돌려준다.
    fn fake_daemon(socket: &Path, reply: &'static str) -> std::thread::JoinHandle<String> {
        let listener = UnixListener::bind(socket).unwrap();
        std::thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            let mut line = String::new();
            BufReader::new(stream.try_clone().unwrap()).read_line(&mut line).unwrap();
            let mut writer = stream;
            if reply.is_empty() {
                // 침묵하는 데몬: 연결을 열어 둔 채 클라이언트가 먼저 시간 초과가 되게 한다.
                std::thread::sleep(Duration::from_millis(600));
            } else {
                writeln!(writer, "{reply}").unwrap();
            }
            line
        })
    }

    #[test]
    fn ask_sends_one_line_and_reads_the_answer() {
        let dir = temp_dir("ok");
        let socket = dir.join("d.sock");
        let server = fake_daemon(&socket, r#"{"answer":{"type":"choice"},"latency_ms":1.0}"#);
        let request = json!({"state": "s", "type": "choice"});
        let answer = ask(&socket, &request, Duration::from_secs(2)).unwrap();
        assert_eq!(answer["answer"]["type"], "choice");
        let received: Value = serde_json::from_str(server.join().unwrap().trim()).unwrap();
        assert_eq!(received, request);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn ask_reports_a_missing_socket_as_down() {
        let dir = temp_dir("down");
        let result = ask(&dir.join("nobody.sock"), &json!({}), Duration::from_millis(200));
        assert_eq!(result, Err(AskError::Down));
        // 소켓 파일은 있지만 듣는 이가 없는 경우(죽은 데몬)도 같다.
        let dead = dir.join("dead.sock");
        drop(UnixListener::bind(&dead).unwrap());
        assert_eq!(ask(&dead, &json!({}), Duration::from_millis(200)), Err(AskError::Down));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn ask_times_out_when_the_daemon_stays_silent() {
        let dir = temp_dir("timeout");
        let socket = dir.join("d.sock");
        let server = fake_daemon(&socket, "");
        let started = Instant::now();
        let result = ask(&socket, &json!({}), Duration::from_millis(150));
        assert_eq!(result, Err(AskError::Timeout));
        assert!(started.elapsed() < Duration::from_secs(2), "{:?}", started.elapsed());
        let _ = server.join();
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn ask_maps_error_stale_and_garbage_replies() {
        for (reply, expected) in [
            (r#"{"error":"백엔드 실패"}"#, AskError::Backend("백엔드 실패".into())),
            (r#"{"stale":true,"version":"9.9.9"}"#, AskError::Stale),
        ] {
            let dir = temp_dir("map");
            let socket = dir.join("d.sock");
            let server = fake_daemon(&socket, reply);
            assert_eq!(ask(&socket, &json!({}), Duration::from_secs(2)), Err(expected));
            let _ = server.join();
            let _ = std::fs::remove_dir_all(&dir);
        }
        let dir = temp_dir("garbage");
        let socket = dir.join("d.sock");
        let server = fake_daemon(&socket, "not json");
        assert!(matches!(
            ask(&socket, &json!({}), Duration::from_secs(2)),
            Err(AskError::Invalid(_))
        ));
        let _ = server.join();
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn ask_or_start_starts_a_daemon_when_none_is_running() {
        let dir = temp_dir("start");
        let mut spawned = 0;
        let result = ask_or_start(&dir.join("none.sock"), &json!({}), Duration::from_millis(200), &mut || {
            spawned += 1;
        });
        assert_eq!(spawned, 1);
        let reason = result.unwrap_err();
        assert!(reason.contains("데몬") && reason.contains("시작"), "{reason}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn ask_or_start_replaces_a_stale_daemon_after_it_is_gone() {
        let dir = temp_dir("stale");
        let socket = dir.join("d.sock");
        let server = fake_daemon(&socket, r#"{"stale":true,"version":"0.0.1"}"#);
        let mut spawned = 0;
        let result = ask_or_start(&socket, &json!({}), Duration::from_secs(2), &mut || spawned += 1);
        let _ = server.join(); // 가짜 데몬은 답한 뒤 끝나고, 리스너가 닫혀 연결이 안 된다.
        assert_eq!(spawned, 1);
        assert!(result.unwrap_err().contains("버전"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn ask_or_start_does_not_start_anything_on_success_timeout_or_backend_error() {
        let dir = temp_dir("nostart");
        let socket = dir.join("d.sock");
        let server = fake_daemon(&socket, r#"{"answer":{}}"#);
        let mut spawned = 0;
        let ok = ask_or_start(&socket, &json!({}), Duration::from_secs(2), &mut || spawned += 1);
        assert!(ok.is_ok());
        let _ = server.join();

        let socket2 = dir.join("e.sock");
        let server = fake_daemon(&socket2, r#"{"error":"x"}"#);
        let backend = ask_or_start(&socket2, &json!({}), Duration::from_secs(2), &mut || spawned += 1);
        assert!(backend.unwrap_err().contains("x"));
        let _ = server.join();

        let socket3 = dir.join("f.sock");
        let server = fake_daemon(&socket3, "");
        let slow = ask_or_start(&socket3, &json!({}), Duration::from_millis(100), &mut || spawned += 1);
        assert!(slow.unwrap_err().contains("100"));
        let _ = server.join();
        assert_eq!(spawned, 0);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn wait_gone_returns_when_nobody_listens_and_gives_up_otherwise() {
        let dir = temp_dir("wait");
        assert!(wait_gone(&dir.join("never.sock"), Duration::from_millis(200)));
        let socket = dir.join("live.sock");
        let _listener = UnixListener::bind(&socket).unwrap();
        let started = Instant::now();
        assert!(!wait_gone(&socket, Duration::from_millis(150)));
        assert!(started.elapsed() >= Duration::from_millis(140));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn append_audit_creates_the_directory_and_appends_lines() {
        let dir = temp_dir("audit");
        let path = dir.join("nested/dir/gate.log");
        append_audit(&path, &json!({"gate": "bash-risk", "verdict": "ask"})).unwrap();
        append_audit(&path, &json!({"gate": "bash-risk", "verdict": "deny"})).unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        let lines: Vec<Value> = text.lines().map(|line| serde_json::from_str(line).unwrap()).collect();
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0]["verdict"], "ask");
        assert_eq!(lines[1]["verdict"], "deny");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn audit_path_lives_under_the_cache_dir() {
        assert_eq!(audit_path(Some("/home/u")), Some(PathBuf::from("/home/u/.cache/decide/gate.log")));
        assert_eq!(audit_path(None), None);
        assert_eq!(audit_path(Some("")), None);
    }
}
