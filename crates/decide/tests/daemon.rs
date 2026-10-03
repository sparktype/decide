use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::process::{Command, Stdio};
use std::time::Duration;

#[test]
fn daemon_returns_one_line_and_stays_up_after_an_error() {
    // macOS unix socket paths must stay under 104 bytes. `std::env::temp_dir()`
    // is `/var/folders/...` and makes `~/.cache/decide/decide.sock` too long.
    let home = std::path::PathBuf::from(format!(
        "/tmp/dd-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let _ = std::fs::remove_dir_all(&home);
    std::fs::create_dir_all(&home).unwrap();
    let sock = home.join(".cache/decide/decide.sock");
    // 가중치가 없는 빈 디렉터리를 `CLEF_WEIGHTS`로 지정해, 네트워크 다운로드
    // 없이 결정론적으로 "가중치를 찾을 수 없다" 오류를 받는다 — 로컬
    // 백엔드가 더 이상 HTTP(`jev-style serve`)를 호출하지 않고
    // `local::infer`를 직접 호출하므로, 실패 모드도 가중치 유무로 바뀌었다.
    let weights = home.join("empty-weights");
    std::fs::create_dir_all(&weights).unwrap();
    let mut child = Command::new(env!("CARGO_BIN_EXE_decide"))
        .arg("daemon")
        .env("HOME", &home)
        .env("DECIDE_BACKEND", "local")
        .env("CLEF_WEIGHTS", &weights)
        .env_remove("TYPESAFE_API_KEY")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();

    let mut connected = None;
    for _ in 0..100 {
        if let Ok(stream) = UnixStream::connect(&sock) {
            connected = Some(stream);
            break;
        }
        if child.try_wait().unwrap().is_some() {
            break;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    let stream = match connected {
        Some(stream) => stream,
        None => {
            let mut err = String::new();
            if let Some(mut stderr) = child.stderr.take() {
                let _ = std::io::Read::read_to_string(&mut stderr, &mut err);
            }
            let _ = child.kill();
            let _ = child.wait();
            let _ = std::fs::remove_dir_all(&home);
            panic!("데몬 소켓이 열리지 않았다: {err}");
        }
    };

    let first = roundtrip(stream);
    assert!(first["error"].as_str().unwrap().contains("찾을 수 없습니다"));
    assert!(child.try_wait().unwrap().is_none());

    let second = UnixStream::connect(&sock).expect("오류 뒤에도 소켓이 살아 있어야 한다");
    let again = roundtrip(second);
    assert!(again["error"].as_str().unwrap().contains("찾을 수 없습니다"));

    let _ = child.kill();
    let _ = child.wait();
    let _ = std::fs::remove_dir_all(&home);
}

fn roundtrip(mut stream: UnixStream) -> serde_json::Value {
    stream
        .set_read_timeout(Some(Duration::from_secs(2)))
        .unwrap();
    stream
        .write_all(r#"{"state":"한글","type":"noul","instructions":"참인가?"}"#.as_bytes())
        .unwrap();
    stream.write_all(b"\n").unwrap();
    let mut line = String::new();
    BufReader::new(stream).read_line(&mut line).unwrap();
    serde_json::from_str(&line).unwrap()
}
