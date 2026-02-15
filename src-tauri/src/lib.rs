pub mod agent_service;

use std::sync::Arc;
use std::time::Instant;

use agent_service::types::{
    AgentEvent, AgentRuntimeConfig, AgentStreamInput, McpConfigTestResult, McpRuntimeConfig,
    SkillScanResult, SkillsRuntimeConfig,
};
use agent_service::AgentService;
use serde::Serialize;
use tauri::async_runtime::Mutex;
use tauri::Emitter;

#[derive(Clone)]
struct AppState {
    agent_service: Arc<Mutex<AgentService>>,
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
    let stream_input = parse_stream_input(input)?;
    let service = { state.agent_service.lock().await.clone() };
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
        value => serde_json::from_value::<AgentStreamInput>(value)
            .map_err(|err| format!("invalid stream input: {err}")),
    }
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
    config: AgentRuntimeConfig,
) -> Result<(), String> {
    let service = AgentService::new_with_config(config)
        .await
        .map_err(|err| err.to_string())?;
    let mut guard = state.agent_service.lock().await;
    *guard = service;
    Ok(())
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
async fn scan_skills_config(config: SkillsRuntimeConfig) -> Result<SkillScanResult, String> {
    Ok(agent_service::scan_skills_runtime_config(config).await)
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

    tauri::Builder::default()
        .manage(AppState {
            agent_service: Arc::new(Mutex::new(agent_service)),
        })
        .invoke_handler(tauri::generate_handler![
            ask_agent,
            start_agent_stream,
            cancel_agent_task,
            update_runtime_config,
            test_runtime_config,
            test_mcp_config,
            scan_skills_config,
            frontend_log
        ])
        .plugin(tauri_plugin_store::Builder::new().build())
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
