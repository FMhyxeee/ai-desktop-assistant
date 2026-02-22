# AGENTS.md

Primary contributor guide: `CLAUDE.md`.

## Repository-Specific Rules

- Keep `agent-lib` dependency in `src-tauri/Cargo.toml` without `features = [...]`.
- During local development use path dependency:
  - `agent-lib = { path = "../../agent-lib" }`

## Stage2/Stage3 Workflow Invariants

- Build guidance via structured result objects, then render prompt fragment:
  - `build_stage2_guidance_result(...) -> Stage2GuidanceResult`
  - `render_guidance_system_fragment(...) -> String`
- Emit `guidance_context` protocol event every turn (input + output).
- Keep backward-compatible warning events (`Guidance context injected...` + `GuidanceAudit ...`).
- `tool_call_requested.normalization` is preview observability only; it must not change tool execution semantics.
- Runtime MCP argument normalization at execution time remains authoritative.

## Memory and Secret Handling

- Use runtime memory policy from `AgentRuntimeConfig.memory` (and env override `AGENT_MEMORY_CONFIG_JSON`).
- Build recall via `build_recall_context_with_options(...)` (default wrapper remains for compatibility).
- Never write secret values into guidance protocol structures/events.
- Secret values may only appear in internal recall system message and must be redacted in guidance summaries.

## Protocol Change Checklist

- Any Rust protocol change in `src-tauri/src/agent_service/types.rs` must be mirrored in:
  - `frontend/src/types/index.ts`
  - `frontend/src/store/appStore.ts` (summary + level mapping)
- Preserve compatibility for existing front-end event handling.

## Validation Baseline

- `cd src-tauri && cargo check`
- `cd src-tauri && cargo test`
- `cd frontend && pnpm lint`
- `cd frontend && pnpm build`
