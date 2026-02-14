pub mod types;

use std::collections::HashMap;
use std::sync::Arc;

use agent_lib::model::provider::{GlmProvider, OpenAiProvider};
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
    let provider = if provider_env == "glm" {
        AgentProvider::Glm
    } else {
        AgentProvider::OpenAi
    };
    let mut config = AgentRuntimeConfig {
        provider,
        ..AgentRuntimeConfig::default()
    };
    if let Ok(model) = std::env::var("AGENT_MODEL") {
        config.model = model;
    }
    if let Ok(api_key_env) = std::env::var("AGENT_API_KEY_ENV") {
        config.api_key_env = api_key_env;
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
        let api_key = std::env::var(&config.api_key_env)
            .map_err(|_| AppError::MissingEnv(config.api_key_env.clone()))?;

        let builder = match config.provider {
            AgentProvider::OpenAi => {
                let provider = OpenAiProvider::new(config.model).with_api_key(api_key);
                AgentBuilder::new().with_model(provider)
            }
            AgentProvider::Glm => {
                let provider = GlmProvider::new(config.model, api_key);
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
