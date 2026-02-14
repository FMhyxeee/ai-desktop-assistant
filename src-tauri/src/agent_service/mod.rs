pub mod types;

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use agent_lib::model::provider::{
    AnthropicProvider, GlmCodingPlanProvider, GlmProvider, LocalProvider, OpenAiProvider,
};
use agent_lib::model::{ModelClient, TokenUsage};
use agent_lib::protocol::{ApprovalPolicy, Op, ReasoningSummary, SandboxPolicy, UserInputItem};
use agent_lib::session::{Session, SessionConfig, SessionHandle};
use agent_lib::{AgentBuilder, AgentError, AgentResult, Event, TurnAbortReason};
use serde_json::Value;
use tauri::async_runtime::{JoinHandle, Mutex};
use tokio::time::timeout;

use self::types::{
    AgentEvent, AgentProvider, AgentRuntimeConfig, AgentStreamInput, AppError, ProtocolEventPayload,
    ProtocolOpPayload, ProtocolTokenUsage, StreamInputKind,
};

struct ActiveTask {
    join_handle: JoinHandle<()>,
    session_handle: Option<SessionHandle>,
}

#[derive(Clone)]
pub struct AgentService {
    runner: Arc<dyn Runner>,
    runtime_config: Option<AgentRuntimeConfig>,
    tasks: Arc<Mutex<HashMap<String, ActiveTask>>>,
}

impl AgentService {
    pub fn from_env() -> Result<Self, AppError> {
        let config = load_runtime_config();
        match Self::new_with_config(config) {
            Ok(service) => Ok(service),
            Err(err) => Ok(Self::with_runner(Arc::new(FailingRunner {
                message: err.to_string(),
            }))),
        }
    }

    pub fn new_with_config(config: AgentRuntimeConfig) -> Result<Self, AppError> {
        let runner = AgentLibRunner::new(config.clone())?;
        Ok(Self {
            runner: Arc::new(runner),
            runtime_config: Some(config),
            tasks: Arc::new(Mutex::new(HashMap::new())),
        })
    }

    pub fn with_runner(runner: Arc<dyn Runner>) -> Self {
        Self {
            runner,
            runtime_config: None,
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
