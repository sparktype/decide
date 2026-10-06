// DECIDE_LOCAL_TIMING=1일 때 로컬 추론 한 번의 단계별 소요 시간을 stderr에 한 줄로 찍는 계측
use std::cell::RefCell;
use std::sync::OnceLock;
use std::time::Instant;

struct Laps {
    last: Instant,
    items: Vec<(&'static str, f64)>,
}

thread_local! {
    static LAPS: RefCell<Option<Laps>> = const { RefCell::new(None) };
}

fn enabled() -> bool {
    static ON: OnceLock<bool> = OnceLock::new();
    *ON.get_or_init(|| std::env::var("DECIDE_LOCAL_TIMING").is_ok_and(|v| !v.trim().is_empty() && v != "0"))
}

/// 추론 한 번의 시작. 꺼져 있으면 아무것도 하지 않는다.
pub fn start() {
    if enabled() {
        LAPS.with(|laps| *laps.borrow_mut() = Some(Laps { last: Instant::now(), items: Vec::new() }));
    }
}

/// 직전 lap 이후 지난 시간을 `name`으로 기록한다.
pub fn lap(name: &'static str) {
    LAPS.with(|laps| {
        if let Some(laps) = laps.borrow_mut().as_mut() {
            let now = Instant::now();
            laps.items.push((name, now.duration_since(laps.last).as_secs_f64() * 1000.0));
            laps.last = now;
        }
    });
}

/// 기록한 lap을 한 줄로 출력하고 비운다.
pub fn finish(tokens: usize) {
    LAPS.with(|laps| {
        if let Some(laps) = laps.borrow_mut().take() {
            let total: f64 = laps.items.iter().map(|(_, ms)| ms).sum();
            let parts: Vec<String> = laps.items.iter().map(|(name, ms)| format!("{name}={ms:.1}ms")).collect();
            eprintln!("[decide-timing] tokens={tokens} {} total={total:.1}ms", parts.join(" "));
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lap_and_finish_are_noops_when_not_started() {
        lap("x");
        finish(0);
        LAPS.with(|laps| assert!(laps.borrow().is_none()));
    }
}
