pub mod types;

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use std::time::Instant;

use agent_lib::mcp::{
    AuthConfig as AgentMcpAuthConfig, AuthType as AgentMcpAuthType, CallToolRequestParams,
    McpClient, McpManager, ServerConfig as AgentMcpServerConfig, TlsConfig as AgentMcpTlsConfig,
    TransportType as AgentMcpTransportType,
};
use agent_lib::model::provider::{
    AnthropicProvider, GlmCodingPlanProvider, GlmProvider, LocalProvider, OpenAiProvider,
};
use agent_lib::model::{ModelClient, TokenUsage};
use agent_lib::protocol::{ApprovalPolicy, Op, ReasoningSummary, SandboxPolicy, UserInputItem};
use agent_lib::skills::{SkillConfig, SkillLoader, SkillSource};
use agent_lib::session::{Session, SessionConfig, SessionHandle};
use agent_lib::tools::{Tool, ToolContext, ToolDef, ToolExecutor, ToolRegistry, ToolResult};
use agent_lib::{AgentBuilder, AgentError, AgentResult, Event, TurnAbortReason};
use serde_json::Value;
use tauri::async_runtime::{JoinHandle, Mutex};
use tokio::time::timeout;

use self::types::{
    AgentEvent, AgentProvider, AgentRuntimeConfig, AgentStreamInput, AppError, McpAuthRuntimeConfig,
    McpAuthType, McpConfigTestResult, McpRuntimeConfig, McpServerRuntimeConfig,
    McpServerTestResult, McpTlsRuntimeConfig, McpTransportKind, ProtocolEventPayload,
    ProtocolMcpPromptInfo, ProtocolMcpResourceInfo, ProtocolMcpToolInfo, ProtocolOpPayload,
    ProtocolPromptArgumentInfo, ProtocolPromptContent, ProtocolPromptMessage, ProtocolSkillEntry,
    ProtocolTokenUsage, SkillScanEntry, SkillScanResult, SkillsRuntimeConfig, StreamInputKind,
};

struct ActiveTask {
    join_handle: JoinHandle<()>,
    session_handle: Option<SessionHandle>,
}

#[derive(Clone)]
pub struct AgentService {
    runner: Arc<dyn Runner>,
    runtime_config: Option<AgentRuntimeConfig>,
    runtime_mcp_manager: Option<Arc<McpManager>>,
    runtime_tool_executor: Option<Arc<ToolExecutor>>,
    runtime_skill_config: Option<SkillConfig>,
    tasks: Arc<Mutex<HashMap<String, ActiveTask>>>,
}

impl AgentService {
    pub async fn from_env() -> Result<Self, AppError> {
        let config = load_runtime_config();
        match Self::new_with_config(config).await {
            Ok(service) => Ok(service),
            Err(err) => Ok(Self::with_runner(Arc::new(FailingRunner {
                message: err.to_string(),
            }))),
        }
    }

    pub async fn new_with_config(config: AgentRuntimeConfig) -> Result<Self, AppError> {
        let runner = AgentLibRunner::new(config.clone())?;
        let runtime_resources = build_runtime_resources(&config).await?;
        Ok(Self {
            runner: Arc::new(runner),
            runtime_config: Some(config),
            runtime_mcp_manager: runtime_resources.mcp_manager,
            runtime_tool_executor: runtime_resources.tool_executor,
            runtime_skill_config: Some(runtime_resources.skill_config),
            tasks: Arc::new(Mutex::new(HashMap::new())),
        })
    }

    pub fn with_runner(runner: Arc<dyn Runner>) -> Self {
        Self {
            runner,
            runtime_config: None,
            runtime_mcp_manager: None,
            runtime_tool_executor: None,
            runtime_skill_config: None,
            tasks: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    pub async fn chat(&self, input: String) -> Result<String, AppError> {
        self.runner.run(&input).await.map_err(Into::into)
    }

    pub async fn chat_stream<F>(
        &self,
        task_id: String,
        input: AgentStreamInput,
        mut emit: F,
    ) -> Result<(), AppError>
    where
        F: FnMut(AgentEvent) + Send + 'static,
    {
        emit(AgentEvent::Started {
            task_id: task_id.clone(),
        });

        if let Some(config) = self.runtime_config.clone() {
            self.chat_stream_protocol(task_id, input, config, emit).await
        } else {
            self.chat_stream_legacy(task_id, input, emit).await
        }
    }

    async fn chat_stream_protocol<F>(
        &self,
        task_id: String,
        input: AgentStreamInput,
        config: AgentRuntimeConfig,
        mut emit: F,
    ) -> Result<(), AppError>
    where
        F: FnMut(AgentEvent) + Send + 'static,
    {
        let model = build_model_client(&config)?;

        let session_config = SessionConfig {
            model: Some(model),
            default_model: config.model.clone(),
            default_cwd: Some(".".to_string()),
            default_approval_policy: Some(ApprovalPolicy::NeverAsk),
            mcp_manager: self.runtime_mcp_manager.clone(),
            tool_executor: self.runtime_tool_executor.clone(),
            skill_config: self.runtime_skill_config.clone(),
            ..Default::default()
        };

        let (_session, handle) = Session::with_config(64, session_config);
        let control_handle = handle.clone();
        let (op, op_payload, is_command_input) = build_stream_op(&input, &config);

        emit(AgentEvent::OpSubmitted {
            task_id: task_id.clone(),
            seq: 1,
            payload: op_payload,
        });

        handle.submit(op).await?;

        let tasks = Arc::clone(&self.tasks);
        let task_key = task_id.clone();
        let task_id_for_worker = task_id.clone();
        let worker_handle = handle;

        let join_handle = tauri::async_runtime::spawn(async move {
            let mut seq = 1_u64;
            let mut completed_sent = false;
            let mut aggregated_output = String::new();

            loop {
                let next_event = timeout(Duration::from_secs(120), worker_handle.next_event()).await;
                let event = match next_event {
                    Ok(Some(event)) => event,
                    Ok(None) => {
                        seq += 1;
                        emit(AgentEvent::ProtocolEvent {
                            task_id: task_id_for_worker.clone(),
                            seq,
                            payload: ProtocolEventPayload::Error {
                                code: "event_stream_closed".to_string(),
                                message: "Event stream closed".to_string(),
                            },
                        });
                        emit(AgentEvent::Error {
                            task_id: task_id_for_worker.clone(),
                            code: "event_stream_closed".to_string(),
                            message: "Event stream closed".to_string(),
                        });
                        break;
                    }
                    Err(_) => {
                        seq += 1;
                        emit(AgentEvent::ProtocolEvent {
                            task_id: task_id_for_worker.clone(),
                            seq,
                            payload: ProtocolEventPayload::Error {
                                code: "timeout".to_string(),
                                message: "No events received within 120s".to_string(),
                            },
                        });
                        emit(AgentEvent::Error {
                            task_id: task_id_for_worker.clone(),
                            code: "timeout".to_string(),
                            message: "No events received within 120s".to_string(),
                        });
                        break;
                    }
                };

                if let Some(payload) = map_protocol_event(&event) {
                    seq += 1;
                    emit(AgentEvent::ProtocolEvent {
                        task_id: task_id_for_worker.clone(),
                        seq,
                        payload,
                    });
                }

                match event {
                    Event::ModelStreaming { chunk } => {
                        if !chunk.is_empty() {
                            aggregated_output.push_str(&chunk);
                            emit(AgentEvent::Delta {
                                task_id: task_id_for_worker.clone(),
                                chunk,
                            });
                        }
                    }
                    Event::ModelComplete { content, .. } => {
                        if !completed_sent {
                            let output = if content.is_empty() {
                                aggregated_output.clone()
                            } else {
                                aggregated_output = content.clone();
                                content
                            };
                            emit(AgentEvent::Completed {
                                task_id: task_id_for_worker.clone(),
                                output,
                            });
                            completed_sent = true;
                        }
                        if !is_command_input {
                            break;
                        }
                    }
                    Event::ToolCallResult { tool, result } => {
                        if is_command_input && tool == "shell" {
                            if aggregated_output.trim().is_empty() {
                                aggregated_output = value_to_text(&result.output);
                            }
                            if !completed_sent {
                                emit(AgentEvent::Completed {
                                    task_id: task_id_for_worker.clone(),
                                    output: aggregated_output.clone(),
                                });
                            }
                            break;
                        }
                    }
                    Event::TurnComplete { result } => {
                        if !completed_sent {
                            let output = if aggregated_output.trim().is_empty() {
                                value_to_text(&result)
                            } else {
                                aggregated_output.clone()
                            };
                            emit(AgentEvent::Completed {
                                task_id: task_id_for_worker.clone(),
                                output,
                            });
                        }
                        break;
                    }
                    Event::TurnAborted { reason } => {
                        emit(AgentEvent::Error {
                            task_id: task_id_for_worker.clone(),
                            code: "turn_aborted".to_string(),
                            message: format!("Turn aborted: {}", turn_abort_reason_to_text(&reason)),
                        });
                        break;
                    }
                    Event::Error { error } => {
                        emit(AgentEvent::Error {
                            task_id: task_id_for_worker.clone(),
                            code: error_code(&error),
                            message: error.to_string(),
                        });
                        break;
                    }
                    _ => {}
                }
            }

            tasks.lock().await.remove(&task_key);
        });

        self.tasks.lock().await.insert(
            task_id,
            ActiveTask {
                join_handle,
                session_handle: Some(control_handle),
            },
        );
        Ok(())
    }

    async fn chat_stream_legacy<F>(
        &self,
        task_id: String,
        input: AgentStreamInput,
        mut emit: F,
    ) -> Result<(), AppError>
    where
        F: FnMut(AgentEvent) + Send + 'static,
    {
        let prompt = input.content;
        let runner = Arc::clone(&self.runner);
        let tasks = Arc::clone(&self.tasks);
        let task_key = task_id.clone();
        let task_id_for_worker = task_id.clone();

        let join_handle = tauri::async_runtime::spawn(async move {
            match runner.run(&prompt).await {
                Ok(output) => {
                    for chunk in chunk_text(&output, 48) {
                        emit(AgentEvent::Delta {
                            task_id: task_id_for_worker.clone(),
                            chunk,
                        });
                    }
                    emit(AgentEvent::Completed {
                        task_id: task_id_for_worker,
                        output,
                    });
                }
                Err(err) => {
                    emit(AgentEvent::Error {
                        task_id: task_id_for_worker,
                        code: "agent_error".to_string(),
                        message: err.to_string(),
                    });
                }
            }
            tasks.lock().await.remove(&task_key);
        });

        self.tasks.lock().await.insert(
            task_id,
            ActiveTask {
                join_handle,
                session_handle: None,
            },
        );
        Ok(())
    }

    pub async fn cancel(&self, task_id: &str) -> Result<(), AppError> {
        let task = self.tasks.lock().await.remove(task_id);
        if let Some(task) = task {
            if let Some(handle) = task.session_handle {
                let _ = handle.submit(Op::Interrupt).await;
            }
            task.join_handle.abort();
            Ok(())
        } else {
            Err(AppError::TaskNotFound(task_id.to_string()))
        }
    }
}

struct RuntimeResources {
    mcp_manager: Option<Arc<McpManager>>,
    tool_executor: Option<Arc<ToolExecutor>>,
    skill_config: SkillConfig,
}

pub async fn test_mcp_runtime_config(config: McpRuntimeConfig) -> McpConfigTestResult {
    if !config.enabled {
        return McpConfigTestResult {
            success: true,
            message: "MCP 已禁用".to_string(),
            server_results: vec![],
        };
    }

    let mut server_results = Vec::new();
    let default_timeout_secs = config.default_timeout_secs.unwrap_or(30).max(1);
    let max_retries = config.max_retries.unwrap_or(3);

    for server in &config.servers {
        let started = Instant::now();
        if !server.enabled {
            server_results.push(McpServerTestResult {
                name: server.name.clone(),
                enabled: false,
                success: true,
                latency_ms: 0,
                tool_count: 0,
                tools: vec![],
                error: None,
            });
            continue;
        }

        let server_config = match map_runtime_mcp_server_config(server, default_timeout_secs) {
            Ok(value) => value,
            Err(err) => {
                server_results.push(McpServerTestResult {
                    name: server.name.clone(),
                    enabled: true,
                    success: false,
                    latency_ms: started.elapsed().as_millis(),
                    tool_count: 0,
                    tools: vec![],
                    error: Some(err.to_string()),
                });
                continue;
            }
        };

        let manager = McpManager::with_timeout_and_retries(
            Duration::from_secs(default_timeout_secs),
            max_retries,
        );
        match manager.add_server_with_config(server_config).await {
            Ok(tools) => {
                let tool_names = tools
                    .iter()
                    .map(|tool| tool.name.to_string())
                    .collect::<Vec<_>>();
                server_results.push(McpServerTestResult {
                    name: server.name.clone(),
                    enabled: true,
                    success: true,
                    latency_ms: started.elapsed().as_millis(),
                    tool_count: tools.len(),
                    tools: tool_names,
                    error: None,
                });
            }
            Err(err) => {
                server_results.push(McpServerTestResult {
                    name: server.name.clone(),
                    enabled: true,
                    success: false,
                    latency_ms: started.elapsed().as_millis(),
                    tool_count: 0,
                    tools: vec![],
                    error: Some(err.to_string()),
                });
            }
        }
    }

    let all_enabled_ok = server_results
        .iter()
        .filter(|item| item.enabled)
        .all(|item| item.success);
    let enabled_count = server_results.iter().filter(|item| item.enabled).count();

    McpConfigTestResult {
        success: all_enabled_ok,
        message: if enabled_count == 0 {
            "未配置启用的 MCP 服务器".to_string()
        } else if all_enabled_ok {
            format!("{} 个 MCP 服务器测试通过", enabled_count)
        } else {
            format!("{} 个 MCP 服务器中存在连接失败", enabled_count)
        },
        server_results,
    }
}

pub async fn scan_skills_runtime_config(config: SkillsRuntimeConfig) -> SkillScanResult {
    if !config.enabled {
        return SkillScanResult {
            success: true,
            message: "Skills 已禁用".to_string(),
            warnings: vec![],
            skills: vec![],
        };
    }

    let skill_config = map_runtime_skill_config(&config);
    let mut warnings = Vec::new();
    let mut skills = Vec::new();
    let loader = SkillLoader::new();

    if let Some(personal_dir) = &skill_config.personal_dir {
        match loader
            .load_from_directory(personal_dir, &SkillSource::Personal)
            .await
        {
            Ok(entries) => skills.extend(entries),
            Err(err) => warnings.push(format!(
                "扫描 personal skills 目录失败 ({}): {}",
                personal_dir.display(),
                err
            )),
        }
    } else if let Some(home) = skill_home_dir() {
        let dir = home.join(".cursor").join("skills");
        match loader.load_from_directory(&dir, &SkillSource::Personal).await {
            Ok(entries) => skills.extend(entries),
            Err(err) => warnings.push(format!(
                "扫描默认 personal skills 目录失败 ({}): {}",
                dir.display(),
                err
            )),
        }
    }

    if skill_config.project_dirs.is_empty() {
        let default_project_dir = PathBuf::from(".cursor").join("skills");
        match loader
            .load_from_directory(&default_project_dir, &SkillSource::Project)
            .await
        {
            Ok(entries) => skills.extend(entries),
            Err(err) => warnings.push(format!(
                "扫描默认 project skills 目录失败 ({}): {}",
                default_project_dir.display(),
                err
            )),
        }
    } else {
        for dir in &skill_config.project_dirs {
            match loader.load_from_directory(dir, &SkillSource::Project).await {
                Ok(entries) => skills.extend(entries),
                Err(err) => warnings.push(format!(
                    "扫描 project skills 目录失败 ({}): {}",
                    dir.display(),
                    err
                )),
            }
        }
    }

    let entries = skills
        .into_iter()
        .map(|skill| SkillScanEntry {
            name: skill.metadata.name,
            description: skill.metadata.description,
            path: skill.path.to_string_lossy().to_string(),
            source: skill.source.as_label().to_string(),
            has_auxiliary_files: !skill.auxiliary_files.is_empty(),
        })
        .collect::<Vec<_>>();

    SkillScanResult {
        success: warnings.is_empty(),
        message: format!("扫描完成，发现 {} 个 skills", entries.len()),
        warnings,
        skills: entries,
    }
}

async fn build_runtime_resources(config: &AgentRuntimeConfig) -> Result<RuntimeResources, AppError> {
    let skill_config = map_runtime_skill_config(&config.skills);
    if !config.mcp.enabled {
        return Ok(RuntimeResources {
            mcp_manager: None,
            tool_executor: None,
            skill_config,
        });
    }

    let default_timeout_secs = config.mcp.default_timeout_secs.unwrap_or(30).max(1);
    let max_retries = config.mcp.max_retries.unwrap_or(3);
    let manager =
        McpManager::with_timeout_and_retries(Duration::from_secs(default_timeout_secs), max_retries);

    for server in &config.mcp.servers {
        if !server.enabled {
            continue;
        }

        let server_config = map_runtime_mcp_server_config(server, default_timeout_secs)?;
        manager
            .add_server_with_config(server_config)
            .await
            .map_err(|err| AppError::InvalidConfig(err.to_string()))?;
    }

    let mut registry = ToolRegistry::new();
    let tools = manager.get_all_tools().await;
    for (server_name, tool_def, client) in tools {
        let tool = PrefixedMcpTool::new(
            server_name,
            tool_def.name.to_string(),
            tool_def
                .description
                .as_deref()
                .unwrap_or_default()
                .to_string(),
            Value::Object((*tool_def.input_schema).clone()),
            client,
        );
        registry.register(Arc::new(tool));
    }

    Ok(RuntimeResources {
        mcp_manager: Some(manager),
        tool_executor: Some(Arc::new(ToolExecutor::new(registry))),
        skill_config,
    })
}

fn map_runtime_skill_config(config: &SkillsRuntimeConfig) -> SkillConfig {
    let personal_dir = normalize_optional(config.personal_dir.clone()).map(PathBuf::from);
    let project_dirs = config
        .project_dirs
        .iter()
        .filter_map(|dir| {
            let trimmed = dir.trim();
            if trimmed.is_empty() {
                None
            } else {
                Some(PathBuf::from(trimmed))
            }
        })
        .collect::<Vec<_>>();

    SkillConfig {
        enabled: config.enabled,
        personal_dir,
        project_dirs,
        auto_apply: config.auto_apply,
    }
}

fn map_runtime_mcp_server_config(
    server: &McpServerRuntimeConfig,
    default_timeout_secs: u64,
) -> Result<AgentMcpServerConfig, AppError> {
    let name = normalize_optional(Some(server.name.clone()))
        .ok_or_else(|| AppError::InvalidConfig("MCP server name cannot be empty".to_string()))?;
    let transport = map_runtime_transport(server.transport)?;
    let endpoint = normalize_optional(server.endpoint.clone()).unwrap_or_default();
    let command = normalize_optional(server.command.clone());
    let args = server
        .args
        .iter()
        .filter_map(|arg| {
            let trimmed = arg.trim();
            if trimmed.is_empty() {
                None
            } else {
                Some(trimmed.to_string())
            }
        })
        .collect::<Vec<_>>();
    let auth = map_runtime_mcp_auth(server.auth.as_ref(), &name)?;
    let tls = map_runtime_mcp_tls(server.tls.as_ref());

    Ok(AgentMcpServerConfig {
        name,
        transport,
        endpoint,
        command,
        args,
        auth,
        headers: normalize_map(&server.headers),
        tls,
        timeout: Duration::from_secs(server.timeout_secs.unwrap_or(default_timeout_secs).max(1)),
        enabled: server.enabled,
        env: normalize_map(&server.env),
    })
}

fn unsupported_transport_message(value: &str) -> String {
    format!(
        "Unsupported transport '{}'. Supported: stdio, streamable_http. http/https -> streamable_http; tcp/ws/wss/sse are removed in strict official mode.",
        value
    )
}

fn map_runtime_transport(kind: McpTransportKind) -> Result<AgentMcpTransportType, AppError> {
    match kind {
        McpTransportKind::Stdio => Ok(AgentMcpTransportType::Stdio),
        McpTransportKind::StreamableHttp | McpTransportKind::Http | McpTransportKind::Https => {
            Ok(AgentMcpTransportType::StreamableHttp)
        }
        McpTransportKind::Tcp => Err(AppError::InvalidConfig(unsupported_transport_message("tcp"))),
        McpTransportKind::Websocket => Err(AppError::InvalidConfig(
            unsupported_transport_message("websocket"),
        )),
        McpTransportKind::Wss => Err(AppError::InvalidConfig(unsupported_transport_message("wss"))),
        McpTransportKind::Sse => Err(AppError::InvalidConfig(unsupported_transport_message("sse"))),
    }
}

fn map_runtime_mcp_tls(config: Option<&McpTlsRuntimeConfig>) -> Option<AgentMcpTlsConfig> {
    config.map(|tls| AgentMcpTlsConfig {
        ca_cert_path: normalize_optional(tls.ca_cert_path.clone()),
        client_cert_path: normalize_optional(tls.client_cert_path.clone()),
        client_key_path: normalize_optional(tls.client_key_path.clone()),
        danger_accept_invalid_certs: tls.danger_accept_invalid_certs,
        danger_accept_invalid_hostnames: tls.danger_accept_invalid_hostnames,
    })
}

fn map_runtime_mcp_auth(
    config: Option<&McpAuthRuntimeConfig>,
    server_name: &str,
) -> Result<Option<AgentMcpAuthConfig>, AppError> {
    let Some(config) = config else {
        return Ok(None);
    };

    let auth_type = match config.auth_type {
        McpAuthType::None => AgentMcpAuthType::None,
        McpAuthType::Bearer => AgentMcpAuthType::Bearer,
        McpAuthType::Basic => AgentMcpAuthType::Basic,
        McpAuthType::ApiKey => AgentMcpAuthType::ApiKey,
        McpAuthType::OAuth2 => AgentMcpAuthType::OAuth2,
    };

    let auth = AgentMcpAuthConfig {
        auth_type: auth_type.clone(),
        token: resolve_secret_env(config.token_env.as_deref(), server_name, "token_env")?,
        username: resolve_secret_env(config.username_env.as_deref(), server_name, "username_env")?,
        password: resolve_secret_env(config.password_env.as_deref(), server_name, "password_env")?,
        api_key: resolve_secret_env(config.api_key_env.as_deref(), server_name, "api_key_env")?,
        api_key_header: normalize_optional(config.api_key_header.clone()),
        query_param: normalize_optional(config.query_param.clone()),
        token_url: normalize_optional(config.token_url.clone()),
        client_id: resolve_secret_env(config.client_id_env.as_deref(), server_name, "client_id_env")?,
        client_secret: resolve_secret_env(
            config.client_secret_env.as_deref(),
            server_name,
            "client_secret_env",
        )?,
        scope: normalize_optional(config.scope.clone()),
        audience: normalize_optional(config.audience.clone()),
    };

    match auth_type {
        AgentMcpAuthType::Bearer => {
            if auth.token.is_none() {
                return Err(AppError::InvalidConfig(format!(
                    "MCP server '{}' requires token_env for bearer auth",
                    server_name
                )));
            }
        }
        AgentMcpAuthType::Basic => {
            if auth.username.is_none() || auth.password.is_none() {
                return Err(AppError::InvalidConfig(format!(
                    "MCP server '{}' requires username_env/password_env for basic auth",
                    server_name
                )));
            }
        }
        AgentMcpAuthType::ApiKey => {
            if auth.api_key.is_none() {
                return Err(AppError::InvalidConfig(format!(
                    "MCP server '{}' requires api_key_env for api_key auth",
                    server_name
                )));
            }
        }
        AgentMcpAuthType::OAuth2 => {
            let has_static_token = auth.token.is_some();
            let has_flow = auth.token_url.is_some() && auth.client_id.is_some() && auth.client_secret.is_some();
            if !has_static_token && !has_flow {
                return Err(AppError::InvalidConfig(format!(
                    "MCP server '{}' requires token_env or token_url + client_id_env + client_secret_env for oauth2",
                    server_name
                )));
            }
        }
        AgentMcpAuthType::None => {}
    }

    Ok(Some(auth))
}

fn resolve_secret_env(
    env_name: Option<&str>,
    server_name: &str,
    field_name: &str,
) -> Result<Option<String>, AppError> {
    let Some(raw_name) = env_name else {
        return Ok(None);
    };
    let key = raw_name.trim();
    if key.is_empty() {
        return Ok(None);
    }

    let value = std::env::var(key).map_err(|_| {
        AppError::MissingEnv(format!(
            "{} (required by MCP server '{}', field '{}')",
            key, server_name, field_name
        ))
    })?;
    let normalized = normalize_optional(Some(value)).ok_or_else(|| {
        AppError::MissingEnv(format!(
            "{} (required by MCP server '{}', field '{}')",
            key, server_name, field_name
        ))
    })?;
    Ok(Some(normalized))
}

fn normalize_map(input: &HashMap<String, String>) -> HashMap<String, String> {
    input
        .iter()
        .filter_map(|(key, value)| {
            let normalized_key = key.trim();
            let normalized_value = value.trim();
            if normalized_key.is_empty() || normalized_value.is_empty() {
                None
            } else {
                Some((normalized_key.to_string(), normalized_value.to_string()))
            }
        })
        .collect()
}

fn skill_home_dir() -> Option<PathBuf> {
    if let Ok(home) = std::env::var("USERPROFILE") {
        return Some(PathBuf::from(home));
    }
    std::env::var("HOME").ok().map(PathBuf::from)
}

#[derive(Debug, Clone)]
struct PrefixedMcpTool {
    public_name: String,
    call_name: String,
    description: String,
    schema: Value,
    client: Arc<McpClient>,
}

impl PrefixedMcpTool {
    fn new(
        server_name: String,
        call_name: String,
        description: String,
        schema: Value,
        client: Arc<McpClient>,
    ) -> Self {
        Self {
            public_name: format!("mcp:{server_name}:{call_name}"),
            call_name,
            description,
            schema,
            client,
        }
    }
}

#[async_trait::async_trait]
impl Tool for PrefixedMcpTool {
    fn definition(&self) -> ToolDef {
        ToolDef {
            name: self.public_name.clone(),
            description: self.description.clone(),
            schema: self.schema.clone(),
        }
    }

    async fn execute(&self, args: Value, _ctx: &ToolContext) -> AgentResult<ToolResult> {
        let arguments = match args {
            Value::Object(arguments) => arguments,
            _ => {
                return Err(AgentError::Tool(
                    "MCP tool arguments must be a JSON object".to_string(),
                ));
            }
        };

        let result = self
            .client
            .call_tool(CallToolRequestParams {
                meta: None,
                name: self.call_name.clone().into(),
                arguments: Some(arguments),
                task: None,
            })
            .await
            .map_err(|err| AgentError::Tool(format!("MCP tool call failed: {}", err)))?;

        let output = serde_json::to_value(result)
            .map_err(|err| AgentError::Tool(format!("Failed to encode MCP tool result: {}", err)))?;

        Ok(ToolResult { output })
    }
}

fn map_protocol_event(event: &Event) -> Option<ProtocolEventPayload> {
    match event {
        Event::TurnStarted { turn_id } => Some(ProtocolEventPayload::TurnStarted {
            turn_id: turn_id.clone(),
        }),
        Event::ModelStreaming { chunk } => Some(ProtocolEventPayload::ModelStreaming {
            chunk: chunk.clone(),
        }),
        Event::ModelComplete { content, usage } => Some(ProtocolEventPayload::ModelComplete {
            content: content.clone(),
            usage: protocol_usage(usage),
        }),
        Event::ToolCallRequested { tool, args } => Some(ProtocolEventPayload::ToolCallRequested {
            tool: tool.clone(),
            args: args.clone(),
        }),
        Event::ToolCallResult { tool, result } => Some(ProtocolEventPayload::ToolCallResult {
            tool: tool.clone(),
            result: result.output.clone(),
        }),
        Event::RunUserShellCommand { command } => Some(ProtocolEventPayload::RunUserShellCommand {
            command: command.clone(),
        }),
        Event::Warning { message } => Some(ProtocolEventPayload::Warning {
            message: message.clone(),
        }),
        Event::Error { error } => Some(ProtocolEventPayload::Error {
            code: error_code(error),
            message: error.to_string(),
        }),
        Event::TurnAborted { reason } => Some(ProtocolEventPayload::TurnAborted {
            reason: turn_abort_reason_to_text(reason),
        }),
        Event::TurnComplete { result } => Some(ProtocolEventPayload::TurnComplete {
            result: result.clone(),
        }),
        Event::McpListToolsResponse { tools } => Some(ProtocolEventPayload::McpListToolsResponse {
            tools: tools
                .iter()
                .map(|item| ProtocolMcpToolInfo {
                    name: item.name.clone(),
                    description: item.description.clone(),
                    server: item.server.clone(),
                })
                .collect(),
        }),
        Event::McpListResourcesResponse { resources } => {
            Some(ProtocolEventPayload::McpListResourcesResponse {
                resources: resources
                    .iter()
                    .map(|item| ProtocolMcpResourceInfo {
                        uri: item.uri.clone(),
                        name: item.name.clone(),
                        description: item.description.clone(),
                        mime_type: item.mime_type.clone(),
                    })
                    .collect(),
            })
        }
        Event::McpResourceContent { uri, content } => Some(ProtocolEventPayload::McpResourceContent {
            uri: uri.clone(),
            content: content.clone(),
        }),
        Event::McpListPromptsResponse { prompts } => Some(ProtocolEventPayload::McpListPromptsResponse {
            prompts: prompts
                .iter()
                .map(|item| ProtocolMcpPromptInfo {
                    name: item.name.clone(),
                    description: item.description.clone(),
                    arguments: item.arguments.as_ref().map(|args| {
                        args.iter()
                            .map(|arg| ProtocolPromptArgumentInfo {
                                name: arg.name.clone(),
                                description: arg.description.clone(),
                                required: arg.required,
                            })
                            .collect()
                    }),
                })
                .collect(),
        }),
        Event::McpPromptResult { messages } => Some(ProtocolEventPayload::McpPromptResult {
            messages: messages
                .iter()
                .map(|item| ProtocolPromptMessage {
                    role: item.role.clone(),
                    content: match &item.content {
                        agent_lib::protocol::PromptContent::Text { text } => {
                            ProtocolPromptContent::Text { text: text.clone() }
                        }
                        agent_lib::protocol::PromptContent::Image { data, mime_type } => {
                            ProtocolPromptContent::Image {
                                data: data.clone(),
                                mime_type: mime_type.clone(),
                            }
                        }
                    },
                })
                .collect(),
        }),
        Event::ListSkillsResponse { skills } => Some(ProtocolEventPayload::ListSkillsResponse {
            skills: skills
                .iter()
                .map(|item| ProtocolSkillEntry {
                    name: item.name.clone(),
                    description: item.description.clone(),
                    path: item.path.to_string_lossy().to_string(),
                    source: item.source.clone(),
                    has_auxiliary_files: item.has_auxiliary_files,
                })
                .collect(),
        }),
        Event::SkillContent {
            name,
            content,
            auxiliary_files,
        } => Some(ProtocolEventPayload::SkillContent {
            name: name.clone(),
            content: content.clone(),
            auxiliary_files: auxiliary_files.clone(),
        }),
        Event::SkillApplied { name } => Some(ProtocolEventPayload::SkillApplied {
            name: name.clone(),
        }),
        Event::SkillFileContent {
            skill_name,
            file_path,
            content,
        } => Some(ProtocolEventPayload::SkillFileContent {
            skill_name: skill_name.clone(),
            file_path: file_path.clone(),
            content: content.clone(),
        }),
        _ => None,
    }
}

fn protocol_usage(usage: &TokenUsage) -> ProtocolTokenUsage {
    ProtocolTokenUsage {
        prompt_tokens: usage.prompt_tokens,
        completion_tokens: usage.completion_tokens,
        total_tokens: usage.total_tokens,
    }
}

fn build_stream_op(
    input: &AgentStreamInput,
    config: &AgentRuntimeConfig,
) -> (Op, ProtocolOpPayload, bool) {
    match input.kind {
        StreamInputKind::Text => {
            let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
            (
                Op::UserTurn {
                    items: vec![UserInputItem::Text {
                        text: input.content.clone(),
                    }],
                    cwd: cwd.clone(),
                    approval_policy: ApprovalPolicy::NeverAsk,
                    sandbox_policy: SandboxPolicy::Persistent,
                    model: config.model.clone(),
                    effort: None,
                    summary: ReasoningSummary {
                        summary: String::new(),
                        token_count: 0,
                    },
                    final_output_json_schema: None,
                    collaboration_mode: None,
                },
                ProtocolOpPayload::UserTurn {
                    model: config.model.clone(),
                    cwd: cwd.to_string_lossy().to_string(),
                    approval_policy: "never_ask".to_string(),
                    sandbox_policy: "persistent".to_string(),
                    text: input.content.clone(),
                },
                false,
            )
        }
        StreamInputKind::Command => (
            Op::RunUserShellCommand {
                command: input.content.clone(),
            },
            ProtocolOpPayload::RunUserShellCommand {
                command: input.content.clone(),
            },
            true,
        ),
    }
}

fn turn_abort_reason_to_text(reason: &TurnAbortReason) -> String {
    match reason {
        TurnAbortReason::UserCancelled => "user_cancelled".to_string(),
        TurnAbortReason::Error(message) => format!("error:{message}"),
        TurnAbortReason::Timeout => "timeout".to_string(),
        TurnAbortReason::TokenLimitExceeded => "token_limit_exceeded".to_string(),
    }
}

fn error_code(error: &AgentError) -> String {
    match error {
        AgentError::Model(_) => "model_error",
        AgentError::Tool(_) => "tool_error",
        AgentError::Mcp(_) => "mcp_error",
        AgentError::Session(_) => "session_error",
        AgentError::NotImplemented(_) => "not_implemented",
        AgentError::InvalidConfig(_) => "invalid_config",
    }
    .to_string()
}

fn value_to_text(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        _ => serde_json::to_string_pretty(value).unwrap_or_else(|_| value.to_string()),
    }
}

fn load_runtime_config() -> AgentRuntimeConfig {
    let provider_env = std::env::var("AGENT_PROVIDER")
        .unwrap_or_else(|_| "openai".to_string())
        .to_lowercase();
    let provider = parse_provider(&provider_env);

    let mut config = AgentRuntimeConfig {
        provider,
        ..AgentRuntimeConfig::default()
    };
    config.model = default_model_for_provider(&config.provider).to_string();
    config.api_key_env = default_api_env_for_provider(&config.provider).to_string();

    if let Ok(model) = std::env::var("AGENT_MODEL") {
        config.model = model;
    }
    if let Ok(api_key_env) = std::env::var("AGENT_API_KEY_ENV") {
        config.api_key_env = api_key_env;
    }
    if let Ok(api_key) = std::env::var("AGENT_API_KEY") {
        config.api_key = normalize_optional(Some(api_key));
    }
    if let Ok(base_url) = std::env::var("AGENT_BASE_URL") {
        config.base_url = normalize_optional(Some(base_url));
    }
    if let Ok(max_tokens) = std::env::var("AGENT_MAX_TOKENS") {
        config.max_tokens = max_tokens.parse::<u32>().ok();
    }
    if let Ok(system_prompt) = std::env::var("AGENT_SYSTEM_PROMPT") {
        config.system_prompt = system_prompt;
    }

    if let Ok(mcp_config_json) = std::env::var("AGENT_MCP_CONFIG_JSON") {
        match serde_json::from_str::<McpRuntimeConfig>(&mcp_config_json) {
            Ok(mcp_config) => {
                config.mcp = mcp_config;
            }
            Err(err) => {
                log::warn!("failed to parse AGENT_MCP_CONFIG_JSON: {}", err);
            }
        }
    }

    if let Ok(skills_config_json) = std::env::var("AGENT_SKILLS_CONFIG_JSON") {
        match serde_json::from_str::<SkillsRuntimeConfig>(&skills_config_json) {
            Ok(skills_config) => {
                config.skills = skills_config;
            }
            Err(err) => {
                log::warn!("failed to parse AGENT_SKILLS_CONFIG_JSON: {}", err);
            }
        }
    }

    config
}

fn build_model_client(config: &AgentRuntimeConfig) -> Result<Arc<dyn ModelClient>, AppError> {
    let model = config.model.clone();
    let base_url = normalize_optional(config.base_url.clone());

    if matches!(config.max_tokens, Some(0)) {
        return Err(AppError::InvalidConfig(
            "max_tokens must be greater than 0".to_string(),
        ));
    }

    let provider: Arc<dyn ModelClient> = match config.provider {
        AgentProvider::OpenAi => {
            let api_key = resolve_api_key(config)?;
            Arc::new(OpenAiProvider::new(model).with_api_key(api_key))
        }
        AgentProvider::Glm => {
            let api_key = resolve_api_key(config)?;
            let provider = if let Some(url) = base_url.clone() {
                GlmProvider::new(model, api_key).with_base_url(url)
            } else {
                GlmProvider::new(model, api_key)
            };
            Arc::new(provider)
        }
        AgentProvider::GlmCoding => {
            let api_key = resolve_api_key(config)?;
            let provider = if let Some(url) = base_url.clone() {
                GlmCodingPlanProvider::new(model, api_key).with_base_url(url)
            } else {
                GlmCodingPlanProvider::new(model, api_key)
            };
            Arc::new(provider)
        }
        AgentProvider::Anthropic => {
            let api_key = resolve_api_key(config)?;
            let mut provider = AnthropicProvider::new(model).with_api_key(api_key);
            if let Some(url) = base_url {
                provider = provider.with_base_url(url);
            }
            if let Some(max_tokens) = config.max_tokens {
                provider = provider.with_max_tokens(max_tokens);
            }
            Arc::new(provider)
        }
        AgentProvider::Local => {
            let provider = if let Some(url) = base_url {
                LocalProvider::new(model).with_base_url(url)
            } else {
                LocalProvider::new(model)
            };
            Arc::new(provider)
        }
    };

    Ok(provider)
}

fn chunk_text(text: &str, max_chars: usize) -> Vec<String> {
    if text.is_empty() {
        return vec![String::new()];
    }
    let mut chunks = Vec::new();
    let mut current = String::new();
    for ch in text.chars() {
        current.push(ch);
        if current.chars().count() >= max_chars {
            chunks.push(current.clone());
            current.clear();
        }
    }
    if !current.is_empty() {
        chunks.push(current);
    }
    chunks
}

#[async_trait::async_trait]
pub trait Runner: Send + Sync {
    async fn run(&self, prompt: &str) -> AgentResult<String>;
}

struct AgentLibRunner {
    agent: agent_lib::Agent,
}

impl AgentLibRunner {
    fn new(config: AgentRuntimeConfig) -> Result<Self, AppError> {
        let model = config.model.clone();
        let base_url = normalize_optional(config.base_url.clone());
        if matches!(config.max_tokens, Some(0)) {
            return Err(AppError::InvalidConfig(
                "max_tokens must be greater than 0".to_string(),
            ));
        }

        let builder = match config.provider {
            AgentProvider::OpenAi => {
                let api_key = resolve_api_key(&config)?;
                let provider = OpenAiProvider::new(model).with_api_key(api_key);
                AgentBuilder::new().with_model(provider)
            }
            AgentProvider::Glm => {
                let api_key = resolve_api_key(&config)?;
                let provider = if let Some(url) = base_url.clone() {
                    GlmProvider::new(model, api_key).with_base_url(url)
                } else {
                    GlmProvider::new(model, api_key)
                };
                AgentBuilder::new().with_model(provider)
            }
            AgentProvider::GlmCoding => {
                let api_key = resolve_api_key(&config)?;
                let provider = if let Some(url) = base_url.clone() {
                    GlmCodingPlanProvider::new(model, api_key).with_base_url(url)
                } else {
                    GlmCodingPlanProvider::new(model, api_key)
                };
                AgentBuilder::new().with_model(provider)
            }
            AgentProvider::Anthropic => {
                let api_key = resolve_api_key(&config)?;
                let mut provider = AnthropicProvider::new(model).with_api_key(api_key);
                if let Some(url) = base_url {
                    provider = provider.with_base_url(url);
                }
                if let Some(max_tokens) = config.max_tokens {
                    provider = provider.with_max_tokens(max_tokens);
                }
                AgentBuilder::new().with_model(provider)
            }
            AgentProvider::Local => {
                let provider = if let Some(url) = base_url {
                    LocalProvider::new(model).with_base_url(url)
                } else {
                    LocalProvider::new(model)
                };
                AgentBuilder::new().with_model(provider)
            }
        };

        let agent = builder.with_instructions(config.system_prompt).build()?;
        Ok(Self { agent })
    }
}

#[async_trait::async_trait]
impl Runner for AgentLibRunner {
    async fn run(&self, prompt: &str) -> AgentResult<String> {
        self.agent.run(prompt).await
    }
}

struct FailingRunner {
    message: String,
}

#[async_trait::async_trait]
impl Runner for FailingRunner {
    async fn run(&self, _prompt: &str) -> AgentResult<String> {
        Err(AgentError::InvalidConfig(self.message.clone()))
    }
}

fn parse_provider(value: &str) -> AgentProvider {
    match value {
        "glm" => AgentProvider::Glm,
        "glm-coding" | "glm_coding" | "glmcoding" => AgentProvider::GlmCoding,
        "anthropic" => AgentProvider::Anthropic,
        "local" | "local-llm" | "local_llm" => AgentProvider::Local,
        _ => AgentProvider::OpenAi,
    }
}

fn default_model_for_provider(provider: &AgentProvider) -> &'static str {
    match provider {
        AgentProvider::OpenAi => "gpt-4o-mini",
        AgentProvider::Glm => "glm-5",
        AgentProvider::GlmCoding => "glm-5-coding",
        AgentProvider::Anthropic => "claude-3-5-sonnet-latest",
        AgentProvider::Local => "qwen2.5-coder:7b",
    }
}

fn default_api_env_for_provider(provider: &AgentProvider) -> &'static str {
    match provider {
        AgentProvider::OpenAi => "OPENAI_API_KEY",
        AgentProvider::Glm | AgentProvider::GlmCoding => "GLM_API_KEY",
        AgentProvider::Anthropic => "ANTHROPIC_API_KEY",
        AgentProvider::Local => "LOCAL_API_KEY",
    }
}

fn resolve_api_key(config: &AgentRuntimeConfig) -> Result<String, AppError> {
    if let Some(api_key) = normalize_optional(config.api_key.clone()) {
        return Ok(api_key);
    }

    if config.api_key_env.trim().is_empty() {
        return Err(AppError::InvalidConfig(
            "api_key_env cannot be empty".to_string(),
        ));
    }

    std::env::var(&config.api_key_env)
        .map_err(|_| AppError::MissingEnv(config.api_key_env.clone()))
        .and_then(|value| {
            normalize_optional(Some(value))
                .ok_or_else(|| AppError::MissingEnv(config.api_key_env.clone()))
        })
}

fn normalize_optional(value: Option<String>) -> Option<String> {
    value.and_then(|item| {
        let trimmed = item.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_string())
        }
    })
}
