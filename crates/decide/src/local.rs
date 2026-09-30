pub const DEFAULT_URL: &str = "http://127.0.0.1:8765/v1/systemone";
pub const CONNECT_HINT: &str = "jev-style serve가 실행 중인지 확인하세요";

pub fn url() -> String {
    std::env::var("DECIDE_LOCAL_URL")
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| DEFAULT_URL.to_string())
}
