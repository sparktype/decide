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
    let mut child = Command::new(env!("CARGO_BIN_EXE_decide"))
        .arg("daemon")
        .env("HOME", &home)
        .env("DECIDE_BACKEND", "local")
        .env("DECIDE_LOCAL_URL", "http://127.0.0.1:1/v1/systemone")
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
    assert!(first["error"].as_str().unwrap().contains("jev-style serve"));
    assert!(child.try_wait().unwrap().is_none());

    let second = UnixStream::connect(&sock).expect("오류 뒤에도 소켓이 살아 있어야 한다");
    let again = roundtrip(second);
    assert!(again["error"].as_str().unwrap().contains("jev-style serve"));

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
