use crate::backend::{decide, decide_many, live_transport, select_backend, Backend, Env};
use crate::protocol::{parse_arguments, parse_many, Incoming, IncomingMany};
use crate::typesafe::Transport;
use serde_json::{json, Value};
use std::collections::{HashMap, VecDeque};
use std::io::{BufRead, BufReader, Write};
use std::os::fd::AsRawFd;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

const MAX_CACHE_ENTRIES: usize = 64;

/// 이 데몬 바이너리의 버전. 요청의 `client_version`과 비교한다.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

pub const TYPE_WITH_QUESTIONS: &str = "type과 questions는 함께 쓸 수 없습니다";

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
enum CacheKey {
    One {
        backend: Backend,
        incoming: Incoming,
    },
    Many {
        backend: Backend,
        incoming: IncomingMany,
    },
}

#[derive(Default)]
pub struct Cache {
    entries: HashMap<CacheKey, Value>,
    order: VecDeque<CacheKey>,
}

impl Cache {
    fn get(&self, key: &CacheKey) -> Option<&Value> {
        self.entries.get(key)
    }

    fn insert(&mut self, key: CacheKey, value: Value) {
        if !self.entries.contains_key(&key) {
            self.order.push_back(key.clone());
            if self.order.len() > MAX_CACHE_ENTRIES {
                if let Some(oldest) = self.order.pop_front() {
                    self.entries.remove(&oldest);
                }
            }
        }
        self.entries.insert(key, value);
    }
}

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

pub fn handle_line<T: Transport>(
    line: &str,
    env: &Env,
    transport: &mut T,
    cache: &mut Cache,
) -> Option<String> {
    handle_request(line, env, transport, cache).map(|(payload, _)| payload)
}

/// 한 줄 요청을 처리해 `(응답, 계속 서비스할지)`를 돌려준다. 요청의 선택 필드 `client_version`이 이
/// 데몬의 버전과 다르면 백엔드를 부르지 않고 `{"stale":true,"version":…}`로 답하며 데몬은 끝난다 —
/// 업그레이드 뒤에도 옛 바이너리의 데몬이 계속 답하는 것을 막는다. 필드가 없는 옛 클라이언트는 그대로 동작한다.
pub fn handle_request<T: Transport>(
    line: &str,
    env: &Env,
    transport: &mut T,
    cache: &mut Cache,
) -> Option<(String, bool)> {
    if line.trim().is_empty() {
        return None;
    }
    let raw = match serde_json::from_str::<Value>(line) {
        Ok(raw) => raw,
        Err(err) => return Some((error_json(&err.to_string()), true)),
    };
    if let Some(client) = raw.get("client_version").and_then(Value::as_str) {
        if client != VERSION {
            return Some((json!({"stale": true, "version": VERSION}).to_string(), false));
        }
    }
    let payload = match call(&raw, env, transport, cache) {
        Ok(result) => {
            serde_json::to_string(&result).unwrap_or_else(|err| error_json(&err.to_string()))
        }
        Err(err) => error_json(&err),
    };
    Some((payload, true))
}

fn call<T: Transport>(
    raw: &Value,
    env: &Env,
    transport: &mut T,
    cache: &mut Cache,
) -> Result<Value, String> {
    let many = raw.get("questions").is_some();
    if many && raw.get("type").is_some() {
        return Err(TYPE_WITH_QUESTIONS.to_string());
    }
    let backend = select_backend(env.backend.as_deref(), env.api_key.as_deref()).ok();
    let cache_key = backend.and_then(|backend| {
        if many {
            parse_many(raw)
                .ok()
                .map(|incoming| CacheKey::Many { backend, incoming })
        } else {
            parse_arguments(raw)
                .ok()
                .map(|incoming| CacheKey::One { backend, incoming })
        }
    });
    if let Some(key) = &cache_key {
        if let Some(cached) = cache.get(key) {
            let mut result = cached.clone();
            result["routing"]["cached"] = json!(true);
            result["latency_ms"] = json!(0.0);
            return Ok(result);
        }
    }
    let origin = Instant::now();
    let clock = || origin.elapsed().as_secs_f64() * 1000.0;
    let pause = || std::thread::sleep(Duration::from_secs(1));
    let result = if many {
        serde_json::to_value(decide_many(raw, env, transport, clock, pause)?)
    } else {
        serde_json::to_value(decide(raw, env, transport, clock, pause)?)
    }
    .map_err(|err| err.to_string())?;
    if let Some(key) = cache_key {
        cache.insert(key, result.clone());
    }
    Ok(result)
}

fn error_json(message: &str) -> String {
    serde_json::to_string(&json!({"error": message}))
        .unwrap_or_else(|_| r#"{"error":"직렬화에 실패했습니다"}"#.into())
}

/// 게이트용 Unix 소켓을 서빙한다. 이미 데몬이 떠 있으면(소켓이 살아 있으면) 바로 끝난다.
pub fn serve(uds_path: &Path, idle: Duration) -> std::io::Result<()> {
    if !claim_socket(uds_path)? {
        eprintln!("daemon already running");
        return Ok(());
    }
    let cache = Arc::new(Mutex::new(Cache::default()));
    let last_activity = Arc::new(Mutex::new(Instant::now()));
    serve_uds_with_activity(uds_path, idle, cache, last_activity)
}

fn serve_uds_with_activity(
    path: &Path,
    idle: Duration,
    cache: Arc<Mutex<Cache>>,
    last_activity: Arc<Mutex<Instant>>,
) -> std::io::Result<()> {
    let listener = UnixListener::bind(path)?;
    let _guard = SocketGuard(path.to_path_buf());
    listener.set_nonblocking(true)?;
    let env = Env::from_process();
    let mut transport = live_transport(&env);
    loop {
        match accept_within(&listener, Duration::from_secs(1))? {
            Some(stream) => {
                if !handle_connection_shared(stream, &env, &mut transport, &cache) {
                    break;
                }
                *last_activity.lock().unwrap() = Instant::now();
            }
            None => {
                if last_activity.lock().unwrap().elapsed() >= idle {
                    break;
                }
            }
        }
    }
    Ok(())
}

pub fn serve_default() -> std::io::Result<()> {
    serve(&default_socket_path(), Duration::from_secs(30 * 60))
}

/// 연결 하나를 처리한다. 계속 서비스해야 하면 true, stale 응답을 했으면 false(데몬이 끝난다).
fn handle_connection_shared<T: Transport>(
    stream: UnixStream,
    env: &Env,
    transport: &mut T,
    cache: &Arc<Mutex<Cache>>,
) -> bool {
    // 리스너는 idle 종료를 위해 논블로킹이고, macOS(BSD)에서는 accept한 연결도 그 모드를 물려받는다.
    // 블로킹으로 되돌리지 않으면 아직 도착하지 않은 요청에 WouldBlock을 받고 연결을 닫아 버린다.
    let _ = stream.set_nonblocking(false);
    let _ = stream.set_read_timeout(None);
    let mut writer = match stream.try_clone() {
        Ok(writer) => writer,
        Err(_) => return true,
    };
    let mut line = String::new();
    if BufReader::new(stream).read_line(&mut line).is_err() {
        return true;
    }
    let mut locked = cache.lock().unwrap();
    let Some((payload, keep_serving)) = handle_request(&line, env, transport, &mut locked) else {
        return true;
    };
    drop(locked);
    let _ = writer.write_all(payload.as_bytes());
    let _ = writer.write_all(b"\n");
    let _ = writer.flush();
    keep_serving
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
        let mut cache = Cache::default();
        let line = handle_line(
            r#"{"state":"s","type":"noul","instructions":"참인가?"}"#,
            &env,
            &mut script,
            &mut cache,
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
        // 로컬도 같은 transport를 타고, 백엔드가 다르니 위 TypeSafe 응답을 캐시에서 받지 않는다.
        let line = handle_line(
            r#"{"state":"s","type":"noul","instructions":"참인가?"}"#,
            &env,
            &mut script,
            &mut cache,
        )
        .unwrap();
        let parsed: Value = serde_json::from_str(&line).unwrap();
        assert_eq!(parsed["routing"]["backend"], "local");
        assert_eq!(parsed["routing"].get("cached"), None);
        assert!(handle_line("\n", &env, &mut script, &mut cache).is_none());
        assert_eq!(script.calls.get(), 2);
    }

    #[test]
    fn identical_requests_hit_the_cache_and_skip_the_transport() {
        let env = Env {
            backend: None,
            api_key: Some("k".into()),
        };
        let mut script = Script {
            calls: Cell::new(0),
        };
        let mut cache = Cache::default();
        let request = r#"{"state":"s","type":"noul","instructions":"참인가?"}"#;

        let first = handle_line(request, &env, &mut script, &mut cache).unwrap();
        let first: Value = serde_json::from_str(&first).unwrap();
        assert_eq!(first["routing"].get("cached"), None);
        assert_eq!(script.calls.get(), 1);

        let second = handle_line(request, &env, &mut script, &mut cache).unwrap();
        let second: Value = serde_json::from_str(&second).unwrap();
        assert_eq!(second["answer"]["noul"], 0.8);
        assert_eq!(second["routing"]["cached"], true);
        assert_eq!(script.calls.get(), 1, "캐시가 있으면 두 번째 호출은 전송을 타지 않는다");
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

    const NOUL_LINE: &str = r#"{"state":"s","type":"noul","instructions":"참인가?"}"#;

    fn with_client_version(version: &str) -> String {
        format!(
            r#"{{"state":"s","type":"noul","instructions":"참인가?","client_version":"{version}"}}"#
        )
    }

    #[test]
    fn a_matching_client_version_is_answered_normally_and_keeps_serving() {
        let env = Env {
            backend: None,
            api_key: Some("k".into()),
        };
        let mut script = Script {
            calls: Cell::new(0),
        };
        let mut cache = Cache::default();
        let (payload, keep_serving) =
            handle_request(&with_client_version(VERSION), &env, &mut script, &mut cache).unwrap();
        let parsed: Value = serde_json::from_str(&payload).unwrap();
        assert_eq!(parsed["answer"]["noul"], 0.8);
        assert!(keep_serving);
        assert_eq!(script.calls.get(), 1);
    }

    #[test]
    fn a_different_client_version_gets_a_stale_reply_without_a_backend_call() {
        let env = Env {
            backend: None,
            api_key: Some("k".into()),
        };
        let mut script = Script {
            calls: Cell::new(0),
        };
        let mut cache = Cache::default();
        let (payload, keep_serving) =
            handle_request(&with_client_version("0.0.1"), &env, &mut script, &mut cache).unwrap();
        let parsed: Value = serde_json::from_str(&payload).unwrap();
        assert_eq!(parsed, json!({"stale": true, "version": VERSION}));
        assert!(!keep_serving, "stale 응답 뒤 데몬은 끝나야 한다");
        assert_eq!(script.calls.get(), 0);
    }

    #[test]
    fn a_request_without_a_client_version_behaves_as_before() {
        let env = Env {
            backend: None,
            api_key: Some("k".into()),
        };
        let mut script = Script {
            calls: Cell::new(0),
        };
        let mut cache = Cache::default();
        let (payload, keep_serving) = handle_request(NOUL_LINE, &env, &mut script, &mut cache).unwrap();
        assert!(serde_json::from_str::<Value>(&payload).unwrap().get("stale").is_none());
        assert!(keep_serving);
    }

    #[test]
    fn serve_replies_stale_then_exits_and_unlinks_the_socket() {
        let dir = std::env::temp_dir().join(format!(
            "decide-stale-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("decide.sock");
        let server_path = path.clone();
        // 유휴 종료(10초)가 아니라 stale 때문에 끝나는지 보려고 일부러 길게 잡는다.
        let server = std::thread::spawn(move || {
            serve_uds_with_activity(
                &server_path,
                Duration::from_secs(10),
                Arc::new(Mutex::new(Cache::default())),
                Arc::new(Mutex::new(Instant::now())),
            )
        });
        let started = Instant::now();
        while !path.exists() {
            assert!(started.elapsed() < Duration::from_secs(5), "소켓이 생기지 않았다");
            std::thread::sleep(Duration::from_millis(10));
        }
        let mut stream = UnixStream::connect(&path).unwrap();
        writeln!(stream, "{}", with_client_version("0.0.1")).unwrap();
        let mut reply = String::new();
        BufReader::new(stream).read_line(&mut reply).unwrap();
        assert_eq!(
            serde_json::from_str::<Value>(&reply).unwrap(),
            json!({"stale": true, "version": VERSION})
        );
        server.join().unwrap().unwrap();
        assert!(started.elapsed() < Duration::from_secs(5), "stale 뒤 바로 끝나야 한다");
        assert!(!path.exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_client_that_sends_its_request_late_still_gets_an_answer() {
        // macOS(BSD)에서는 논블로킹 리스너가 accept한 연결도 논블로킹을 물려받는다. 데몬이 accept 직후
        // 바로 읽다가 아직 도착하지 않은 요청에 WouldBlock을 받고 연결을 닫아 버리면, 느리게 쓰는
        // 클라이언트는 BrokenPipe나 빈 응답을 받는다. 연결한 뒤 한참 있다가 보내도 답해야 한다.
        let dir = std::env::temp_dir().join(format!("dgd-late-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("d.sock");
        let server_path = path.clone();
        let server = std::thread::spawn(move || {
            serve_uds_with_activity(
                &server_path,
                Duration::from_secs(10),
                Arc::new(Mutex::new(Cache::default())),
                Arc::new(Mutex::new(Instant::now())),
            )
        });
        let started = Instant::now();
        while !path.exists() {
            assert!(started.elapsed() < Duration::from_secs(5), "소켓이 생기지 않았다");
            std::thread::sleep(Duration::from_millis(10));
        }
        let mut stream = UnixStream::connect(&path).unwrap();
        std::thread::sleep(Duration::from_millis(250));
        // stale 응답 경로를 써서 백엔드 없이 답을 받는다. 이 답이 오면 데몬도 끝난다.
        writeln!(stream, "{}", with_client_version("0.0.1")).expect("늦게 보내도 연결이 살아 있어야 한다");
        let mut reply = String::new();
        BufReader::new(stream).read_line(&mut reply).unwrap();
        assert_eq!(
            serde_json::from_str::<Value>(&reply).expect("빈 응답이면 연결이 먼저 닫힌 것이다"),
            json!({"stale": true, "version": VERSION})
        );
        server.join().unwrap().unwrap();
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
        serve_uds_with_activity(
            &path,
            Duration::from_millis(300),
            Arc::new(Mutex::new(Cache::default())),
            Arc::new(Mutex::new(Instant::now())),
        )
        .unwrap();
        assert!(!path.exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn serve_binds_the_socket_answers_and_a_second_daemon_backs_off() {
        // 기본 소켓 경로를 쓰면 다른 테스트나 실제 데몬과 충돌하므로, serve_default가 아니라
        // 그 내부 로직을 그대로 쓰는 임시 경로로 돈다.
        let dir = std::env::temp_dir().join(format!(
            "decide-serve-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let socket_path = dir.join("d.sock");

        let socket_for_thread = socket_path.clone();
        let server = std::thread::spawn(move || serve(&socket_for_thread, Duration::from_secs(5)));

        let started = Instant::now();
        while !socket_path.exists() {
            assert!(started.elapsed() < Duration::from_secs(5), "UDS 소켓이 생기지 않았다");
            std::thread::sleep(Duration::from_millis(10));
        }
        // 이미 떠 있는 데몬이 있으면 두 번째는 아무것도 하지 않고 바로 끝난다.
        serve(&socket_path, Duration::from_secs(5)).unwrap();
        assert!(socket_path.exists(), "두 번째 데몬이 살아 있는 소켓을 지우면 안 된다");

        let mut stream = UnixStream::connect(&socket_path).unwrap();
        writeln!(stream, "{}", with_client_version("0.0.1")).unwrap();
        let mut reply = String::new();
        BufReader::new(stream).read_line(&mut reply).unwrap();
        assert_eq!(
            serde_json::from_str::<Value>(&reply).unwrap(),
            json!({"stale": true, "version": VERSION})
        );

        server.join().unwrap().unwrap();
        let _ = std::fs::remove_dir_all(&dir);
    }

    struct ManyScript {
        calls: Cell<usize>,
    }

    impl Transport for ManyScript {
        fn post_json(&mut self, _body: &Value) -> Result<RawResponse, String> {
            self.calls.set(self.calls.get() + 1);
            Ok(RawResponse {
                status: 200,
                body: r#"{"model":"jev-1.13.0","answers":{"a":{"type":"noul","noul":0.2},"b":{"type":"noul","noul":0.6},"q":{"type":"noul","noul":0.4}}}"#.into(),
            })
        }
    }

    fn typesafe_env() -> Env {
        Env {
            backend: None,
            api_key: Some("k".into()),
        }
    }

    const MANY_LINE: &str = r#"{"state":"s","questions":{"a":{"type":"noul","instructions":"참인가?"},"b":{"type":"noul","instructions":"급한가?"}}}"#;

    #[test]
    fn questions_line_returns_answers_and_hits_the_cache() {
        let mut script = ManyScript {
            calls: Cell::new(0),
        };
        let mut cache = Cache::default();
        let first = handle_line(MANY_LINE, &typesafe_env(), &mut script, &mut cache).unwrap();
        let first: Value = serde_json::from_str(&first).unwrap();
        assert_eq!(first["answers"]["a"]["noul"], 0.2);
        assert_eq!(first["answers"]["b"]["noul"], 0.6);
        assert_eq!(first["routing"]["backend"], "typesafe");
        assert_eq!(first["routing"].get("cached"), None);
        assert!(first.get("answer").is_none());
        assert_eq!(script.calls.get(), 1);

        let second = handle_line(MANY_LINE, &typesafe_env(), &mut script, &mut cache).unwrap();
        let second: Value = serde_json::from_str(&second).unwrap();
        assert_eq!(second["answers"]["b"]["noul"], 0.6);
        assert_eq!(second["routing"]["cached"], true);
        assert_eq!(second["latency_ms"], 0.0);
        assert_eq!(script.calls.get(), 1, "동일 요청은 전송을 타지 않는다");
    }

    #[test]
    fn reordered_questions_are_a_different_cache_entry() {
        let mut script = ManyScript {
            calls: Cell::new(0),
        };
        let mut cache = Cache::default();
        handle_line(MANY_LINE, &typesafe_env(), &mut script, &mut cache).unwrap();
        let reordered = r#"{"state":"s","questions":{"b":{"type":"noul","instructions":"급한가?"},"a":{"type":"noul","instructions":"참인가?"}}}"#;
        handle_line(reordered, &typesafe_env(), &mut script, &mut cache).unwrap();
        assert_eq!(script.calls.get(), 2);
    }

    #[test]
    fn type_together_with_questions_is_rejected_without_a_call() {
        let mut script = ManyScript {
            calls: Cell::new(0),
        };
        let mut cache = Cache::default();
        let line = r#"{"state":"s","type":"noul","instructions":"?","questions":{"a":{"type":"noul","instructions":"?"}}}"#;
        let out = handle_line(line, &typesafe_env(), &mut script, &mut cache).unwrap();
        let out: Value = serde_json::from_str(&out).unwrap();
        assert_eq!(out["error"], "type과 questions는 함께 쓸 수 없습니다");
        assert_eq!(script.calls.get(), 0);
    }

    #[test]
    fn single_and_many_with_the_same_content_do_not_share_a_cache_entry() {
        let mut script = ManyScript {
            calls: Cell::new(0),
        };
        let mut cache = Cache::default();
        let single = r#"{"state":"s","type":"noul","instructions":"참인가?"}"#;
        let many = r#"{"state":"s","questions":{"q":{"type":"noul","instructions":"참인가?"}}}"#;
        let one = handle_line(single, &typesafe_env(), &mut script, &mut cache).unwrap();
        let one: Value = serde_json::from_str(&one).unwrap();
        assert_eq!(one["answer"]["noul"], 0.4);
        let out = handle_line(many, &typesafe_env(), &mut script, &mut cache).unwrap();
        let out: Value = serde_json::from_str(&out).unwrap();
        assert_eq!(out["answers"]["q"]["noul"], 0.4);
        assert!(out["routing"].get("cached").is_none());
        assert_eq!(script.calls.get(), 2, "단일과 다중은 캐시를 공유하지 않는다");
    }
}
