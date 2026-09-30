# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## What this is

`decide` is an MCP tool for one non-generative judgment: `choice`, `score`, or `noul`.
The installed program is the Rust binary `/opt/homebrew/bin/decide`. One process has two
backends. `DECIDE_BACKEND=typesafe|local` selects one. When that variable is unset, a
non-empty `TYPESAFE_API_KEY` selects TypeSafe Jev and an absent key selects local.
A failed call stays on its backend. The two backends are not calibrated to each
other. Both answer `noul` with only `type` and `noul`.

The local backend does not run a model in this process. It POSTs the same body as TypeSafe
to `jev-style serve` (`DECIDE_LOCAL_URL`, default `http://127.0.0.1:8765/v1/systemone`, no
auth) and reads `answers.q`. The model is Jev-Style-2B-Decision-v3-MLX 8bit. That server is
a separate Python install, so only the local backend needs Python. Design:
`docs/superpowers/specs/2026-09-30-decide-local-jev-style-design.md`, which replaces the
Laya ONNX/Candle runtime section of the 2026-09-29 design.

The Python package in `src/decide/` and its `pytest` suite still exist. Delete them only in
the change that finishes the execution verification listed in the 2026-09-30 design
(`cargo test`, one TypeSafe call, one local call, one daemon socket call).

## Commands

Install from the tap on arm64 macOS:

```bash
brew install sparktype/tap/decide
```

The published formula is `Formula/decide.rb` in `sparktype/homebrew-tap`. This repo's
`packaging/homebrew/decide.rb` records the same install. Version is 0.0.3. GitHub
Actions builds `crates/decide` and uploads a release asset when a `v*` tag is pushed;
the formula downloads that prebuilt arm64 binary and installs it, no Rust toolchain
required at install time. The API key stays in the environment as `TYPESAFE_API_KEY`.
`.mcp.json` points `decide` at `/opt/homebrew/bin/decide` with `args: ["mcp"]` and no
`env` entry. Running `decide install` registers the tool in Claude Code's user scope
by shelling out to `claude mcp add -s user decide -- /opt/homebrew/bin/decide mcp`,
as an alternative to editing `.mcp.json` by hand.

Runtime tests, no weights and no network:

```bash
cargo test --manifest-path crates/decide/Cargo.toml
```

One Rust test:

```bash
cargo test --manifest-path crates/decide/Cargo.toml protocol::tests::noul_rejects_options_and_criteria
```

Python package tests (to be deleted with the execution verification above):

```bash
.venv/bin/python -m pytest
.venv/bin/python -m pytest tests/test_decide_impl.py::test_noul_builds_question_without_criteria -v
.venv/bin/python test_smoke.py
```

Daemon socket `~/.cache/decide/decide.sock`, 30 minutes idle:

```bash
/opt/homebrew/bin/decide daemon
```

`decide mcp` is the stdio MCP server. Running `decide` with no arguments or `--help`
prints the help text and exits without starting anything. After editing `.mcp.json`
or `.claude/settings.json`, restart Claude Code.

## Architecture

`crates/decide` is the runtime.

- `protocol.rs` validates `state`, `type`, `instructions`, `options`, and `criteria`.
  Keep the Korean strings for too few choice options, too few score levels, and noul
  receiving options or criteria. The question id sent to either backend is `"q"`.
- `backend.rs` selects the backend and measures `latency_ms` around that call only.
- `typesafe.rs` posts to `https://api.typesafe.ai/v1/systemone` with model
  `jev-latest`. Choice criteria are `{option: option}` in insertion order. Retry 429
  and 529 once after one second. Choice above 255 options and score above 10 levels
  fail before the request.
- `local.rs` holds the default local URL (`DEFAULT_URL`), `url()` (reads
  `DECIDE_LOCAL_URL`) and the connect hint. `backend::live_transport` picks the
  `LiveTransport` once per process: `typesafe(key)` or `local(url)` (no auth header).
  `execute` and `map_response` take a label (`TypeSafe` or `로컬`) for error text.
  The 255-option check runs only for TypeSafe; the local server enforces its own
  255 cap with a 422 that is passed through.
- `mcp.rs` speaks newline-delimited JSON-RPC. Tool failures are `isError` results.
- `daemon.rs` serves one JSON line per connection. A live socket is left in place. A
  dead socket file is replaced. Idle exit uses `poll`. It also holds an in-process
  LRU (`MAX_CACHE_ENTRIES` = 64, keyed on the parsed request plus the resolved
  backend) so identical requests within one daemon lifetime skip the transport;
  cached answers carry `routing.cached: true` and `latency_ms: 0.0`. `decide mcp`
  is a fresh process per call and has no cache.
- `main.rs` routes `mcp`, `daemon`, and `install` subcommands. No arguments prints
  help. `install` shells out to `claude mcp add -s user decide -- <bin> mcp`.

`.claude/hooks/stop_verify.py` remains a stdlib client. It spawns
`/opt/homebrew/bin/decide daemon` with the hook environment and fail-opens when the
socket is down. Its noul confidence threshold (default 0.4) reads from
`DECIDE_STOP_THRESHOLD`, falling back to 0.4 on a missing or unparsable value. This
one file is tracked in git despite `.claude/` being gitignored — see the exception
rule in `.gitignore`.

**`noul` is a probability, 0.0–1.0.** Apply any threshold at the call site. Phrase
`instructions` as a direct question. A rhetorical negation flips the direction
unreliably. See `.claude/skills/decide/SKILL.md`.

**Tests inject the backend.** Rust tests use a scripted transport and a fake clock.
Python tests still inject `predict_fn`. Keep real weight checks in `test_smoke.py`
and out of the default suites.
