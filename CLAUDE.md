# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Development Commands

### Frontend
```bash
cd frontend
pnpm install
pnpm dev
pnpm build
```

### Tauri / Rust
```bash
cd src-tauri
cargo check
cargo test --no-run
cargo tauri dev
```

## Dependency Rules

- Local development must use:
  - `agent-lib = { path = "../../agent-lib" }`
- Do not add `features = [...]` to the `agent-lib` dependency.
- When promoting to stable integration, switch to `git + tag` dependency while still keeping no feature list.
