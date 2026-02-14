pub mod types;

use std::collections::HashMap;
use std::sync::Arc;

use agent_lib::model::provider::{
    AnthropicProvider, GlmCodingPlanProvider, GlmProvider, LocalProvider, OpenAiProvider,
};
use agent_lib::{AgentBuilder, AgentResult};
use tauri::async_runtime::{JoinHandle, Mutex};

use self::types::{AgentEvent, AgentProvider, AgentRuntimeConfig, AppError};

#[derive(Clone)]
pub struct AgentService {
    runner: Arc<dyn Runner>,
    tasks: Arc<Mutex<HashMap<String, JoinHandle<()>>>>,
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
        let runner = AgentLibRunner::new(config)?;
        Ok(Self {
            runner: Arc::new(runner),
            tasks: Arc::new(Mutex::new(HashMap::new())),
        })
    }

    pub fn with_runner(runner: Arc<dyn Runner>) -> Self {
        Self {
            runner,
            tasks: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    pub async fn chat(&self, input: String) -> Result<String, AppError> {
        self.runner.run(&input).await.map_err(Into::into)
    }

    pub async fn chat_stream<F>(
        &self,
        task_id: String,
        input: String,
        mut emit: F,
    ) -> Result<(), AppError>
    where
        F: FnMut(AgentEvent) + Send + 'static,
    {
        emit(AgentEvent::Started {
            task_id: task_id.clone(),
        });

        let runner = Arc::clone(&self.runner);
        let tasks = Arc::clone(&self.tasks);
        let task_key = task_id.clone();
        let task_id_for_worker = task_id.clone();

        let handle = tauri::async_runtime::spawn(async move {
            match runner.run(&input).await {
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

        self.tasks.lock().await.insert(task_id, handle);
        Ok(())
    }

    pub async fn cancel(&self, task_id: &str) -> Result<(), AppError> {
        let mut tasks = self.tasks.lock().await;
        if let Some(handle) = tasks.remove(task_id) {
            handle.abort();
            Ok(())
        } else {
            Err(AppError::TaskNotFound(task_id.to_string()))
        }
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
        Err(agent_lib::AgentError::InvalidConfig(self.message.clone()))
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
