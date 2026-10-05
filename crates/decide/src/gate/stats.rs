// 감사 로그(gate.log)를 집계해 판정 방식, 판정 분포, 지연, 시간 초과, 규칙 적중 같은 사용 통계를 만든다
use serde_json::{json, Value};
use std::collections::BTreeMap;

/// 로그 한 줄에서 통계에 필요한 필드만 뽑은 것. 명령 문자열은 통계에 쓰지 않아 담지 않는다.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Record {
    pub ts: u64,
    pub verdict: Option<String>,
    /// `probs` 중 가장 큰 확률.
    pub max_prob: Option<f64>,
    pub backend: Option<String>,
    pub latency_ms: Option<f64>,
    pub prefiltered: bool,
    pub rule: Option<String>,
    pub failure: Option<String>,
}

/// 집계할 기간.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Since {
    All,
    Hours(u64),
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Latency {
    pub n: usize,
    pub mean: f64,
    pub median: f64,
    pub p90: f64,
    pub p95: f64,
    pub max: f64,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Stats {
    pub from: Option<u64>,
    pub to: Option<u64>,
    pub total: usize,
    pub rule: usize,
    pub prefiltered: usize,
    pub judged: usize,
    pub failed: usize,
    /// 모델과 규칙이 낸 판정의 분포.
    pub allow: usize,
    pub ask: usize,
    pub deny: usize,
    /// 낮은 확신을 가르는 기준(최고 확률이 이 값 미만)과, 그래서 `ask`가 된 모델 판정 수.
    pub confidence: f64,
    pub low_confidence: usize,
    /// 모델이 필요했던 호출(모델 판정 + 실패·통과) 수와 그중 시간 초과 수.
    pub model_needed: usize,
    pub timeouts: usize,
    pub failures: Vec<(String, usize)>,
    pub latency: Vec<(String, Latency)>,
    pub top_rules: Vec<(String, usize)>,
    pub skipped_lines: usize,
}

/// `24h`, `7d`, `all`을 읽는다. 0이나 다른 단위는 오류다.
pub fn parse_since(text: &str) -> Result<Since, String> {
    if text == "all" {
        return Ok(Since::All);
    }
    let error = || format!("기간은 24h, 7d, all 처럼 써야 합니다: {text:?}");
    let (digits, unit) = text.split_at(text.len().saturating_sub(1));
    let amount: u64 = digits.parse().ok().filter(|amount| *amount > 0).ok_or_else(error)?;
    match unit {
        "h" => Ok(Since::Hours(amount)),
        "d" => Ok(Since::Hours(amount * 24)),
        _ => Err(error()),
    }
}

/// 로그 텍스트를 읽는다. 읽을 수 없는 줄(깨진 JSON, 숫자가 아닌 `ts`)은 건너뛰고 그 수를 함께 돌려준다. 빈 줄은 세지 않는다.
pub fn parse_log(text: &str) -> (Vec<Record>, usize) {
    let mut records = Vec::new();
    let mut skipped = 0;
    for line in text.lines().filter(|line| !line.trim().is_empty()) {
        match serde_json::from_str::<Value>(line).ok().and_then(|value| record_from(&value)) {
            Some(record) => records.push(record),
            None => skipped += 1,
        }
    }
    (records, skipped)
}

fn record_from(value: &Value) -> Option<Record> {
    let text = |key: &str| value.get(key).and_then(Value::as_str).map(str::to_string);
    let max_prob = value.get("probs").and_then(Value::as_object).and_then(|probs| {
        probs.values().filter_map(Value::as_f64).fold(None, |best: Option<f64>, p| Some(best.map_or(p, |b| b.max(p))))
    });
    Some(Record {
        ts: value.get("ts")?.as_u64()?,
        verdict: text("verdict"),
        max_prob,
        backend: text("backend"),
        latency_ms: value.get("latency_ms").and_then(Value::as_f64),
        prefiltered: value.get("prefiltered").and_then(Value::as_bool).unwrap_or(false),
        rule: text("rule"),
        failure: text("failure"),
    })
}

/// 기록을 집계한다. `now`(초)와 `since`로 기간을 거르고, `confidence`로 낮은 확신 건수를 센다.
pub fn summarize(records: &[Record], now: u64, since: Since, confidence: f64) -> Stats {
    let cutoff = match since {
        Since::All => 0,
        Since::Hours(hours) => now.saturating_sub(hours * 3600),
    };
    let rows: Vec<&Record> = records.iter().filter(|record| record.ts >= cutoff).collect();
    let mut stats = Stats {
        from: rows.iter().map(|record| record.ts).min(),
        to: rows.iter().map(|record| record.ts).max(),
        total: rows.len(),
        confidence,
        ..Stats::default()
    };
    let mut failures: BTreeMap<String, usize> = BTreeMap::new();
    let mut rules: BTreeMap<String, usize> = BTreeMap::new();
    let mut latencies: BTreeMap<String, Vec<f64>> = BTreeMap::new();
    for record in rows {
        if let Some(pattern) = &record.rule {
            stats.rule += 1;
            *rules.entry(pattern.clone()).or_default() += 1;
            stats.count_verdict(record.verdict.as_deref());
        } else if record.prefiltered {
            stats.prefiltered += 1;
        } else if let Some(reason) = &record.failure {
            stats.failed += 1;
            if reason.contains("안에 답하지 않았습니다") {
                stats.timeouts += 1;
            }
            *failures.entry(reason.clone()).or_default() += 1;
        } else if record.verdict.is_some() {
            stats.judged += 1;
            stats.count_verdict(record.verdict.as_deref());
            if record.verdict.as_deref() == Some("ask") && record.max_prob.is_some_and(|p| p < confidence) {
                stats.low_confidence += 1;
            }
            if let (Some(backend), Some(latency)) = (&record.backend, record.latency_ms) {
                latencies.entry(backend.clone()).or_default().push(latency);
            }
        }
    }
    stats.model_needed = stats.judged + stats.failed;
    stats.failures = ranked(failures);
    stats.top_rules = ranked(rules).into_iter().take(5).collect();
    stats.latency = latencies.into_iter().map(|(backend, values)| (backend, latency_of(values))).collect();
    stats
}

impl Stats {
    fn count_verdict(&mut self, verdict: Option<&str>) {
        match verdict {
            Some("allow") => self.allow += 1,
            Some("ask") => self.ask += 1,
            Some("deny") => self.deny += 1,
            _ => {}
        }
    }
}

/// 건수가 많은 순, 같으면 이름순으로 정렬한다.
fn ranked(counts: BTreeMap<String, usize>) -> Vec<(String, usize)> {
    let mut items: Vec<(String, usize)> = counts.into_iter().collect();
    items.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    items
}

fn latency_of(mut values: Vec<f64>) -> Latency {
    values.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let n = values.len();
    let nearest = |fraction: f64| values[((fraction * (n as f64 - 1.0)).round() as usize).min(n - 1)];
    let median = if n % 2 == 1 { values[n / 2] } else { (values[n / 2 - 1] + values[n / 2]) / 2.0 };
    Latency {
        n,
        mean: values.iter().sum::<f64>() / n as f64,
        median,
        p90: nearest(0.90),
        p95: nearest(0.95),
        max: values[n - 1],
    }
}

fn percent(part: usize, whole: usize) -> f64 {
    if whole == 0 {
        0.0
    } else {
        part as f64 * 100.0 / whole as f64
    }
}

/// 사람이 읽는 텍스트.
pub fn render(stats: &Stats) -> String {
    let (Some(from), Some(to)) = (stats.from, stats.to) else {
        return "게이트 사용 기록이 없습니다(감사 로그가 비었거나 이 기간에 기록이 없습니다).".to_string();
    };
    let mut lines = vec![
        format!("게이트 사용 통계 (기간 {} ~ {} UTC, 총 {}건)", format_utc(from), format_utc(to), stats.total),
        "판정 방식".to_string(),
        format!("  정적 규칙   {}건", stats.rule),
        format!("  사전 필터   {}건", stats.prefiltered),
        format!("  모델 판정   {}건", stats.judged),
        format!("  실패·통과   {}건", stats.failed),
    ];
    let decided = stats.allow + stats.ask + stats.deny;
    let share = |label: &str, count: usize| format!("{label} {count}건 ({:.0}%)", percent(count, decided));
    lines.push(format!("판정 분포 (모델·규칙이 낸 판정 {decided}건)"));
    lines.push(format!(
        "  {} · {} · {}",
        share("allow", stats.allow),
        share("ask", stats.ask),
        share("deny", stats.deny)
    ));
    lines.push(format!(
        "  낮은 확신(최고 확률 {:.2} 미만)으로 ask가 된 모델 판정: {}건",
        stats.confidence, stats.low_confidence
    ));
    if !stats.latency.is_empty() {
        lines.push("지연 (모델 판정, 백엔드 구간만 — 훅 프로세스 시작과 소켓 통신은 제외)".to_string());
        for (backend, latency) in &stats.latency {
            lines.push(format!(
                "  {backend}: n={} 평균 {:.0}ms 중앙값 {:.0}ms p90 {:.0}ms p95 {:.0}ms 최대 {:.0}ms",
                latency.n, latency.mean, latency.median, latency.p90, latency.p95, latency.max
            ));
        }
    }
    if stats.model_needed > 0 {
        lines.push(format!(
            "모델이 필요했던 호출 중 시간 초과 {}/{}건 ({:.1}%)",
            stats.timeouts,
            stats.model_needed,
            percent(stats.timeouts, stats.model_needed)
        ));
    }
    if !stats.failures.is_empty() {
        lines.push("실패·통과 사유".to_string());
        lines.extend(stats.failures.iter().map(|(reason, count)| format!("  {count}건 {reason}")));
    }
    if !stats.top_rules.is_empty() {
        lines.push("규칙 적중 상위".to_string());
        lines.extend(stats.top_rules.iter().map(|(pattern, count)| format!("  {count}건 {pattern}")));
    }
    if stats.skipped_lines > 0 {
        lines.push(format!("읽지 못해 건너뛴 로그 줄: {}줄", stats.skipped_lines));
    }
    lines.join("\n")
}

/// `--json` 출력.
pub fn to_json(stats: &Stats) -> Value {
    let latency: serde_json::Map<String, Value> = stats
        .latency
        .iter()
        .map(|(backend, l)| {
            (
                backend.clone(),
                json!({"n": l.n, "mean_ms": l.mean, "median_ms": l.median, "p90_ms": l.p90, "p95_ms": l.p95,
                    "max_ms": l.max}),
            )
        })
        .collect();
    json!({
        "from": stats.from,
        "to": stats.to,
        "total": stats.total,
        "kinds": {"rule": stats.rule, "prefiltered": stats.prefiltered, "judged": stats.judged, "failed": stats.failed},
        "verdicts": {"allow": stats.allow, "ask": stats.ask, "deny": stats.deny},
        "confidence": stats.confidence,
        "low_confidence": stats.low_confidence,
        "model_needed": stats.model_needed,
        "timeouts": stats.timeouts,
        "failures": stats.failures.iter().map(|(reason, count)| json!({"reason": reason, "count": count})).collect::<Vec<_>>(),
        "latency": latency,
        "top_rules": stats.top_rules.iter().map(|(pattern, count)| json!({"pattern": pattern, "count": count})).collect::<Vec<_>>(),
        "skipped_lines": stats.skipped_lines,
    })
}

/// 에포크 초를 `YYYY-MM-DD HH:MM:SS`(UTC)로 바꾼다.
pub fn format_utc(ts: u64) -> String {
    let (days, secs) = ((ts / 86_400) as i64, ts % 86_400);
    // 날짜 계산은 Howard Hinnant의 civil_from_days 알고리즘이다.
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let day_of_era = z.rem_euclid(146_097);
    let year_of_era = (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_index = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_index + 2) / 5 + 1;
    let month = if month_index < 10 { month_index + 3 } else { month_index - 9 };
    let year = year_of_era + era * 400 + i64::from(month <= 2);
    format!("{year:04}-{month:02}-{day:02} {:02}:{:02}:{:02}", secs / 3600, secs % 3600 / 60, secs % 60)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const NOW: u64 = 1_800_000_000;

    fn line(value: Value) -> String {
        value.to_string()
    }

    fn judged(ts: u64, backend: &str, latency: f64, verdict: &str, probs: [f64; 3]) -> Value {
        json!({"ts": ts, "gate": "bash-risk", "mode": "audit", "verdict": verdict,
            "probs": {"allow": probs[0], "ask": probs[1], "deny": probs[2]},
            "backend": backend, "model": "m", "latency_ms": latency, "prefiltered": false,
            "failure": null, "command": "x", "cwd_tail": "a/b"})
    }

    fn failed(ts: u64, reason: &str) -> Value {
        json!({"ts": ts, "gate": "bash-risk", "mode": "audit", "verdict": null, "probs": null, "backend": null,
            "model": null, "latency_ms": null, "prefiltered": false, "failure": reason, "command": "x"})
    }

    fn prefiltered(ts: u64) -> Value {
        json!({"ts": ts, "gate": "bash-risk", "mode": "audit", "verdict": null, "probs": null, "backend": null,
            "model": null, "latency_ms": null, "prefiltered": true, "failure": null, "command": "ls"})
    }

    fn ruled(ts: u64, verdict: &str, pattern: &str) -> Value {
        json!({"ts": ts, "gate": "bash-risk", "mode": "audit", "verdict": verdict, "probs": null, "backend": null,
            "model": null, "latency_ms": null, "prefiltered": false, "rule": pattern, "failure": null,
            "command": "x"})
    }

    /// 지연이 있는 로컬 판정 2건, TypeSafe 1건, 사전 필터 1건, 규칙 2건(같은 패턴), 시간 초과 1건, 데몬 시작 1건.
    fn sample() -> Vec<Record> {
        let text = [
            line(judged(NOW - 100, "local", 900.0, "allow", [0.9, 0.05, 0.05])),
            line(judged(NOW - 90, "local", 1100.0, "ask", [0.4, 0.35, 0.25])),
            line(judged(NOW - 80, "typesafe", 200.0, "deny", [0.1, 0.1, 0.8])),
            line(prefiltered(NOW - 70)),
            line(ruled(NOW - 60, "deny", "mkfs*")),
            line(ruled(NOW - 50, "ask", "mkfs*")),
            line(failed(NOW - 40, "데몬이 2000ms 안에 답하지 않았습니다")),
            line(failed(NOW - 30, "데몬이 꺼져 있어 새로 시작합니다")),
        ]
        .join("\n");
        let (records, skipped) = parse_log(&text);
        assert_eq!(skipped, 0);
        records
    }

    #[test]
    fn parse_log_reads_fields_and_skips_broken_lines() {
        let text = format!(
            "{}\nnot json\n\n{}\n{{\"ts\": \"oops\"}}\n",
            line(judged(100, "local", 950.5, "ask", [0.2, 0.5, 0.3])),
            line(ruled(200, "deny", "mkfs*")),
        );
        let (records, skipped) = parse_log(&text);
        assert_eq!(records.len(), 2);
        assert_eq!(skipped, 2, "깨진 줄과 ts가 숫자가 아닌 줄은 건너뛴다(빈 줄은 세지 않는다)");
        assert_eq!(records[0].ts, 100);
        assert_eq!(records[0].verdict.as_deref(), Some("ask"));
        assert_eq!(records[0].max_prob, Some(0.5));
        assert_eq!(records[0].backend.as_deref(), Some("local"));
        assert_eq!(records[0].latency_ms, Some(950.5));
        assert!(!records[0].prefiltered);
        assert_eq!(records[1].rule.as_deref(), Some("mkfs*"));
        assert_eq!(records[1].max_prob, None);
    }

    #[test]
    fn older_log_lines_without_a_rule_field_still_parse() {
        let old = r#"{"ts": 5, "gate": "bash-risk", "mode": "audit", "verdict": "allow",
            "probs": {"allow": 0.9, "ask": 0.1, "deny": 0.0}, "backend": "local", "model": "m",
            "latency_ms": 700.0, "prefiltered": false, "failure": null, "command": "x"}"#
            .replace('\n', " ");
        let (records, skipped) = parse_log(&old);
        assert_eq!((records.len(), skipped), (1, 0));
        assert_eq!(records[0].rule, None);
    }

    #[test]
    fn parse_since_accepts_hours_days_and_all() {
        assert_eq!(parse_since("all"), Ok(Since::All));
        assert_eq!(parse_since("24h"), Ok(Since::Hours(24)));
        assert_eq!(parse_since("7d"), Ok(Since::Hours(168)));
        assert_eq!(parse_since("1h"), Ok(Since::Hours(1)));
        for bad in ["", "0h", "0d", "10m", "h", "-3h", "24", "1.5d", "ALL "] {
            assert!(parse_since(bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn summarize_classifies_each_record_by_how_it_was_decided() {
        let stats = summarize(&sample(), NOW, Since::All, 0.7);
        assert_eq!(stats.total, 8);
        assert_eq!(stats.rule, 2);
        assert_eq!(stats.prefiltered, 1);
        assert_eq!(stats.judged, 3);
        assert_eq!(stats.failed, 2);
        assert_eq!((stats.from, stats.to), (Some(NOW - 100), Some(NOW - 30)));
    }

    #[test]
    fn summarize_counts_verdicts_over_model_and_rule_decisions() {
        let stats = summarize(&sample(), NOW, Since::All, 0.7);
        // 모델: allow 1, ask 1, deny 1 / 규칙: deny 1, ask 1
        assert_eq!((stats.allow, stats.ask, stats.deny), (1, 2, 2));
    }

    #[test]
    fn summarize_measures_latency_per_backend_from_model_decisions_only() {
        let stats = summarize(&sample(), NOW, Since::All, 0.7);
        let local = &stats.latency.iter().find(|(name, _)| name == "local").unwrap().1;
        assert_eq!(local.n, 2);
        assert_eq!(local.mean, 1000.0);
        assert_eq!(local.median, 1000.0, "짝수 개는 가운데 두 값의 평균이다");
        assert_eq!(local.max, 1100.0);
        let typesafe = &stats.latency.iter().find(|(name, _)| name == "typesafe").unwrap().1;
        assert_eq!((typesafe.n, typesafe.mean, typesafe.p95, typesafe.max), (1, 200.0, 200.0, 200.0));
        assert_eq!(stats.latency.len(), 2, "규칙·사전 필터·실패는 지연에 들어가지 않는다");
    }

    #[test]
    fn percentiles_use_the_nearest_rank_of_the_sorted_values() {
        let mut lines = Vec::new();
        for (index, latency) in (1..=10).map(|n| n as f64 * 100.0).enumerate() {
            lines.push(line(judged(NOW - index as u64, "local", latency, "allow", [0.9, 0.05, 0.05])));
        }
        let (records, _) = parse_log(&lines.join("\n"));
        let stats = summarize(&records, NOW, Since::All, 0.7);
        let local = &stats.latency[0].1;
        assert_eq!(local.n, 10);
        assert_eq!(local.median, 550.0);
        assert_eq!(local.p90, 900.0, "(10-1)*0.9=8.1 → 반올림 8번째(0부터) = 900");
        assert_eq!(local.p95, 1000.0, "(10-1)*0.95=8.55 → 반올림 9번째 = 1000");
        assert_eq!(local.max, 1000.0);
    }

    #[test]
    fn summarize_counts_timeouts_against_the_calls_that_needed_the_model() {
        let stats = summarize(&sample(), NOW, Since::All, 0.7);
        assert_eq!(stats.model_needed, 5, "모델 판정 3건 + 실패·통과 2건");
        assert_eq!(stats.timeouts, 1);
        let reasons: Vec<&str> = stats.failures.iter().map(|(reason, _)| reason.as_str()).collect();
        assert!(reasons.contains(&"데몬이 2000ms 안에 답하지 않았습니다"));
        assert!(reasons.contains(&"데몬이 꺼져 있어 새로 시작합니다"));
        assert!(stats.failures.iter().all(|(_, count)| *count == 1));
    }

    #[test]
    fn low_confidence_counts_model_asks_whose_best_probability_is_below_the_threshold() {
        let stats = summarize(&sample(), NOW, Since::All, 0.7);
        assert_eq!(stats.low_confidence, 1, "ask 판정 0.4/0.35/0.25는 최고 확률 0.4가 0.7 미만이다");
        let stricter = summarize(&sample(), NOW, Since::All, 0.3);
        assert_eq!(stricter.low_confidence, 0, "임계값이 0.3이면 0.4는 낮지 않다");
    }

    #[test]
    fn top_rules_are_the_most_hit_patterns() {
        let stats = summarize(&sample(), NOW, Since::All, 0.7);
        assert_eq!(stats.top_rules, vec![("mkfs*".to_string(), 2)]);
    }

    #[test]
    fn the_period_filter_keeps_only_recent_records() {
        let stats = summarize(&sample(), NOW, Since::Hours(1), 0.7);
        assert_eq!(stats.total, 8, "모두 최근 1시간 안이다");
        let old = line(judged(NOW - 2 * 3600, "local", 800.0, "allow", [0.9, 0.05, 0.05]));
        let (mut records, _) = parse_log(&old);
        records.extend(sample());
        assert_eq!(summarize(&records, NOW, Since::Hours(1), 0.7).total, 8, "2시간 전 기록은 뺀다");
        assert_eq!(summarize(&records, NOW, Since::Hours(3), 0.7).total, 9);
        assert_eq!(summarize(&records, NOW, Since::All, 0.7).total, 9);
    }

    #[test]
    fn an_empty_log_summarizes_to_zeros_and_renders_a_notice() {
        let stats = summarize(&[], NOW, Since::All, 0.7);
        assert_eq!(stats.total, 0);
        assert_eq!((stats.from, stats.to), (None, None));
        assert!(render(&stats).contains("기록이 없습니다"), "{}", render(&stats));
        assert_eq!(to_json(&stats)["total"], 0);
    }

    #[test]
    fn render_shows_counts_percentages_latency_and_timeouts() {
        let text = render(&summarize(&sample(), NOW, Since::All, 0.7));
        for needle in [
            "총 8건",
            "정적 규칙",
            "사전 필터",
            "모델 판정",
            "실패·통과",
            "allow 1건 (20%)",
            "ask 2건 (40%)",
            "deny 2건 (40%)",
            "낮은 확신",
            "local",
            "평균 1000ms",
            "중앙값 1000ms",
            "시간 초과 1/5건 (20.0%)",
            "백엔드 구간만",
            "mkfs*",
        ] {
            assert!(text.contains(needle), "{needle:?}가 없다:\n{text}");
        }
    }

    #[test]
    fn to_json_has_the_same_numbers_as_the_text() {
        let value = to_json(&summarize(&sample(), NOW, Since::All, 0.7));
        assert_eq!(value["total"], 8);
        assert_eq!(value["kinds"], serde_json::json!({"rule": 2, "prefiltered": 1, "judged": 3, "failed": 2}));
        assert_eq!(value["verdicts"], serde_json::json!({"allow": 1, "ask": 2, "deny": 2}));
        assert_eq!(value["timeouts"], 1);
        assert_eq!(value["model_needed"], 5);
        assert_eq!(value["low_confidence"], 1);
        assert_eq!(value["latency"]["local"]["n"], 2);
        assert_eq!(value["latency"]["local"]["mean_ms"], 1000.0);
        assert_eq!(value["top_rules"][0]["pattern"], "mkfs*");
        assert_eq!(value["top_rules"][0]["count"], 2);
        assert_eq!(value["skipped_lines"], 0);
    }

    #[test]
    fn format_utc_converts_epoch_seconds() {
        assert_eq!(format_utc(0), "1970-01-01 00:00:00");
        assert_eq!(format_utc(1_791_118_483), "2026-10-04 12:54:43");
        assert_eq!(format_utc(951_782_400), "2000-02-29 00:00:00", "윤일");
    }
}
