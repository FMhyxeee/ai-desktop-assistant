use serde::{Deserialize, Serialize};
use serde_json::Value;
use thiserror::Error;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentProvider {
    OpenAi,
    Glm,
    GlmCoding,
    Anthropic,
    Local,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentRuntimeConfig {
    pub provider: AgentProvider,
    pub model: String,
    pub api_key_env: String,
    pub api_key: Option<String>,
    pub base_url: Option<String>,
    pub max_tokens: Option<u32>,
    pub system_prompt: String,
}

impl Default for AgentRuntimeConfig {
    fn default() -> Self {
        Self {
            provider: AgentProvider::OpenAi,
            model: "gpt-4o-mini".to_string(),
            api_key_env: "OPENAI_API_KEY".to_string(),
            api_key: None,
            base_url: None,
            max_tokens: None,
            system_prompt: "You are a desktop AI assistant.".to_string(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum StreamInputKind {
    Text,
    Command,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentStreamInput {
    pub kind: StreamInputKind,
    pub content: String,
}

impl AgentStreamInput {
    pub fn text(content: impl Into<String>) -> Self {
        Self {
            kind: StreamInputKind::Text,
            content: content.into(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ProtocolOpPayload {
    UserTurn {
        model: String,
        cwd: String,
        approval_policy: String,
        sandbox_policy: String,
        text: String,
    },
    RunUserShellCommand {
        command: String,
    },
    Interrupt,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct ProtocolTokenUsage {
    pub prompt_tokens: u32,
    pub completion_tokens: u32,
    pub total_tokens: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ProtocolEventPayload {
    TurnStarted {
        turn_id: String,
    },
    ModelStreaming {
        chunk: String,
    },
    ModelComplete {
        content: String,
        usage: ProtocolTokenUsage,
    },
    ToolCallRequested {
        tool: String,
        args: Value,
    },
    ToolCallResult {
        tool: String,
        result: Value,
    },
    RunUserShellCommand {
        command: String,
    },
    Warning {
        message: String,
    },
    Error {
        code: String,
        message: String,
    },
    TurnAborted {
        reason: String,
    },
    TurnComplete {
        result: Value,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AgentEvent {
    Started {
        task_id: String,
    },
    Delta {
        task_id: String,
        chunk: String,
    },
    Completed {
        task_id: String,
        output: String,
    },
    Error {
        task_id: String,
        code: String,
        message: String,
    },
    OpSubmitted {
        task_id: String,
        seq: u64,
        payload: ProtocolOpPayload,
    },
    ProtocolEvent {
        task_id: String,
        seq: u64,
        payload: ProtocolEventPayload,
    },
}

#[derive(Debug, Error)]
pub enum AppError {
    #[error("missing environment variable: {0}")]
    MissingEnv(String),
    #[error("invalid config: {0}")]
    InvalidConfig(String),
    #[error("agent error: {0}")]
    Agent(String),
    #[error("task not found: {0}")]
    TaskNotFound(String),
}

impl From<agent_lib::AgentError> for AppError {
    fn from(value: agent_lib::AgentError) -> Self {
        Self::Agent(value.to_string())
    }
}
