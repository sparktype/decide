// Stop 훅(stop_verify.py)의 테스트 흔적 판별과 차단 임계값을 python3로 검사한다
use std::path::Path;
use std::process::Command;

const CHECK: &str = r#"
import importlib.util, sys
spec = importlib.util.spec_from_file_location("stop_verify", sys.argv[1])
m = importlib.util.module_from_spec(spec)
spec.loader.exec_module(m)
expected = float(sys.argv[2])
assert m.THRESHOLD == expected, (m.THRESHOLD, expected)
assert m.mentions_test_run("ran: pytest -q tests/")
assert m.mentions_test_run("$ cargo test")
assert not m.mentions_test_run("just editing files, no test command")
assert m.should_block(0.0)
assert m.should_block(expected - 0.01)
assert not m.should_block(expected)
assert not m.should_block(1.0)
"#;

fn check(threshold_env: Option<&str>, expected: &str) {
    let hook = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../.claude/hooks/stop_verify.py");
    let mut command = Command::new("python3");
    command.args(["-c", CHECK, hook.to_str().unwrap(), expected]);
    command.env_remove("DECIDE_STOP_THRESHOLD");
    if let Some(value) = threshold_env {
        command.env("DECIDE_STOP_THRESHOLD", value);
    }
    let output = command.output().expect("python3를 실행할 수 없다");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn default_threshold_is_0_4() {
    check(None, "0.4");
}

#[test]
fn threshold_reads_the_env_and_falls_back_on_garbage() {
    check(Some("0.7"), "0.7");
    check(Some("abc"), "0.4");
}
