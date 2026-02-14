pub mod agent_service;

use agent_service::types::AgentEvent;
use agent_service::AgentService;
use tauri::Emitter;

#[derive(Clone)]
struct AppState {
    agent_service: AgentService,
}

#[tauri::command]
async fn ask_agent(state: tauri::State<'_, AppState>, input: String) -> Result<String, String> {
    state
        .agent_service
        .chat(input)
        .await
        .map_err(|err| err.to_string())
}

#[tauri::command]
async fn start_agent_stream(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
    input: String,
    task_id: Option<String>,
) -> Result<String, String> {
    let task_id = task_id.unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
    state
        .agent_service
        .chat_stream(task_id.clone(), input, move |event: AgentEvent| {
            let _ = app.emit("agent://event", &event);
        })
        .await
        .map_err(|err| err.to_string())?;
    Ok(task_id)
}

#[tauri::command]
async fn cancel_agent_task(
    state: tauri::State<'_, AppState>,
    task_id: String,
) -> Result<(), String> {
    state
        .agent_service
        .cancel(&task_id)
        .await
        .map_err(|err| err.to_string())
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
    let agent_service = AgentService::from_env().expect("agent service bootstrap failed");

    tauri::Builder::default()
        .manage(AppState { agent_service })
        .invoke_handler(tauri::generate_handler![
            ask_agent,
            start_agent_stream,
            cancel_agent_task,
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
