use crate::backend::{decide, nonempty, Env};
use crate::typesafe::{LiveTransport, Transport};
use serde_json::{json, Value};
use std::io::{BufRead, BufReader, Write};
use std::os::fd::AsRawFd;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

pub fn default_socket_path() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".into());
    PathBuf::from(home).join(".cache/decide/decide.sock")
}

pub fn claim_socket(path: &Path) -> std::io::Result<bool> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    if !path.exists() {
        return Ok(true);
    }
    match UnixStream::connect(path) {
        Ok(_) => Ok(false),
        Err(_) => {
            std::fs::remove_file(path)?;
            Ok(true)
        }
    }
}

pub fn handle_line<T: Transport>(line: &str, env: &Env, transport: &mut T) -> Option<String> {
    if line.trim().is_empty() {
        return None;
    }
    let payload = match serde_json::from_str::<Value>(line) {
        Ok(raw) => match call(&raw, env, transport) {
            Ok(result) => {
                serde_json::to_string(&result).unwrap_or_else(|err| error_json(&err.to_string()))
            }
            Err(err) => error_json(&err),
        },
        Err(err) => error_json(&err.to_string()),
    };
    Some(payload)
}

fn call<T: Transport>(
    raw: &Value,
    env: &Env,
    transport: &mut T,
) -> Result<crate::protocol::DecideResult, String> {
    let origin = Instant::now();
    decide(
        raw,
        env,
        transport,
        || origin.elapsed().as_secs_f64() * 1000.0,
        || std::thread::sleep(Duration::from_secs(1)),
    )
}

fn error_json(message: &str) -> String {
    serde_json::to_string(&json!({"error": message}))
        .unwrap_or_else(|_| r#"{"error":"직렬화에 실패했습니다"}"#.into())
}

pub fn serve(path: &Path, idle: Duration) -> std::io::Result<()> {
    if !claim_socket(path)? {
        eprintln!("daemon already running at {}", path.display());
        return Ok(());
    }
    let listener = UnixListener::bind(path)?;
    let _guard = SocketGuard(path.to_path_buf());
    listener.set_nonblocking(true)?;
    let env = Env::from_process();
    let key = nonempty(env.api_key.as_deref()).unwrap_or("");
    let mut transport = LiveTransport::new(key);
    loop {
        match accept_within(&listener, idle)? {
            Some(stream) => handle_connection(stream, &env, &mut transport),
            None => break,
        }
    }
    Ok(())
}

pub fn serve_default() -> std::io::Result<()> {
    serve(&default_socket_path(), Duration::from_secs(30 * 60))
}

fn handle_connection<T: Transport>(stream: UnixStream, env: &Env, transport: &mut T) {
    let _ = stream.set_read_timeout(None);
    let mut writer = match stream.try_clone() {
        Ok(writer) => writer,
        Err(_) => return,
    };
    let mut line = String::new();
    if BufReader::new(stream).read_line(&mut line).is_err() {
        return;
    }
    let Some(payload) = handle_line(&line, env, transport) else {
        return;
    };
    let _ = writer.write_all(payload.as_bytes());
    let _ = writer.write_all(b"\n");
    let _ = writer.flush();
}

struct SocketGuard(PathBuf);

impl Drop for SocketGuard {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

fn accept_within(listener: &UnixListener, idle: Duration) -> std::io::Result<Option<UnixStream>> {
    let deadline = Instant::now() + idle;
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        let timeout_ms = libc::c_int::try_from(remaining.as_millis()).unwrap_or(libc::c_int::MAX);
        if timeout_ms == 0 {
            return Ok(None);
        }
        let mut pollfd = libc::pollfd {
            fd: listener.as_raw_fd(),
            events: libc::POLLIN,
            revents: 0,
        };
        let rc = unsafe { libc::poll(&mut pollfd, 1, timeout_ms) };
        if rc < 0 {
            let err = std::io::Error::last_os_error();
            if err.kind() == std::io::ErrorKind::Interrupted {
                continue;
            }
            return Err(err);
        }
        if rc == 0 {
            return Ok(None);
        }
        match listener.accept() {
            Ok((stream, _)) => return Ok(Some(stream)),
            Err(err)
                if err.kind() == std::io::ErrorKind::WouldBlock
                    || err.kind() == std::io::ErrorKind::Interrupted =>
            {
                continue
            }
            Err(err) => return Err(err),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::local;
    use crate::typesafe::{RawResponse, Transport};
    use std::cell::Cell;

    struct Script {
        calls: Cell<usize>,
    }

    impl Transport for Script {
        fn post_json(&mut self, _body: &Value) -> Result<RawResponse, String> {
            self.calls.set(self.calls.get() + 1);
            Ok(RawResponse {
                status: 200,
                body: r#"{"model":"jev-1.13.0","answers":{"q":{"type":"noul","noul":0.8}}}"#.into(),
            })
        }
    }

    #[test]
    fn line_protocol_returns_one_json_object() {
        let env = Env {
            backend: None,
            api_key: Some("k".into()),
        };
        let mut script = Script {
            calls: Cell::new(0),
        };
        let line = handle_line(
            r#"{"state":"s","type":"noul","instructions":"참인가?"}"#,
            &env,
            &mut script,
        )
        .unwrap();
        let parsed: Value = serde_json::from_str(&line).unwrap();
        assert_eq!(parsed["answer"]["noul"], 0.8);
        assert_eq!(parsed["routing"]["backend"], "typesafe");
        assert!(parsed["latency_ms"].is_number());

        let env = Env {
            backend: Some("local".into()),
            api_key: None,
        };
        let err_line = handle_line(
            r#"{"state":"s","type":"noul","instructions":"참인가?"}"#,
            &env,
            &mut script,
        )
        .unwrap();
        let parsed: Value = serde_json::from_str(&err_line).unwrap();
        assert_eq!(parsed["error"], local::NOT_READY);
        assert!(handle_line("\n", &env, &mut script).is_none());
        assert_eq!(script.calls.get(), 1);
    }

    #[test]
    fn claim_replaces_a_dead_file_and_keeps_a_live_socket() {
        let dir = std::env::temp_dir().join(format!("decide-claim-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("decide.sock");
        std::fs::write(&path, b"stale").unwrap();
        assert!(claim_socket(&path).unwrap());
        assert!(!path.exists());

        let listener = UnixListener::bind(&path).unwrap();
        assert!(!claim_socket(&path).unwrap());
        assert!(path.exists());
        drop(listener);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn idle_timeout_unlinks_the_socket() {
        let dir = std::env::temp_dir().join(format!(
            "decide-idle-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("decide.sock");
        serve(&path, Duration::from_millis(300)).unwrap();
        assert!(!path.exists());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
