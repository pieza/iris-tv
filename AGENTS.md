# Repository Instructions

## Project Overview

IRIS is a Rust CLI and local HTTP server for controlling infrared TVs from a Raspberry Pi. It loads editable TOML remote profiles from `profiles/tv/<brand>/<model>.toml` and sends IR commands through a GPIO-connected infrared LED.

The source should stay profile-data driven. Do not hardcode TV remote codes in Rust when a TOML profile can express them.

## Architecture

- `src/cli`: Clap command definitions and command orchestration.
- `src/config`: User config loading and persistence for `~/.config/iris/config.toml`.
- `src/profiles`: TOML profile parsing, brand/model resolution, and command lookup.
- `src/ir`: Protocol-neutral IR signal types plus dry-run, mock, and Raspberry Pi GPIO transmitters.
- `src/server`: Local HTTP API for automation integrations.
- `src/daemon`: PID-file based background server management.
- `src/discovery`: Zeroconf/mDNS Home Assistant discovery metadata.
- `src/errors`: Typed user-facing errors.

Read `README.md` for product behavior and `docs/ARCHITECTURE.md` before making non-trivial changes.

## Development Commands

Run the narrowest relevant checks for the change, and run the full set before claiming completion for behavior changes:

```bash
cargo fmt --check
cargo clippy --all-targets --no-default-features -- -D warnings
cargo test --no-default-features
cargo check --features rpi-gpio
```

For local development away from a Raspberry Pi, use dry-run behavior:

```bash
cargo run -- send power --dry-run
```

## Coding Guidelines

- Keep public CLI behavior, error text, and README examples aligned.
- Prefer small, focused modules that follow the existing architecture.
- Keep new IR protocols behind `IrSignal`/transmitter abstractions.
- Add new TV support as TOML profiles under `profiles/tv/<brand>/<model>.toml`.
- Keep tests deterministic; use temp directories and mock or dry-run transmitters instead of real GPIO.
- Do not run commands that require real Raspberry Pi GPIO hardware unless the user explicitly asks.

## Security And Safety

- Do not commit real API tokens, local config files, secrets, or `.env` values.
- The server must not be exposed on a LAN address without an `api_token`.
- Treat Home Assistant discovery data as integration metadata, not authentication.
- Preserve user changes in the working tree. Do not revert unrelated edits.

## Agent Workflow

- Inspect the repo first, then make scoped changes.
- Use Conventional Commits for commit messages: `<type>: <concise imperative summary>`, for example `feat: add power command` or `fix: handle missing active profile`.
- Prefer CodeGraph for structural code questions when it is initialized. If `.codegraph/` is missing or the server reports "not initialized", ask before running `codegraph init -i`.
- Treat CodeGraph indexes and generated caches as local-only unless the user explicitly asks to commit them.
- Use `rg`/`rg --files` for literal text and file searches.
- Before finalizing, report which verification commands ran and whether any were skipped.
