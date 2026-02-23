use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
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
    #[serde(default = "default_true")]
    pub model_supports_image_input: bool,
    pub api_key_env: String,
    pub api_key: Option<String>,
    pub base_url: Option<String>,
    pub max_tokens: Option<u32>,
    #[serde(default)]
    pub workspace: WorkspaceRuntimeConfig,
    pub mcp: McpRuntimeConfig,
    pub skills: SkillsRuntimeConfig,
    #[serde(default)]
    pub memory: MemoryRuntimeConfig,
    #[serde(default)]
    pub control: AppControlRuntimeConfig,
}

impl Default for AgentRuntimeConfig {
    fn default() -> Self {
        Self {
            provider: AgentProvider::OpenAi,
            model: "gpt-4o-mini".to_string(),
            model_supports_image_input: true,
            api_key_env: "OPENAI_API_KEY".to_string(),
            api_key: None,
            base_url: None,
            max_tokens: None,
            workspace: WorkspaceRuntimeConfig::default(),
            mcp: McpRuntimeConfig::default(),
            skills: SkillsRuntimeConfig::default(),
            memory: MemoryRuntimeConfig::default(),
            control: AppControlRuntimeConfig::default(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceRuntimeConfig {
    #[serde(default)]
    pub root_dir: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppControlRuntimeConfig {
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default = "default_history_window_turns")]
    pub history_window_turns: usize,
    #[serde(default = "default_approval_timeout_secs")]
    pub approval_timeout_secs: u64,
    #[serde(default = "default_true")]
    pub model_fallback_enabled: bool,
    #[serde(default = "default_control_fallback_provider")]
    pub model_fallback_provider: AgentProvider,
    #[serde(default = "default_control_fallback_model")]
    pub model_fallback_model: String,
}

impl Default for AppControlRuntimeConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            history_window_turns: default_history_window_turns(),
            approval_timeout_secs: default_approval_timeout_secs(),
            model_fallback_enabled: true,
            model_fallback_provider: default_control_fallback_provider(),
            model_fallback_model: default_control_fallback_model(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MemoryRuntimeConfig {
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default = "default_memory_max_recall_items")]
    pub max_recall_items: usize,
    #[serde(default = "default_memory_max_non_secret_items")]
    pub max_non_secret_items: usize,
    #[serde(default = "default_memory_max_secret_items")]
    pub max_secret_items: usize,
    #[serde(default = "default_memory_secret_similarity_threshold")]
    pub secret_similarity_threshold: f32,
    #[serde(default = "default_true")]
    pub require_explicit_secret_intent: bool,
}

impl Default for MemoryRuntimeConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            max_recall_items: default_memory_max_recall_items(),
            max_non_secret_items: default_memory_max_non_secret_items(),
            max_secret_items: default_memory_max_secret_items(),
            secret_similarity_threshold: default_memory_secret_similarity_threshold(),
            require_explicit_secret_intent: true,
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum McpTransportKind {
    #[default]
    Stdio,
    StreamableHttp,
    // Legacy values kept only for reading older configs.
    Tcp,
    Http,
    Https,
    Websocket,
    Wss,
    Sse,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum McpAuthType {
    #[default]
    None,
    Bearer,
    Basic,
    ApiKey,
    #[serde(rename = "oauth2")]
    OAuth2,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct McpAuthRuntimeConfig {
    #[serde(rename = "type")]
    pub auth_type: McpAuthType,
    pub token_env: Option<String>,
    pub username_env: Option<String>,
    pub password_env: Option<String>,
    pub api_key_env: Option<String>,
    pub api_key_header: Option<String>,
    pub query_param: Option<String>,
    pub token_url: Option<String>,
    pub client_id_env: Option<String>,
    pub client_secret_env: Option<String>,
    pub scope: Option<String>,
    pub audience: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct McpTlsRuntimeConfig {
    pub ca_cert_path: Option<String>,
    pub client_cert_path: Option<String>,
    pub client_key_path: Option<String>,
    #[serde(default)]
    pub danger_accept_invalid_certs: bool,
    #[serde(default)]
    pub danger_accept_invalid_hostnames: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct McpServerRuntimeConfig {
    pub name: String,
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default)]
    pub transport: McpTransportKind,
    pub endpoint: Option<String>,
    pub command: Option<String>,
    #[serde(default)]
    pub args: Vec<String>,
    pub timeout_secs: Option<u64>,
    #[serde(default)]
    pub env: HashMap<String, String>,
    #[serde(default)]
    pub headers: HashMap<String, String>,
    pub auth: Option<McpAuthRuntimeConfig>,
    pub tls: Option<McpTlsRuntimeConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct McpRuntimeConfig {
    #[serde(default)]
    pub enabled: bool,
    pub default_timeout_secs: Option<u64>,
    pub max_retries: Option<usize>,
    #[serde(default)]
    pub servers: Vec<McpServerRuntimeConfig>,
    #[serde(default)]
    pub image_recognition: McpImageRecognitionRuntimeConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct McpImageRecognitionRuntimeConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub server_name: String,
    #[serde(default)]
    pub tool_name: String,
    #[serde(default = "default_image_recognition_args_template")]
    pub args_template: Value,
}

impl Default for McpImageRecognitionRuntimeConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            server_name: String::new(),
            tool_name: String::new(),
            args_template: default_image_recognition_args_template(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct SkillsRuntimeConfig {
    #[serde(default)]
    pub enabled: bool,
    pub personal_dir: Option<String>,
    #[serde(default)]
    pub project_dirs: Vec<String>,
    #[serde(default)]
    pub auto_apply: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentInputImage {
    pub name: String,
    pub mime_type: String,
    pub data_url: String,
    pub size_bytes: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentStreamInput {
    pub content: String,
    #[serde(default)]
    pub images: Vec<AgentInputImage>,
    #[serde(default)]
    pub conversation_id: Option<String>,
    #[serde(default)]
    pub recent_messages: Vec<AgentHistoryMessage>,
}

impl AgentStreamInput {
    pub fn text(content: impl Into<String>) -> Self {
        Self {
            content: content.into(),
            images: Vec::new(),
            conversation_id: None,
            recent_messages: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentHistoryRole {
    User,
    Assistant,
    System,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentHistoryMessage {
    pub role: AgentHistoryRole,
    pub content: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct ProtocolInputImage {
    pub name: String,
    pub mime_type: String,
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
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        images: Vec<ProtocolInputImage>,
    },
    RunUserShellCommand {
        command: String,
    },
    ProcCommand {
        command: String,
    },
    RunSubAgent {
        mode: String,
        input: String,
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
#[serde(rename_all = "camelCase")]
pub struct ProtocolProcessInfo {
    pub id: String,
    pub command: String,
    pub pid: Option<u32>,
    pub status: String,
    pub started_at_unix_ms: u64,
    pub exit_code: Option<i32>,
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
        #[serde(default, skip_serializing_if = "Option::is_none")]
        normalization: Option<ProtocolMcpArgNormalization>,
    },
    ToolCallResult {
        tool: String,
        result: Value,
    },
    RunUserShellCommand {
        command: String,
    },
    ProcessStarted {
        process: ProtocolProcessInfo,
    },
    ProcessList {
        processes: Vec<ProtocolProcessInfo>,
    },
    ProcessLogs {
        process_id: String,
        logs: Vec<String>,
    },
    ProcessStopped {
        process: ProtocolProcessInfo,
    },
    ProcessError {
        action: String,
        message: String,
    },
    ThinkStatus {
        active: bool,
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
    McpListToolsResponse {
        tools: Vec<ProtocolMcpToolInfo>,
    },
    McpListResourcesResponse {
        resources: Vec<ProtocolMcpResourceInfo>,
    },
    McpResourceContent {
        uri: String,
        content: String,
    },
    McpListPromptsResponse {
        prompts: Vec<ProtocolMcpPromptInfo>,
    },
    McpPromptResult {
        messages: Vec<ProtocolPromptMessage>,
    },
    ListSkillsResponse {
        skills: Vec<ProtocolSkillEntry>,
    },
    SkillContent {
        name: String,
        content: String,
        auxiliary_files: Vec<String>,
    },
    SkillApplied {
        name: String,
    },
    SkillFileContent {
        skill_name: String,
        file_path: String,
        content: String,
    },
    SubAgentStarted {
        mode: String,
        input: String,
    },
    SubAgentProgress {
        mode: String,
        message: String,
    },
    SubAgentCompleted {
        mode: String,
        output: String,
    },
    SubAgentFailed {
        mode: String,
        error: String,
    },
    ConversationTitleSuggestion {
        conversation_id: String,
        title: String,
    },
    ControlDecision {
        source: PatchDecisionSource,
        confidence: f32,
        summary: String,
        developer_instructions: Option<String>,
        patch: Option<ProtocolRuntimeConfigPatch>,
    },
    ConfigChangeRequest {
        request_id: String,
        summary: String,
        source: PatchDecisionSource,
        confidence: f32,
        patch: ProtocolRuntimeConfigPatch,
        expires_at_unix_ms: u64,
    },
    ConfigChangeResult {
        request_id: String,
        approved: bool,
        persisted: bool,
        applied: bool,
        reason: String,
        patch: Option<ProtocolRuntimeConfigPatch>,
    },
    GovernanceReport {
        report: GovernanceReport,
    },
    GuidanceContext {
        input: ProtocolGuidanceInput,
        output: ProtocolGuidanceOutput,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProtocolMcpRewriteEntry {
    pub field_path: String,
    pub reason: String,
    pub before: String,
    pub after: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProtocolMcpRejectPreview {
    pub field_path: String,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProtocolMcpArgNormalization {
    pub rewritten_count: usize,
    #[serde(default)]
    pub rewrites: Vec<ProtocolMcpRewriteEntry>,
    pub reject_preview: Option<ProtocolMcpRejectPreview>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProtocolGuidanceTemplate {
    pub template_type: String,
    pub target: String,
    pub payload: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProtocolGuidancePathVars {
    pub workspace_root: String,
    pub ah_dir: String,
    pub screenshots_dir: String,
    pub image_recognition_dir: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProtocolGuidanceToolConstraint {
    pub tool: String,
    pub server_name: String,
    pub image_tool: bool,
    #[serde(default)]
    pub image_fields: Vec<String>,
    #[serde(default)]
    pub path_fields: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct ProtocolGuidanceHitRules {
    #[serde(default)]
    pub signals: Vec<String>,
    #[serde(default)]
    pub selected_contracts: Vec<String>,
    #[serde(default)]
    pub selected_servers: Vec<String>,
    #[serde(default)]
    pub selected_prompts: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct ProtocolGuidanceGovernanceSummary {
    pub blocker_count: usize,
    pub warning_count: usize,
    pub info_count: usize,
    #[serde(default)]
    pub top_issue_codes: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct ProtocolGuidanceMemorySummary {
    pub enabled: bool,
    pub context_found: bool,
    pub non_secret_count: usize,
    pub secret_count: usize,
    #[serde(default)]
    pub source_tags: Vec<String>,
    #[serde(default)]
    pub redacted_lines: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProtocolGuidanceInput {
    pub user_input: String,
    pub workspace_root: String,
    pub mcp_contracts_total: usize,
    pub mcp_servers_total: usize,
    pub skills_total: usize,
    pub hit_rules: ProtocolGuidanceHitRules,
    pub governance: ProtocolGuidanceGovernanceSummary,
    pub memory: ProtocolGuidanceMemorySummary,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProtocolGuidanceOutput {
    pub system_prompt_fragment: String,
    pub path_vars: ProtocolGuidancePathVars,
    #[serde(default)]
    pub tool_constraints: Vec<ProtocolGuidanceToolConstraint>,
    #[serde(default)]
    pub templates: Vec<ProtocolGuidanceTemplate>,
    #[serde(default)]
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProtocolMcpToolInfo {
    pub name: String,
    pub description: String,
    pub server: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProtocolMcpResourceInfo {
    pub uri: String,
    pub name: String,
    pub description: Option<String>,
    pub mime_type: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProtocolPromptArgumentInfo {
    pub name: String,
    pub description: Option<String>,
    pub required: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProtocolMcpPromptInfo {
    pub name: String,
    pub description: Option<String>,
    pub arguments: Option<Vec<ProtocolPromptArgumentInfo>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ProtocolPromptContent {
    Text { text: String },
    Image { data: String, mime_type: String },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProtocolPromptMessage {
    pub role: String,
    pub content: ProtocolPromptContent,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProtocolSkillEntry {
    pub name: String,
    pub description: String,
    pub path: String,
    pub source: String,
    pub has_auxiliary_files: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PatchDecisionSource {
    Rule,
    ModelFallback,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProtocolRuntimeConfigPatch {
    #[serde(default)]
    pub mcp: Option<McpRuntimeConfig>,
    #[serde(default)]
    pub skills: Option<SkillsRuntimeConfig>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum GovernanceSeverity {
    Blocker,
    Warning,
    Info,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GovernanceIssue {
    pub severity: GovernanceSeverity,
    pub category: String,
    pub code: String,
    pub message: String,
    pub evidence: Option<String>,
    pub suggestion: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GovernanceReport {
    pub generated_at_unix_ms: u64,
    pub scope: String,
    pub summary: String,
    pub issues: Vec<GovernanceIssue>,
    pub blocker_count: usize,
    pub warning_count: usize,
    pub info_count: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GovernanceUpdateReport {
    pub before: Option<GovernanceReport>,
    pub after: GovernanceReport,
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

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct McpServerTestResult {
    pub name: String,
    pub enabled: bool,
    pub success: bool,
    pub latency_ms: u128,
    pub tool_count: usize,
    pub tools: Vec<String>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct McpConfigTestResult {
    pub success: bool,
    pub message: String,
    pub server_results: Vec<McpServerTestResult>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SkillScanEntry {
    pub name: String,
    pub description: String,
    pub path: String,
    pub source: String,
    pub has_auxiliary_files: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SkillScanResult {
    pub success: bool,
    pub message: String,
    pub warnings: Vec<String>,
    pub skills: Vec<SkillScanEntry>,
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

fn default_true() -> bool {
    true
}

fn default_history_window_turns() -> usize {
    8
}

fn default_approval_timeout_secs() -> u64 {
    60
}

fn default_control_fallback_provider() -> AgentProvider {
    AgentProvider::Local
}

fn default_control_fallback_model() -> String {
    "qwen2.5-coder:7b".to_string()
}

fn default_memory_max_recall_items() -> usize {
    8
}

fn default_memory_max_non_secret_items() -> usize {
    3
}

fn default_memory_max_secret_items() -> usize {
    2
}

fn default_memory_secret_similarity_threshold() -> f32 {
    0.82
}

fn default_image_recognition_args_template() -> Value {
    serde_json::json!({
        "image": "{{data_url}}",
    })
}
