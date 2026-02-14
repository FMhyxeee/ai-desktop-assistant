# AI Desktop Assistant

Desktop assistant app that integrates `agent-lib` as the AI core.

## Repository Strategy

- This project is an independent Git repository.
- `agent-lib` is managed in its own repository.
- During local iteration this project uses a path dependency:
  - `src-tauri/Cargo.toml` -> `agent-lib = { path = "../../agent-lib" }`
  - `agent-lib` uses flattened build now, so do not add `features = [...]` on this dependency.

## Run Locally

1. Set required env vars (OpenAI default):
   - `OPENAI_API_KEY=...`
2. Optional runtime overrides:
   - `AGENT_PROVIDER=openai|glm`
   - `AGENT_MODEL=...`
   - `AGENT_API_KEY_ENV=OPENAI_API_KEY|GLM_API_KEY`
   - `AGENT_SYSTEM_PROMPT=...`
3. Run:
   - `cd src-tauri`
   - `cargo tauri dev`

## Tauri Commands

- `ask_agent(input: String) -> String`
- `start_agent_stream(input: String, task_id?: String) -> String`
- `cancel_agent_task(task_id: String) -> ()`

Stream events are emitted on `agent://event` with payload:

- `started`
- `delta`
- `completed`
- `error`

## Promotion To Git Tag Dependency

After `agent-lib` is stable:

1. Create tag in `agent-lib`, for example `v0.1.0`.
2. Replace path dependency with:

```toml
agent-lib = { git = "https://<your-host>/<org>/agent-lib.git", tag = "v0.1.0" }
```

Do not add a feature list for `agent-lib` in the git dependency form either.

3. Commit dependency upgrade in this repository.
