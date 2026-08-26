# Reflect-TUI

[![CI](https://github.com/Asketisch/Reflect-TUI/actions/workflows/ci.yml/badge.svg)](https://github.com/Asketisch/Reflect-TUI/actions/workflows/ci.yml)
[![License](https://img.shields.io/badge/license-Apache--2.0-blue.svg)](LICENSE)
[![Rust](https://img.shields.io/badge/rust-1.85%20%7C%20edition%202024-orange.svg)](rust-toolchain.toml)
[![中文](https://img.shields.io/badge/README-中文-red.svg)](README.md)

An interactive terminal client for the Reflect Agent, designed for terminal-based AI agent workflows, built on [ratatui](https://github.com/ratatui-org/ratatui).

This repository contains **only the TUI presentation layer** (`reflect-tui` crate). The agent core engine (AgentThread, StateGraph, tools, protocol, …) is provided by [Reflect-Agent](https://github.com/Asketisch/Reflect-Agent) and consumed via a **git submodule**.

## Repository Layout

```
Reflect-Agent (core engine, provided as submodule)
  └─ crates/{protocol,abilities,resources,orchestration,integrations,runtime}
  └─ reflect-core (AgentThread + StateGraph)

Reflect-TUI (this repo)  ──submodule──► Reflect-Agent
  └─ crates/reflect-tui (ratatui UI + event loop)
  └─ reflect-tui binary (terminal entry point)
```

## Quick Start

```bash
# 1. Clone (with submodules)
git clone --recursive https://github.com/Asketisch/Reflect-TUI.git
cd Reflect-TUI

# 2. Build
make build

# 3. Configure an LLM provider (pick one)
#    Option A: environment variable
export ANTHROPIC_API_KEY=sk-ant-...
#    Option B: ~/.reflect/config.toml — [active] / [anthropic] / [openai]

# 4. Run
make run
# or directly:
./target/release/reflect-tui
```

The Rust toolchain is pinned by `rust-toolchain.toml` (stable, MSRV 1.85, edition 2024).

## TUI Command-Line Flags

With no subcommand, the binary launches the TUI. Common flags (see `reflect-tui --help` for the full list):

```bash
reflect-tui [--cwd PATH] [--prompt TEXT] [-c] [-r N] [--resume ID]
            [--plan-mode] [--fullscreen] [--auto-root]
            [--ephemeral-tasks] [--ephemeral-teams]

# Examples
reflect-tui "refactor this module for me"   # launch with an initial prompt
reflect-tui --resume sess_abc123            # resume a specific session
reflect-tui -c                              # continue the last session
reflect-tui -r 2                            # resume the 2nd most recent session
reflect-tui --plan-mode                     # start in Plan mode
reflect-tui --fullscreen                    # alternate-screen fullscreen (also REFLECT_TUI_FULLSCREEN=1)
reflect-tui --cwd /path/to/project          # set the working directory
```

## In-TUI Slash Commands

Type `/` to open the command list. Frequently used commands:

| Command | Description |
|------|------|
| `/model` | Switch model and reasoning strength |
| `/status` / `/usage` | View session config / token usage |
| `/plan` | Enter Plan mode (draft plan first, then approve and execute) |
| `/goal` | Set / view a long-running task goal |
| `/loop <secs> <cmd>` | Repeat a command at an interval (`/loop stop` to cancel) |
| `/fork` | Fork a new sub-session from a historical user prompt |
| `/rename <name>` | Rename the current session |
| `/resume` / `/archive` / `/delete` | Resume / archive / delete sessions |
| `/compact` | Compact the context |
| `/review` | Review the current diff |
| `/mcp` / `/skills` / `/plugins` / `/hooks` | Manage extensions |
| `/theme` / `/keymap` / `/vim` | Appearance and key bindings |
| `/quit` | Exit |

## CLI Subcommands

When invoked with a subcommand, the binary runs headlessly and exits without entering the TUI.

### `session` — local session management

```bash
# List local sessions (supports --limit/-n, --model substring filter)
reflect-tui session ls -n 20
reflect-tui session ls --model anthropic

# Show metadata for a session + the first 5 message previews + cumulative tokens/cost
reflect-tui session show <id-or-prefix>

# Delete a session (--yes to skip confirmation)
reflect-tui session rm <id> --yes

# Fork a parent session's full history up to now into a new child session
reflect-tui session fork <id> --branch my-branch
# After forking the child id is printed and the hint:
#   Resume with: reflect-tui --resume <child_id> "..."

# Set a human-readable name (written to ~/.reflect/sessions/_names/<id>.name)
reflect-tui session rename <id> "my session name"

# Export to Markdown (prints to stdout without --out)
reflect-tui session export <id>
reflect-tui session export <id> --out session.md
```

`<id>` accepts a full UUID or the first 8-character prefix (unique match).

### `traces` — local LLM call records

```bash
# List traces (one line per session: session_id / title / calls / last / model)
reflect-tui traces ls -n 20

# Show per-call LLM details for a session (model / latency / usage / request+response summary)
reflect-tui traces show <id-or-prefix>
```

Data comes from `~/.reflect/traces/model-io-sess_<session_id>.jsonl` (written by reflect-telemetry).

## Build, Test, and Release

```bash
make build          # release build (with LTO)
make test           # cargo test --workspace
make test-fast      # cargo nextest (requires cargo-nextest)
make check-fast C=reflect-tui   # fast type-check for a single crate
make dist           # build + package current-platform artifacts (tar.gz/zip) into dist/
make install        # install to /usr/local/bin (or ~/.local/bin, see scripts/install.sh)
make help           # list all targets
```

- `./scripts/build.sh --fast` uses the `release-fast` profile (no LTO, faster incremental builds).
- The `test/` directory holds all test scripts (layering documented in `test/README.md`): `python3 test/run_all.py` runs the full offline suite (unit / rust / cli / binary / mock E2E) in one shot, while `python3 test/test_workflow_live.py` is the local live end-to-end script (real LLM + real tools) for manually validating Plan mode / sub-agents / `/goal` / `/loop` end-to-end.
- The release CI (`.github/workflows/release.yml`) only triggers on a `v*` tag and builds distribution packages in parallel on macOS / Linux / Windows. Submodules must be checked out recursively.

## Upgrading the Core

Reflect-Agent is a submodule. To bump the core to the latest:

```bash
git submodule update --remote reflect-agent
git add reflect-agent
git commit -m "chore: bump reflect-agent submodule"
```

For local joint development on the core: make changes in the Reflect-Agent repo → `git push` → in this repo run `git submodule update --remote`. See the "Submodule workflow" section in [CONTRIBUTING.md](CONTRIBUTING.md) for the full process (branch switching, pointer updates, joint debugging).

## Documentation

- [CONTRIBUTING.md](CONTRIBUTING.md) — Contributing guide (Chinese-comment policy, submodule workflow, PR requirements)
- [CLAUDE.md](CLAUDE.md) — AI assistant project notes (the source of the Chinese-comment policy)
- [CHANGELOG.md](CHANGELOG.md) — Changelog

## Community & Security

- [CONTRIBUTING.md](CONTRIBUTING.md) — How to contribute code
- [CODE_OF_CONDUCT.md](CODE_OF_CONDUCT.md) — Code of conduct
- [SECURITY.md](SECURITY.md) — Vulnerability disclosure policy
- [GitHub Discussions](https://github.com/Asketisch/Reflect-TUI/discussions) — Questions and discussion
- [GitHub Issues](https://github.com/Asketisch/Reflect-TUI/issues) — Bug reports and feature requests

## Attribution / Third-Party Notices

This project builds on the following open-source works (full attribution in [NOTICE](NOTICE)):

- [Reflect-Agent](https://github.com/Asketisch/Reflect-Agent) — Core engine (Apache-2.0), consumed via git submodule
- [nornagon/ratatui](https://github.com/nornagon/ratatui) and [nornagon/crossterm](https://github.com/nornagon/crossterm) — Forks that expose unstable features (MIT)

---

**Note on language policy**: All source-code comments, commit messages, and PR descriptions in this repository **must be written in Chinese**. This applies to contributions — please see [CONTRIBUTING.md](CONTRIBUTING.md) before opening a PR. This README is provided in English only for accessibility of the project overview.