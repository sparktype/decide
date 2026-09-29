use decide::backend::{nonempty, Env};
use decide::daemon;
use decide::mcp::handle_message;
use decide::typesafe::LiveTransport;
use serde_json::Value;
use std::io::{self, BufRead, Write};

fn main() {
    let mut args = std::env::args().skip(1);
    match args.next().as_deref() {
        Some("-h") | Some("--help") => print_help(),
        Some("mcp") | None => {
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
        Some(other) => {
            eprintln!("알 수 없는 명령입니다: {other}");
            std::process::exit(2);
        }
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
decide [mcp|daemon]

인자 없이 실행하면 stdio MCP 서버다.
  mcp      stdio MCP
  daemon   ~/.cache/decide/decide.sock

DECIDE_BACKEND=typesafe|local
TYPESAFE_API_KEY가 있고 백엔드를 지정하지 않으면 TypeSafe Jev를 호출한다."
    );
}

fn run_mcp() -> io::Result<()> {
    let env = Env::from_process();
    let key = nonempty(env.api_key.as_deref()).unwrap_or("");
    let mut transport = LiveTransport::new(key);
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
