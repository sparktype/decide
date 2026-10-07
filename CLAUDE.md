# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## What this is

`decide` is an MCP tool for one non-generative judgment: `choice`, `score`, or `noul`.
The installed program is the Rust binary `/opt/homebrew/bin/decide`. One process has two
backends and neither runs a model: both POST to a System One server (`POST /v1/systemone`).
`DECIDE_BACKEND=typesafe|local` selects one. When that variable is unset, a non-empty
`TYPESAFE_API_KEY` selects TypeSafe Jev and an absent key selects local. A failed call stays
on its backend. The two backends are not calibrated to each other. Both answer `noul` with
only `type` and `noul`.

- `typesafe` posts to `https://api.typesafe.ai/v1/systemone` with the bearer key (or to
  `DECIDE_TYPESAFE_URL` / config.toml `[typesafe].url` for another remote server of the same format).
- `local` posts, with no auth header, to a server that is started separately:
  `DECIDE_LOCAL_URL`, else config.toml `[local].url`, else `http://127.0.0.1:8009/v1/systemone`.
  `decide` never starts, serves, or watches it. `scripts/serve-local.sh` starts Kev-4B (`kev.serve`,
  MLX, Apple Silicon; kev pinned to commit `5e42a7a`), waits until it answers, sends two warmup
  requests, and stays in the foreground. Answers report `routing.backend: "local"` and
  `routing.model` as the server returns it (Kev echoes the `jev-latest` alias). Setup and
  measurements are in `docs/kev-setup.md`; the decisions are in `docs/local-http/`.
- Before 0.8.0, local ran Clef-flash in-process (`mlx-rs` + candle, weights from HuggingFace). That
  was removed in 0.8.0 in favour of the server; `docs/mlx-backend/` and
  `docs/superpowers/specs/2026-10-02-clef-flash-local-backend-design.md` are history. Old
  `[local].weights`/`repo`/`hf_*` keys and `CLEF_WEIGHTS` are ignored.

The repo has no Python except the stdlib hook `.claude/hooks/stop_verify.py`. The earlier Python
package and the Clef-flash comparison oracle (`scripts/clef_flash_oracle.py`) were deleted with the
code they checked.

## Commands

Install from the tap on arm64 macOS:

```bash
brew install sparktype/tap/decide
```

The published formula is `Formula/decide.rb` in `sparktype/homebrew-tap`. This repo's
`packaging/homebrew/decide.rb` records the same install. Version is 0.8.0 in `Cargo.toml`; the
formula file still names the 0.7.0 release (url, sha256, and the `mlx.metallib` it installs) until the
0.8.0 release asset exists, and must then drop `mlx.metallib`, because from 0.8.0 the release tarball
holds only `decide`. GitHub Actions builds `crates/decide` and uploads a release asset when a `v*`
tag is pushed; the formula downloads that prebuilt arm64 binary and installs it, no Rust toolchain
required at install time. The API key stays in the environment as `TYPESAFE_API_KEY`.
`.mcp.json` points `decide` at the stdio command `/opt/homebrew/bin/decide mcp`, which Claude Code
starts per session (about 5ms, no model to load), so no daemon is needed for MCP. Running
`decide install` registers the tool in Claude Code's user scope the same way, by shelling out to
`claude mcp add -s user decide -- /opt/homebrew/bin/decide mcp`; an existing `decide` registration
(for example the 0.7.0 HTTP one) is left alone with a hint to `claude mcp remove -s user decide`
first. The daemon is only for the gate hook, which starts it when the socket is down; it does not
survive a reboot and needs no manual start. Measured transport cost (`tools/list`, M1 Max): stdio
0.05ms per message, HTTP to the daemon 0.4ms, against backend calls of 100ms and up.

Runtime tests, no model server and no network (`tests/daemon.rs` binds :48080, so it fails while a real `decide daemon` is running):

```bash
cargo test --manifest-path crates/decide/Cargo.toml
```

One Rust test:

```bash
cargo test --manifest-path crates/decide/Cargo.toml protocol::tests::noul_rejects_options_and_criteria
```

Daemon socket `~/.cache/decide/decide.sock`, 30 minutes idle:

```bash
/opt/homebrew/bin/decide daemon
```

`decide mcp` is the stdio MCP server that `decide install` registers (the daemon's HTTP transport
below stays available but nothing registers it). The command
line follows the usual conventions (`src/help.rs`
owns every help text and keeps the command list in one place): `-h`/`--help` or no arguments print the
overview, `<command> --help` and `help <command>` print that command's help, `-V`/`--version` print
`decide <version>`; all of them exit 0 on stdout, and `main.rs` handles them before dispatch so `mcp --help`
and `daemon --help` never start a server. Unknown commands and bad arguments go to stderr with exit 2.
Help lines are kept within 80 terminal columns (Hangul counts as two) by a unit test. After editing `.mcp.json`
or `.claude/settings.json`, restart Claude Code.

## Architecture

`crates/decide` is the runtime.

- `protocol.rs` validates `state`, `type`, `instructions`, `options`, and `criteria`.
  Keep the Korean strings for too few choice options, too few score levels, and noul
  receiving options or criteria. The question id sent to either backend is `"q"`.
- `backend.rs` selects the backend and measures `latency_ms` around that call only.
- `decide_many` (`backend.rs`, `mcp.rs`, `daemon.rs`) sends several questions for one state
  in a single backend call. `protocol::parse_many`/`validate_many` keep input order and
  prefix errors with `질문 "<id>": `; `typesafe::request_body_many`/`map_answers` build the
  body and pick the requested ids. All-or-nothing; no question-count cap. `decide` is unchanged.
- `typesafe.rs` posts to `https://api.typesafe.ai/v1/systemone` (or to `DECIDE_TYPESAFE_URL` / config.toml
  `[typesafe].url` when set, e.g. a local Kev server that speaks the same System One format; see
  `resolved_typesafe_url` in `backend.rs`; the key must still be non-empty) with model
  `jev-latest`. Choice criteria are `{option: option}` in insertion order. Retry 429
  and 529 once after one second. Choice above 255 options and score above 10 levels
  fail before the request.
- `backend.rs` sends the local backend through the same `typesafe::execute` path as TypeSafe:
  `live_transport` picks `LiveTransport::local(url)` (no auth header; `pick_local_url` orders
  `DECIDE_LOCAL_URL`, `[local].url`, `DEFAULT_LOCAL_URL`) when the backend resolves to local. The
  255-option and 10-level limits are checked before the request for TypeSafe only; the local server
  decides for itself. A connection failure on local gets `with_local_hint` appended
  (`scripts/serve-local.sh`); an HTTP error keeps the server's message. `decide_many` on local is one
  request for all questions, so the server computes the state once (Kev also caches it across
  requests: about 470ms first call, 100ms repeat, M1 Max). `decide daemon` loads nothing; there is no
  preload.
- `mcp.rs` speaks newline-delimited JSON-RPC. Tool failures are `isError` results. It
  is transport-agnostic (`handle_message(&Value, &Env, &mut T) -> Option<Value>`) and
  is reused as-is by both the stdio loop (`main.rs::run_mcp`) and `http.rs`'s HTTP handler.
- `http.rs` is the MCP HTTP transport: `handle_http_body` parses one JSON-RPC message
  from a request body and feeds it to `mcp::handle_message`, returning `(status, body)`;
  JSON parse errors and JSON-RPC-level errors go in the response body (code `-32700`
  etc.), never the HTTP status. `serve_http`/`http_port_in_use` wrap this in a
  `tiny_http` server bound to a fixed local address, handling only `POST /mcp` (anything
  else is a 404) — no SSE, one request in and one JSON-RPC result out.
- `daemon.rs` serves one JSON line per UDS connection, same as before, and now also runs
  an HTTP server on the same process via `serve_unified` (`DEFAULT_HTTP_ADDR =
  "127.0.0.1:48080"`): both transports run on their own thread, share one
  `Arc<Mutex<Cache>>` (though in practice only the UDS/gate path uses the cache — MCP
  requests never did) and one `Arc<Mutex<Instant>>` idle timer, so either transport's
  traffic resets the shared 30-minute idle deadline and the daemon only exits once both
  go quiet. `serve_unified` claims the UDS socket first (`claim_socket`) and only then
  checks the HTTP port (`http_port_in_use`); if either is already taken, the process
  exits without starting (and un-claims the UDS socket on an HTTP-port conflict). A live
  socket is left in place; a dead socket file is replaced. Idle exit uses `poll` (UDS
  side) or `recv_timeout` (HTTP side). The LRU (`MAX_CACHE_ENTRIES` = 64, keyed on the
  parsed request plus the resolved backend) makes identical gate requests within one
  daemon lifetime skip the transport; cached answers carry `routing.cached: true` and
  `latency_ms: 0.0`. `decide mcp` (stdio) is a fresh process per call and has no cache.
  A request may carry `client_version`; when it differs from `daemon::VERSION` the
  daemon skips the backend, answers `{"stale":true,"version":…}`, and exits (UDS path
  only), so an upgraded client never keeps talking to an old daemon (`handle_request`
  returns `(reply, keep_serving)`). Requests without the field behave as before.
- `main.rs` routes `mcp`, `daemon`, `install`, `hook`, and `gate` subcommands (`gate` also has `--show` and `stats`). No arguments prints
  help (see `help.rs`). `install` shells out to `claude mcp add -s user decide -- /opt/homebrew/bin/decide mcp`
  (`install_mcp_args`) and no longer spawns the daemon.
- `claude.rs` merges hooks into Claude Code's user settings for `decide install --claude`:
  `add_hook_spec` (pure) appends one group for a `HookSpec` (event, matcher, command, timeout), idempotent on
  the exact command within that event, refusing shapes it cannot merge into; `install_hooks` installs the display
  hook (`PostToolUse`, `mcp__decide__decide`) and the gate hook (`PreToolUse`, `Bash`) in one pass
  (`hook_specs`); it reads the file (missing = `{}`, invalid
  JSON = error without writing), backs up to `settings.json.bak-decide`, and replaces it via a temp file
  keeping permissions. Bare `decide install` stays MCP-only.
- `show.rs` turns a `PostToolUse` hook input for `mcp__decide__decide` into a user-facing summary
  (`render(&Value) -> Option<String>`, pure). `decide hook` (`main.rs`) reads stdin, prints
  `{"systemMessage": ...}` as one JSON line, and stays silent with exit 0 on anything unreadable.
  `score_text` reads `legend` both as TypeSafe's object keyed by index strings and as the local
  backend's plain array.
- `gate/` runs the Claude Code hook gate behind `decide gate <name>` (design:
  `docs/superpowers/specs/2026-10-04-decide-gate-design.md`, plan and decision notes in `docs/decide-gate/`;
  the static rules and `stats` in `docs/gate-rules-stats/`).
  `config.rs` merges built-in defaults, `~/.config/decide/gates.json`, and `<cwd>/.decide/gates.json` (the repo
  layer may only tighten: enforce, enable, lower `deny`, raise `confidence`, shrink the prefilter, add to
  `deny_patterns`/`ask_patterns` as a union; `display` and `timeout_ms` are not repo-settable), holds the built-in
  static rule lists, and tracks each value's source for `--show`. `rules.rs` is the static rule layer: a
  glob matcher (`*` only, `\*` for a literal star, a trailing ` *` also matches no arguments) applied to each
  command segment (split on `&&`, `||`, `;`, newline outside quotes, `sudo`-style wrappers stripped, pipes kept
  whole), never to the whole command because a `*` would span separate commands. `run_hook` checks
  `deny → ask → prefilter → model`, so a rule hit never reaches the daemon (`Kind::Rule`). `stats.rs` aggregates
  `gate.log` (decision kinds, verdict mix, per-backend latency, timeouts, low-confidence asks, top rules) behind
  `decide gate stats`; it reports numbers only. `bash_risk.rs` holds the pure
  logic (secret redaction, the three-way `allow`/`ask`/`deny` choice request, prefilter that never applies to
  commands containing shell metacharacters, probabilities → verdict). `output.rs` turns an outcome into hook
  output JSON and the reasoning passed to the model via `hookSpecificOutput.additionalContext` (not
  `systemMessage`, which Claude Code shows the user but never forwards to the model; audit mode emits
  `additionalContext` only and never a `permissionDecision`; enforce adds `permissionDecision` for deny/ask
  alongside it) and renders `--show`. `client.rs`
  talks to the daemon socket, starts or replaces a daemon, and appends `~/.cache/decide/gate.log`. Every failure
  passes through silently with exit 0 (an unknown gate name is exit 1; exit 2 would block the tool call).

`.claude/hooks/stop_verify.py` remains a stdlib client. It spawns
`/opt/homebrew/bin/decide daemon` with the hook environment and fail-opens when the
socket is down. Its noul confidence threshold (default 0.4) reads from
`DECIDE_STOP_THRESHOLD`, falling back to 0.4 on a missing or unparsable value. This
one file is tracked in git despite `.claude/` being gitignored — see the exception
rule in `.gitignore`.

**`noul` is a probability, 0.0–1.0.** Apply any threshold at the call site. Phrase
`instructions` as a direct question. A rhetorical negation flips the direction
unreliably. See the README section "에이전트가 쓸 때".

**Tests inject the backend.** Rust tests use a scripted transport and a fake clock.
`tests/stop_hook.rs` runs `python3` against the hook's pure logic. Keep checks that
need a live local server or TypeSafe key (the `tests/gate_eval.rs` gate evaluation, which
needs `scripts/serve-local.sh` running) out of the default suites. `tests/gate_rules.rs`
needs no server and stays in the default suite: no allow-labelled command in any fixture set may match a default
rule, and the rules must match heldout3's pre-registered `rule_expect`. `tests/gate_rules_replay.rs` (`#[ignore]`)
replays a real `gate.log` through the default rules for a human to review false positives. Unix socket
paths are limited to about 104 bytes on macOS, so tests that bind sockets use short temp directory names.
