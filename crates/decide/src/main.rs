use decide::backend::{live_transport, Env};
use decide::claude::{self, Installed};
use decide::daemon;
use decide::help;
use decide::mcp::handle_message;
use serde_json::Value;
use std::io::{self, BufRead, Read, Write};
use std::process::Command;

const DECIDE_BIN: &str = "/opt/homebrew/bin/decide";

fn main() {
    let argv: Vec<String> = std::env::args().skip(1).collect();
    if handle_help_and_version(&argv) {
        return;
    }
    let mut args = argv.into_iter();
    match args.next().as_deref() {
        Some("mcp") => {
            reject_extra(args.next());
            if let Err(err) = run_mcp() {
                eprintln!("{err}");
                std::process::exit(1);
            }
        }
        Some("daemon") => {
            reject_extra(args.next());
            if let Err(err) = daemon::serve_default() {
                eprintln!("{err}");
                std::process::exit(1);
            }
        }
        Some("install") => {
            let with_claude = match args.next().as_deref() {
                None => false,
                Some("--claude") => {
                    reject_extra(args.next());
                    true
                }
                Some(other) => {
                    eprintln!("알 수 없는 옵션입니다: {other}");
                    std::process::exit(2);
                }
            };
            let result = if with_claude {
                run_install_claude()
            } else {
                run_install()
            };
            if let Err(err) = result {
                eprintln!("{err}");
                std::process::exit(1);
            }
        }
        Some("hook") => {
            reject_extra(args.next());
            run_hook();
        }
        Some("gate") => run_gate(args.collect()),
        Some(other) => unknown_command(other),
        None => unreachable!("인자 없음은 도움말 처리에서 끝난다"),
    }
}

fn unknown_command(name: &str) -> ! {
    eprintln!("알 수 없는 명령입니다: {name}");
    eprintln!("사용 가능한 명령은 `decide --help`로 볼 수 있습니다.");
    std::process::exit(2);
}

/// 도움말과 버전을 처리했으면 true. 서버를 띄우는 명령(`mcp`, `daemon`)도 도움말 옵션이 있으면 시작하지 않는다.
/// - 인자 없음, `-h`, `--help`: 전체 도움말(종료 코드 0)
/// - `-V`, `--version`: 버전 한 줄(다른 인자가 있으면 종료 코드 2)
/// - `help [명령]`: 전체 또는 명령별 도움말
/// - `<명령> -h|--help`: 그 명령의 도움말(다른 인자보다 우선)
fn handle_help_and_version(argv: &[String]) -> bool {
    match argv.first().map(String::as_str) {
        None | Some("-h") | Some("--help") => println!("{}", help::overview()),
        Some("-V") | Some("--version") => {
            reject_extra(argv.get(1).cloned());
            println!("{}", help::version_line());
        }
        Some("help") => {
            reject_extra(argv.get(2).cloned());
            match argv.get(1) {
                None => println!("{}", help::overview()),
                Some(name) => match help::for_command(name) {
                    Some(text) => println!("{text}"),
                    None => unknown_command(name),
                },
            }
        }
        Some(command) if argv[1..].iter().any(|arg| arg == "-h" || arg == "--help") => {
            match help::for_command(command) {
                Some(text) => println!("{text}"),
                // 알 수 없는 명령이면 도움말이 아니라 분기의 오류로 넘긴다.
                None => return false,
            }
        }
        _ => return false,
    }
    true
}

fn run_hook() {
    let mut input = String::new();
    if io::stdin().read_to_string(&mut input).is_err() {
        return;
    }
    let Ok(value) = serde_json::from_str::<Value>(&input) else {
        return;
    };
    if let Some(text) = decide::show::render(&value) {
        println!("{}", serde_json::json!({ "systemMessage": text }));
    }
}

/// `decide gate <이름>`(stdin 훅 입력 → 훅 출력 JSON), `decide gate --show [이름] [--json]`,
/// `decide gate stats [--since 24h|7d|all] [--json]`. `stats`는 게이트 이름이 아니라 예약어다.
fn run_gate(args: Vec<String>) {
    match args.first().map(String::as_str) {
        Some("--show") => run_gate_show(&args[1..]),
        Some("stats") => run_gate_stats(&args[1..]),
        Some(name) if !name.starts_with('-') && args.len() == 1 => {
            if name != decide::gate::config::BASH_RISK {
                eprintln!("알 수 없는 게이트입니다: {name}");
                std::process::exit(1);
            }
            let mut input = String::new();
            if io::stdin().read_to_string(&mut input).is_err() {
                return;
            }
            let ctx = gate_context();
            let mut spawn = || {
                let _ = decide::gate::client::spawn_daemon();
            };
            if let Some(output) = decide::gate::run_hook(name, &input, &ctx, &mut spawn) {
                println!("{output}");
            }
        }
        _ => {
            eprintln!(
                "사용법: decide gate <이름> | decide gate --show [이름] [--json] | decide gate stats [--since 24h|7d|all] [--json]"
            );
            std::process::exit(2);
        }
    }
}

/// `decide gate stats [--since 24h|7d|all] [--json]`: 감사 로그를 집계해 보여 준다. 로그를 바꾸지 않는다.
fn run_gate_stats(args: &[String]) {
    use decide::gate::{client, config, stats};
    let usage = || -> ! {
        eprintln!("사용법: decide gate stats [--since 24h|7d|all] [--json]");
        std::process::exit(2);
    };
    let (mut since, mut json, mut since_given) = (stats::Since::All, false, false);
    let mut rest = args.iter();
    while let Some(arg) = rest.next() {
        match arg.as_str() {
            "--json" => json = true,
            "--since" if !since_given => {
                since_given = true;
                match rest.next().map(|value| stats::parse_since(value)) {
                    Some(Ok(parsed)) => since = parsed,
                    Some(Err(message)) => {
                        eprintln!("{message}");
                        usage()
                    }
                    None => usage(),
                }
            }
            _ => usage(),
        }
    }
    let ctx = gate_context();
    let path = client::audit_path(ctx.home.as_deref());
    let log = path.as_ref().and_then(|path| std::fs::read_to_string(path).ok());
    if log.is_none() && !json {
        let shown = path.map(|path| path.display().to_string()).unwrap_or_else(|| "~/.cache/decide/gate.log".into());
        println!("감사 로그가 아직 없습니다 ({shown}). 게이트가 판정을 하면 쌓입니다.");
        return;
    }
    let (records, skipped) = stats::parse_log(log.as_deref().unwrap_or(""));
    let loaded = config::load_from_disk(ctx.home.as_deref(), &ctx.fallback_cwd);
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs())
        .unwrap_or(0);
    let mut summary = stats::summarize(&records, now, since, loaded.config.bash_risk.confidence);
    summary.skipped_lines = skipped;
    if json {
        println!("{}", serde_json::to_string_pretty(&stats::to_json(&summary)).unwrap_or_default());
    } else {
        println!("{}", stats::render(&summary));
    }
}

fn gate_context() -> decide::gate::Context {
    decide::gate::Context {
        home: std::env::var("HOME").ok(),
        socket: daemon::default_socket_path(),
        client_version: daemon::VERSION.to_string(),
        fallback_cwd: std::env::current_dir().unwrap_or_else(|_| ".".into()),
    }
}

fn run_gate_show(args: &[String]) {
    use decide::gate::{config, output};
    let json = args.iter().any(|arg| arg == "--json");
    let names: Vec<&String> = args.iter().filter(|arg| !arg.starts_with("--")).collect();
    if args.iter().any(|arg| arg.starts_with("--") && arg != "--json") || names.len() > 1 {
        eprintln!("사용법: decide gate --show [이름] [--json]");
        std::process::exit(2);
    }
    let ctx = gate_context();
    let loaded = config::load_from_disk(ctx.home.as_deref(), &ctx.fallback_cwd);
    match names.first() {
        Some(name) => match output::show_gate(&loaded, name, json) {
            Some(text) => println!("{text}"),
            None => {
                eprintln!("알 수 없는 게이트입니다: {name}");
                std::process::exit(2);
            }
        },
        None if json => {
            eprintln!("--json은 게이트 이름과 함께 써야 합니다: decide gate --show bash-risk --json");
            std::process::exit(2);
        }
        None => {
            let (user_path, repo_path) = config::config_paths(ctx.home.as_deref(), &ctx.fallback_cwd);
            let user = output::Location {
                label: "~/.config/decide/gates.json".to_string(),
                exists: user_path.is_some_and(|path| path.exists()),
            };
            let repo = output::Location {
                label: "./.decide/gates.json".to_string(),
                exists: repo_path.exists(),
            };
            println!("{}", output::show_overview(&loaded, &user, &repo));
        }
    }
}

fn reject_extra(extra: Option<String>) {
    if extra.is_some() {
        eprintln!("인자가 너무 많습니다");
        std::process::exit(2);
    }
}

/// `claude mcp add`에 넘길 인자. HTTP transport로 고정 포트(daemon::DEFAULT_HTTP_ADDR)에 등록한다.
fn install_mcp_args() -> Vec<String> {
    vec![
        "mcp".to_string(),
        "add".to_string(),
        "-s".to_string(),
        "user".to_string(),
        "--transport".to_string(),
        "http".to_string(),
        "decide".to_string(),
        format!("http://{}/mcp", daemon::DEFAULT_HTTP_ADDR),
    ]
}

fn run_install() -> io::Result<()> {
    let output = Command::new("claude")
        .args(install_mcp_args())
        .output()
        .map_err(|err| {
            if err.kind() == io::ErrorKind::NotFound {
                io::Error::new(
                    io::ErrorKind::NotFound,
                    "claude 명령을 찾을 수 없습니다. Claude Code CLI가 설치돼 있는지 확인하세요",
                )
            } else {
                err
            }
        })?;
    io::stdout().write_all(&output.stdout)?;
    io::stderr().write_all(&output.stderr)?;
    if output.status.success() {
        return Ok(());
    }
    let already_registered = [&output.stdout[..], &output.stderr[..]]
        .iter()
        .any(|bytes| String::from_utf8_lossy(bytes).contains("already exists"));
    if already_registered {
        return Ok(());
    }
    Err(io::Error::other("claude mcp add 실행이 실패했습니다"))
}

/// http_addr가 이미 쓰이고 있지 않으면 spawn을 호출한다(실제로는 decide daemon을 띄움).
fn spawn_daemon_if_needed(http_addr: &str, spawn: &mut dyn FnMut()) {
    if !decide::http::http_port_in_use(http_addr) {
        spawn();
    }
}

fn run_install_claude() -> io::Result<()> {
    run_install()?;
    let path = claude::settings_path().map_err(io::Error::other)?;
    let display = claude::hook_command(DECIDE_BIN);
    let gate = claude::gate_command(DECIDE_BIN);
    match claude::install_hooks(&path, &claude::hook_specs(&display, &gate)).map_err(io::Error::other)? {
        Installed::Added => println!("훅을 등록했습니다(결과 표시, bash-risk 게이트): {}", path.display()),
        Installed::AlreadyPresent => println!("훅이 이미 등록돼 있습니다: {}", path.display()),
    }
    spawn_daemon_if_needed(daemon::DEFAULT_HTTP_ADDR, &mut || {
        if decide::gate::client::spawn_daemon().is_ok() {
            println!("decide daemon을 띄웠습니다 (http://{}/mcp)", daemon::DEFAULT_HTTP_ADDR);
        }
    });
    Ok(())
}

fn run_mcp() -> io::Result<()> {
    let env = Env::from_process();
    let mut transport = live_transport(&env);
    let stdin = io::stdin();
    let mut stdout = io::stdout();
    for line in stdin.lock().lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let message: Value = match serde_json::from_str(&line) {
            Ok(message) => message,
            Err(err) => {
                let error = serde_json::json!({
                    "jsonrpc": "2.0",
                    "id": null,
                    "error": {"code": -32700, "message": err.to_string()},
                });
                writeln!(stdout, "{error}")?;
                stdout.flush()?;
                continue;
            }
        };
        if let Some(response) = handle_message(&message, &env, &mut transport) {
            writeln!(stdout, "{response}")?;
            stdout.flush()?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn install_mcp_args_register_http_transport_at_the_fixed_port() {
        let args = install_mcp_args();
        assert_eq!(
            args,
            vec![
                "mcp", "add", "-s", "user", "--transport", "http", "decide",
                "http://127.0.0.1:48080/mcp",
            ]
        );
    }

    #[test]
    fn spawn_daemon_if_needed_skips_when_the_port_is_already_bound() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = format!("127.0.0.1:{}", listener.local_addr().unwrap().port());
        let mut spawned = 0;
        spawn_daemon_if_needed(&addr, &mut || spawned += 1);
        assert_eq!(spawned, 0, "포트가 이미 쓰이고 있으면 다시 띄우지 않는다");
    }

    #[test]
    fn spawn_daemon_if_needed_spawns_when_the_port_is_free() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = format!("127.0.0.1:{}", listener.local_addr().unwrap().port());
        drop(listener); // 포트를 비운다.
        let freed = (0..20).any(|_| {
            let ok = !decide::http::http_port_in_use(&addr);
            if !ok {
                std::thread::sleep(std::time::Duration::from_millis(20));
            }
            ok
        });
        assert!(freed, "포트가 해제되지 않았다");
        let mut spawned = 0;
        spawn_daemon_if_needed(&addr, &mut || spawned += 1);
        assert_eq!(spawned, 1);
    }
}
