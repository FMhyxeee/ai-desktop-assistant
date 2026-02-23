pub mod agent_service;
pub mod storage;

use std::sync::Arc;
use std::time::Instant;

use agent_service::types::{
    AgentEvent, AgentHistoryMessage, AgentHistoryRole, AgentRuntimeConfig, AgentStreamInput,
    GovernanceReport, GovernanceUpdateReport, McpConfigTestResult, McpRuntimeConfig,
    SkillScanResult, SkillsRuntimeConfig, WorkspaceRuntimeConfig,
};
use agent_service::AgentService;
use serde::Serialize;
use tauri::async_runtime::Mutex;
use tauri::Emitter;

#[derive(Clone)]
struct AppState {
    agent_service: Arc<Mutex<AgentService>>,
    storage_service: Arc<storage::StorageService>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ConnectionTestResult {
    success: bool,
    message: String,
    latency_ms: u128,
}

#[tauri::command]
async fn ask_agent(state: tauri::State<'_, AppState>, input: String) -> Result<String, String> {
    let service = { state.agent_service.lock().await.clone() };
    service.chat(input).await.map_err(|err| err.to_string())
}

#[tauri::command]
async fn start_agent_stream(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
    input: serde_json::Value,
    task_id: Option<String>,
) -> Result<String, String> {
    let task_id = task_id.unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
    let mut stream_input = parse_stream_input(input)?;
    let service = { state.agent_service.lock().await.clone() };
    let workspace_root = service.current_workspace_root();
    let memory_config = service.current_memory_config();
    if memory_config.enabled {
        let recall_options = storage::RecallBuildOptions {
            enabled: memory_config.enabled,
            max_recall_items: memory_config.max_recall_items,
            max_non_secret_items: memory_config.max_non_secret_items,
            max_secret_items: memory_config.max_secret_items,
            secret_similarity_threshold: memory_config.secret_similarity_threshold,
            require_explicit_secret_intent: memory_config.require_explicit_secret_intent,
        };
        if let Ok(Some(recall_context)) = state
            .storage_service
            .build_recall_context_with_options(
                &workspace_root,
                &stream_input.content,
                recall_options,
            )
            .await
        {
            stream_input.recent_messages.push(AgentHistoryMessage {
                role: AgentHistoryRole::System,
                content: recall_context,
            });
        }
    }
    service
        .chat_stream(task_id.clone(), stream_input, move |event: AgentEvent| {
            let _ = app.emit("agent://event", &event);
        })
        .await
        .map_err(|err| err.to_string())?;
    Ok(task_id)
}

fn parse_stream_input(input: serde_json::Value) -> Result<AgentStreamInput, String> {
    match input {
        serde_json::Value::String(text) => Ok(AgentStreamInput::text(text)),
        value => {
            if let Ok(parsed) = serde_json::from_value::<AgentStreamInput>(value.clone()) {
                return Ok(parsed);
            }

            #[derive(serde::Deserialize)]
            struct LegacyStreamInput {
                kind: Option<String>,
                content: String,
            }

            let legacy = serde_json::from_value::<LegacyStreamInput>(value)
                .map_err(|err| format!("invalid stream input: {err}"))?;

            let normalized_content = match legacy
                .kind
                .as_deref()
                .map(|kind| kind.trim().to_ascii_lowercase())
            {
                Some(kind) if kind == "command" => {
                    let trimmed = legacy.content.trim();
                    if trimmed.starts_with('/') {
                        trimmed.to_string()
                    } else if trimmed.is_empty() {
                        "/".to_string()
                    } else {
                        format!("/{trimmed}")
                    }
                }
                _ => legacy.content,
            };

            Ok(AgentStreamInput {
                content: normalized_content,
                images: Vec::new(),
                conversation_id: None,
                recent_messages: Vec::new(),
            })
        }
    }
}

#[tauri::command]
async fn resolve_config_change_request(
    state: tauri::State<'_, AppState>,
    task_id: String,
    request_id: String,
    approved: bool,
    persist: bool,
) -> Result<(), String> {
    let service = { state.agent_service.lock().await.clone() };
    service
        .resolve_config_change_request(&task_id, &request_id, approved, persist)
        .await
        .map_err(|err| err.to_string())
}

#[tauri::command]
async fn cancel_agent_task(
    state: tauri::State<'_, AppState>,
    task_id: String,
) -> Result<(), String> {
    let service = { state.agent_service.lock().await.clone() };
    service
        .cancel(&task_id)
        .await
        .map_err(|err| err.to_string())
}

#[tauri::command]
async fn update_runtime_config(
    state: tauri::State<'_, AppState>,
    config: serde_json::Value,
) -> Result<GovernanceUpdateReport, String> {
    let previous_service = { state.agent_service.lock().await.clone() };
    let before = previous_service.run_governance_scan(None).await.ok();
    let previous_runtime_config = previous_service.current_runtime_config();

    let parsed_config = serde_json::from_value::<AgentRuntimeConfig>(config.clone())
        .map_err(|err| format!("invalid runtime config payload: {err}"))?;
    let merged_config = merge_runtime_config_with_previous(
        &config,
        parsed_config,
        previous_runtime_config.as_ref(),
    );

    let service = AgentService::new_with_config(merged_config)
        .await
        .map_err(|err| err.to_string())?;
    let after = service
        .latest_governance_report()
        .ok_or_else(|| "missing governance report after runtime config update".to_string())?;

    let mut guard = state.agent_service.lock().await;
    *guard = service;

    Ok(GovernanceUpdateReport { before, after })
}

fn merge_runtime_config_with_previous(
    raw_config: &serde_json::Value,
    mut incoming: AgentRuntimeConfig,
    previous: Option<&AgentRuntimeConfig>,
) -> AgentRuntimeConfig {
    let Some(previous) = previous else {
        return incoming;
    };

    if raw_config.get("memory").is_none() {
        incoming.memory = previous.memory.clone();
    }
    if raw_config.get("control").is_none() {
        incoming.control = previous.control.clone();
    }

    incoming
}

#[tauri::command]
async fn test_runtime_config(config: AgentRuntimeConfig) -> Result<ConnectionTestResult, String> {
    let started = Instant::now();
    let service = match AgentService::new_with_config(config).await {
        Ok(service) => service,
        Err(err) => {
            return Ok(ConnectionTestResult {
                success: false,
                message: format!("配置校验失败：{err}"),
                latency_ms: started.elapsed().as_millis(),
            });
        }
    };

    let response = service.chat("请只回复：OK".to_string()).await;
    let latency_ms = started.elapsed().as_millis();

    match response {
        Ok(output) => Ok(ConnectionTestResult {
            success: true,
            message: format!("连接成功，模型返回：{}", summarize_text(&output, 80)),
            latency_ms,
        }),
        Err(err) => Ok(ConnectionTestResult {
            success: false,
            message: format!("连接失败：{err}"),
            latency_ms,
        }),
    }
}

#[tauri::command]
async fn test_mcp_config(config: McpRuntimeConfig) -> Result<McpConfigTestResult, String> {
    Ok(agent_service::test_mcp_runtime_config(config).await)
}

#[tauri::command]
async fn run_governance_scan(
    state: tauri::State<'_, AppState>,
    config: Option<AgentRuntimeConfig>,
) -> Result<GovernanceReport, String> {
    if let Some(config) = config {
        return Ok(agent_service::run_governance_scan_with_config(config).await);
    }

    let service = { state.agent_service.lock().await.clone() };
    service
        .run_governance_scan(None)
        .await
        .map_err(|err| err.to_string())
}

#[tauri::command]
async fn scan_skills_config(
    config: SkillsRuntimeConfig,
    workspace: Option<WorkspaceRuntimeConfig>,
) -> Result<SkillScanResult, String> {
    Ok(agent_service::scan_skills_runtime_config(config, workspace).await)
}

async fn resolve_workspace_root(state: &tauri::State<'_, AppState>) -> std::path::PathBuf {
    let service = { state.agent_service.lock().await.clone() };
    service.current_workspace_root()
}

#[tauri::command]
async fn storage_bootstrap(
    state: tauri::State<'_, AppState>,
    request: Option<storage::StorageBootstrapRequest>,
) -> Result<storage::StorageBootstrapResponse, String> {
    let workspace_root = resolve_workspace_root(&state).await;
    state
        .storage_service
        .bootstrap_workspace(&workspace_root, request.unwrap_or_default())
        .await
        .map_err(|err| err.to_string())
}

#[tauri::command]
async fn storage_upsert_conversation(
    state: tauri::State<'_, AppState>,
    conversation: storage::ConversationSnapshot,
) -> Result<(), String> {
    let workspace_root = resolve_workspace_root(&state).await;
    state
        .storage_service
        .upsert_conversation(&workspace_root, conversation)
        .await
        .map_err(|err| err.to_string())
}

#[tauri::command]
async fn storage_delete_conversation(
    state: tauri::State<'_, AppState>,
    conversation_id: String,
) -> Result<(), String> {
    let workspace_root = resolve_workspace_root(&state).await;
    state
        .storage_service
        .delete_conversation(&workspace_root, &conversation_id)
        .await
        .map_err(|err| err.to_string())
}

#[tauri::command]
async fn storage_set_current_conversation(
    state: tauri::State<'_, AppState>,
    conversation_id: Option<String>,
) -> Result<(), String> {
    let workspace_root = resolve_workspace_root(&state).await;
    state
        .storage_service
        .set_current_conversation(&workspace_root, conversation_id)
        .await
        .map_err(|err| err.to_string())
}

#[tauri::command]
async fn storage_export_conversation(
    state: tauri::State<'_, AppState>,
    conversation_id: String,
) -> Result<storage::ConversationSnapshot, String> {
    let workspace_root = resolve_workspace_root(&state).await;
    state
        .storage_service
        .export_conversation(&workspace_root, &conversation_id)
        .await
        .map_err(|err| err.to_string())
}

#[tauri::command]
async fn memory_search(
    state: tauri::State<'_, AppState>,
    request: storage::MemorySearchRequest,
) -> Result<storage::MemorySearchResponse, String> {
    let workspace_root = resolve_workspace_root(&state).await;
    state
        .storage_service
        .memory_search(&workspace_root, request)
        .await
        .map_err(|err| err.to_string())
}

#[tauri::command]
async fn memory_upsert_personal_note(
    state: tauri::State<'_, AppState>,
    request: storage::MemoryUpsertPersonalNoteRequest,
) -> Result<storage::PersonalMemoryEntry, String> {
    state
        .storage_service
        .upsert_personal_note(request)
        .await
        .map_err(|err| err.to_string())
}

#[tauri::command]
async fn memory_upsert_personal_secret(
    state: tauri::State<'_, AppState>,
    request: storage::MemoryUpsertPersonalSecretRequest,
) -> Result<storage::PersonalMemoryEntry, String> {
    state
        .storage_service
        .upsert_personal_secret(request)
        .await
        .map_err(|err| err.to_string())
}

#[tauri::command]
async fn memory_list_personal(
    state: tauri::State<'_, AppState>,
) -> Result<storage::PersonalMemoryListResponse, String> {
    state
        .storage_service
        .list_personal()
        .await
        .map_err(|err| err.to_string())
}

#[tauri::command]
async fn memory_delete_personal(
    state: tauri::State<'_, AppState>,
    request: storage::MemoryDeletePersonalRequest,
) -> Result<(), String> {
    state
        .storage_service
        .delete_personal(request)
        .await
        .map_err(|err| err.to_string())
}

fn summarize_text(input: &str, max_chars: usize) -> String {
    let compact = input.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut buf = String::new();
    for ch in compact.chars().take(max_chars) {
        buf.push(ch);
    }
    if compact.chars().count() > max_chars {
        format!("{buf}...")
    } else if buf.is_empty() {
        "(空响应)".to_string()
    } else {
        buf
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn merge_runtime_config_with_previous_preserves_missing_memory_and_control() {
        let mut previous = AgentRuntimeConfig::default();
        previous.memory.enabled = false;
        previous.control.enabled = false;

        let mut raw = serde_json::to_value(AgentRuntimeConfig::default())
            .expect("default config should serialize");
        let object = raw
            .as_object_mut()
            .expect("serialized config should be json object");
        object.remove("memory");
        object.remove("control");

        let incoming: AgentRuntimeConfig =
            serde_json::from_value(raw.clone()).expect("payload should deserialize");
        let merged = merge_runtime_config_with_previous(&raw, incoming, Some(&previous));

        assert!(!merged.memory.enabled);
        assert!(!merged.control.enabled);
    }

    #[test]
    fn merge_runtime_config_with_previous_keeps_explicit_memory_and_control() {
        let mut previous = AgentRuntimeConfig::default();
        previous.memory.enabled = false;
        previous.control.enabled = false;

        let mut incoming_config = AgentRuntimeConfig::default();
        incoming_config.memory.enabled = true;
        incoming_config.control.enabled = true;

        let raw = serde_json::to_value(&incoming_config).expect("incoming config should serialize");
        let merged =
            merge_runtime_config_with_previous(&raw, incoming_config.clone(), Some(&previous));

        assert!(merged.memory.enabled);
        assert!(merged.control.enabled);
    }
}

#[tauri::command]
fn frontend_log(
    level: String,
    message: String,
    context: Option<serde_json::Value>,
) -> Result<(), String> {
    let context_suffix = context
        .map(|value| format!(" | context={value}"))
        .unwrap_or_default();

    match level.to_ascii_lowercase().as_str() {
        "debug" => log::debug!("frontend: {message}{context_suffix}"),
        "warn" | "warning" => log::warn!("frontend: {message}{context_suffix}"),
        "error" => log::error!("frontend: {message}{context_suffix}"),
        _ => log::info!("frontend: {message}{context_suffix}"),
    }

    Ok(())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let agent_service = tauri::async_runtime::block_on(AgentService::from_env())
        .expect("agent service bootstrap failed");
    let storage_service = Arc::new(storage::StorageService::new());

    tauri::Builder::default()
        .manage(AppState {
            agent_service: Arc::new(Mutex::new(agent_service)),
            storage_service,
        })
        .invoke_handler(tauri::generate_handler![
            ask_agent,
            start_agent_stream,
            resolve_config_change_request,
            cancel_agent_task,
            update_runtime_config,
            test_runtime_config,
            test_mcp_config,
            run_governance_scan,
            scan_skills_config,
            storage_bootstrap,
            storage_upsert_conversation,
            storage_delete_conversation,
            storage_set_current_conversation,
            storage_export_conversation,
            memory_search,
            memory_upsert_personal_note,
            memory_upsert_personal_secret,
            memory_list_personal,
            memory_delete_personal,
            frontend_log
        ])
        .plugin(tauri_plugin_store::Builder::new().build())
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            if cfg!(debug_assertions) {
                app.handle().plugin(
                    tauri_plugin_log::Builder::default()
                        .level(log::LevelFilter::Debug)
                        .build(),
                )?;
            }
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
