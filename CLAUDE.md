# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## What this is

`decide` is an MCP tool for one non-generative judgment: `choice`, `score`, or `noul`.
The installed program is the Rust binary `/opt/homebrew/bin/decide`. One process has two
backends. `DECIDE_BACKEND=typesafe|local` selects one. When that variable is unset, a
non-empty `TYPESAFE_API_KEY` selects TypeSafe Jev and an absent key selects local.
A failed call stays on its backend. The two backends are not calibrated to each
other. Both answer `noul` with only `type` and `noul`.

The local backend runs real in-process inference — no server, no network call per
request. It runs the Clef-flash backbone (Qwen3.5-9B hybrid attention) on the Apple Silicon GPU
through MLX: `mlx-community/clef-flash-8bit` (8-bit affine, about 10.7GB), via `mlx-rs` and a Metal
gated-delta kernel through `mlx-sys` (`local/mlx_backbone.rs`), plus a from-scratch joint schema head that
runs on candle CPU. Weights are read from `CLEF_WEIGHTS` if set (a missing file there is a hard error, no
silent fallback; the directory must hold that repo's `config.json`, `model.safetensors.index.json`, shards,
and `joint_head.safetensors`) or downloaded once from HuggingFace (`mlx-community/clef-flash-8bit` for the
backbone and head, `Cloudflare/clef-flash` for the tokenizer) into `~/.cache/huggingface/hub` otherwise —
first use costs about 10.7GB of disk and real time. Building needs cmake and the Metal Toolchain
(`xcodebuild -downloadComponent MetalToolchain`), and the binary only runs on Apple Silicon. Local answers
follow the same field shape as TypeSafe (`choice`/`score`/`noul` plus `confidence` and `probabilities`);
`routing.model` is `"clef-flash"`. Design: `docs/mlx-backend/` (plan, checklist, decision notes) and
`docs/superpowers/specs/2026-10-02-clef-flash-local-backend-design.md` (the earlier CPU/GGUF design, kept
as history; that engine has since been removed).

Measured on an M1 Max (64GB): `cargo test --features parity` agrees with the BF16 oracle on 5/5 golden
cases (max raw-logit diff 0.095, including an 11-option choice question whose top-2 options differ by only
about 0.02 probability), and a warm call takes about 0.6s for ~150 tokens and 2.8s for ~900 tokens. The
removed candle CPU path took 30s and 125s for the same inputs and agreed on 4/5. Prefill is compute-bound at
roughly 3.6ms/token, so expect little more from kernel work on this chip. The gated-delta kernel is checked
against an ops-based reference in a unit test.

The old Python package (`src/decide/`, its `pytest` suite, `test_smoke.py`, `pyproject.toml`)
was the comparison oracle for an earlier local backend and was deleted once that
backend's execution verification passed. It has since been recreated as a
comparison oracle for Clef-flash — see `scripts/clef_flash_oracle.py` instead, a
Python script using the real HuggingFace `transformers` library against the real
Cloudflare/clef-flash model; it stays uncommitted-weights, run manually with
`cargo test --features parity`, not part of the default test suite. Besides that
script and the stdlib hook `.claude/hooks/stop_verify.py`, the repo has no other
Python.

## Commands

Install from the tap on arm64 macOS:

```bash
brew install sparktype/tap/decide
```

The published formula is `Formula/decide.rb` in `sparktype/homebrew-tap`. This repo's
`packaging/homebrew/decide.rb` records the same install. Version is 0.2.1. GitHub
Actions builds `crates/decide` and uploads a release asset when a `v*` tag is pushed;
the formula downloads that prebuilt arm64 binary and installs it, no Rust toolchain
required at install time. The release tarball holds `decide` and `mlx.metallib` (the MLX GPU
kernels) side by side, and the formula must install both into the same directory: MLX looks next
to the executable first and otherwise falls back to a path baked in at build time
(`/Users/runner/.mlx/lib/...`), which does not exist on a user's machine. v0.2.0 shipped without it
and every inference failed. The API key stays in the environment as `TYPESAFE_API_KEY`.
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
- `decide_many` (`backend.rs`, `mcp.rs`, `daemon.rs`) sends several questions for one state
  in a single backend call. `protocol::parse_many`/`validate_many` keep input order and
  prefix errors with `질문 "<id>": `; `typesafe::request_body_many`/`map_answers` build the
  body and pick the requested ids. All-or-nothing; no question-count cap. `decide` is unchanged.
- `typesafe.rs` posts to `https://api.typesafe.ai/v1/systemone` with model
  `jev-latest`. Choice criteria are `{option: option}` in insertion order. Retry 429
  and 529 once after one second. Choice above 255 options and score above 10 levels
  fail before the request.
- `local/mod.rs` resolves weights (`CLEF_WEIGHTS` env override, else the HuggingFace
  cache; `ensure_weights`/`resolve_pinned`/`download_weights`), lazily builds the backbone + joint
  head + tokenizer once per process (`runtime()`, a `OnceLock`), and exposes
  `infer` (state + question → the same answer shape as TypeSafe, via
  `postprocess::to_answer`). `local/mlx_backbone.rs` is the MLX port of the Qwen3.5
  hybrid-attention forward pass (full attention plus a gated-delta Metal kernel); it hands the
  head the hidden states and only the option tokens' dequantized lm_head rows (`head_inputs`).
  `local/joint_head.rs` is the from-scratch
  schema head (EvidenceRoutingLayer × 2 + TransformerDecoderLayer × 4);
  `local/tokenizer.rs` assembles the Clef schema text and token spans.
  `backend::decide`/`decide_many` call `local::infer` directly — no transport,
  no `Backend::Local` arm in `typesafe::LiveTransport`.
- `mcp.rs` speaks newline-delimited JSON-RPC. Tool failures are `isError` results.
- `daemon.rs` serves one JSON line per connection. A live socket is left in place. A
  dead socket file is replaced. Idle exit uses `poll`. It also holds an in-process
  LRU (`MAX_CACHE_ENTRIES` = 64, keyed on the parsed request plus the resolved
  backend) so identical requests within one daemon lifetime skip the transport;
  cached answers carry `routing.cached: true` and `latency_ms: 0.0`. `decide mcp`
  is a fresh process per call and has no cache. A request may carry `client_version`;
  when it differs from `daemon::VERSION` the daemon skips the backend, answers
  `{"stale":true,"version":…}`, and exits, so an upgraded client never keeps talking to an old
  daemon (`handle_request` returns `(reply, keep_serving)`). Requests without the field behave as before.
- `main.rs` routes `mcp`, `daemon`, `install`, `hook`, and `gate` subcommands. No arguments prints
  help. `install` shells out to `claude mcp add -s user decide -- <bin> mcp`.
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
  `docs/superpowers/specs/2026-10-04-decide-gate-design.md`, plan and decision notes in `docs/decide-gate/`).
  `config.rs` merges built-in defaults, `~/.config/decide/gates.json`, and `<cwd>/.decide/gates.json` (the repo
  layer may only tighten: enforce, enable, lower `deny`, raise `confidence`, shrink the prefilter; `display` and
  `timeout_ms` are not repo-settable) and tracks each value's source for `--show`. `bash_risk.rs` holds the pure
  logic (secret redaction, the three-way `allow`/`ask`/`deny` choice request, prefilter that never applies to
  commands containing shell metacharacters, probabilities → verdict). `output.rs` turns an outcome into hook
  output JSON and the reasoning shown to the user (audit mode emits `systemMessage` only and never a
  `permissionDecision`; enforce adds `permissionDecision` for deny/ask) and renders `--show`. `client.rs`
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
need real weights or a live TypeSafe key (`--features parity`, `#[ignore]` local
inference tests, the `tests/gate_eval.rs` gate evaluation) out of the default suites. Unix socket
paths are limited to about 104 bytes on macOS, so tests that bind sockets use short temp directory names.
