use decide::backend::{live_transport, Env};
use decide::claude::{self, Installed};
use decide::daemon;
use decide::mcp::handle_message;
use serde_json::Value;
use std::io::{self, BufRead, Read, Write};
use std::process::Command;

const DECIDE_BIN: &str = "/opt/homebrew/bin/decide";

fn main() {
    let mut args = std::env::args().skip(1);
    match args.next().as_deref() {
        Some("-h") | Some("--help") | None => print_help(),
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
        Some(other) => {
            eprintln!("알 수 없는 명령입니다: {other}");
            std::process::exit(2);
        }
    }
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

fn reject_extra(extra: Option<String>) {
    if extra.is_some() {
        eprintln!("인자가 너무 많습니다");
        std::process::exit(2);
    }
}

fn print_help() {
    println!(
        "\
decide [mcp|daemon|install [--claude]|hook]

인자 없이 실행하면 이 도움말이다.
  mcp      stdio MCP
  daemon   ~/.cache/decide/decide.sock
  install  claude mcp add로 Claude Code 사용자 스코프에 decide를 등록한다
           --claude  MCP 등록에 더해 표시 훅(PostToolUse)도 Claude Code 사용자 설정에 넣는다
  hook     Claude Code PostToolUse 훅. decide 결과를 사용자에게 한 줄로 보여준다

DECIDE_BACKEND=typesafe|local
TYPESAFE_API_KEY가 있고 백엔드를 지정하지 않으면 TypeSafe Jev를 호출한다."
    );
}

fn run_install() -> io::Result<()> {
    let output = Command::new("claude")
        .args([
            "mcp", "add", "-s", "user", "decide", "--", DECIDE_BIN, "mcp",
        ])
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

fn run_install_claude() -> io::Result<()> {
    run_install()?;
    let path = claude::settings_path().map_err(io::Error::other)?;
    let command = claude::hook_command(DECIDE_BIN);
    match claude::install_hook(&path, &command).map_err(io::Error::other)? {
        Installed::Added => println!("훅을 등록했습니다: {}", path.display()),
        Installed::AlreadyPresent => println!("훅이 이미 등록돼 있습니다: {}", path.display()),
    }
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
