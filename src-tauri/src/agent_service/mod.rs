mod control;
mod governance;
pub mod types;

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use agent_lib::mcp::{
    AuthConfig as AgentMcpAuthConfig, AuthType as AgentMcpAuthType, CallToolRequestParams,
    McpClient, McpManager, ServerConfig as AgentMcpServerConfig, TlsConfig as AgentMcpTlsConfig,
    TransportType as AgentMcpTransportType,
};
use agent_lib::model::provider::{
    AnthropicProvider, GlmCodingPlanProvider, GlmProvider, LocalProvider, OpenAiProvider,
};
use agent_lib::model::{Message, ModelClient, TokenUsage};
use agent_lib::protocol::{
    ApprovalPolicy, Op, PromptDirectives, ReasoningSummary, SandboxPolicy, UserInputItem,
};
use agent_lib::session::{Session, SessionConfig, SessionHandle};
use agent_lib::skills::{SkillConfig, SkillLoader, SkillSource};
use agent_lib::tools::builtin::{CodeExecTool, FileSystemTool, NetworkTool, ShellTool};
use agent_lib::tools::{Tool, ToolContext, ToolDef, ToolExecutor, ToolRegistry, ToolResult};
use agent_lib::{AgentBuilder, AgentError, AgentResult, Event, TurnAbortReason};
use base64::Engine;
use serde_json::{Map, Value};
use tauri::async_runtime::{JoinHandle, Mutex};
use tokio::sync::oneshot;
use tokio::time::timeout;

use self::control::{ControlInput, RuntimeConfigPatch};
use self::types::{
    AgentEvent, AgentHistoryMessage, AgentHistoryRole, AgentInputImage, AgentProvider,
    AgentRuntimeConfig, AgentStreamInput, AppError, GovernanceReport, McpAuthRuntimeConfig,
    McpAuthType, McpConfigTestResult, McpRuntimeConfig, McpServerRuntimeConfig,
    McpServerTestResult, McpTlsRuntimeConfig, McpTransportKind, ProtocolEventPayload,
    ProtocolInputImage, ProtocolMcpPromptInfo, ProtocolMcpResourceInfo, ProtocolMcpToolInfo,
    ProtocolOpPayload, ProtocolPromptArgumentInfo, ProtocolPromptContent, ProtocolPromptMessage,
    ProtocolSkillEntry, ProtocolTokenUsage, SkillScanEntry, SkillScanResult, SkillsRuntimeConfig,
};

struct ActiveTask {
    join_handle: JoinHandle<()>,
    session_handle: Option<SessionHandle>,
}

#[derive(Debug, Clone, Default)]
struct RuntimeConfigPatchAppliedState {
    system_prompt: Option<String>,
    mcp: Option<McpRuntimeConfig>,
    skills: Option<SkillsRuntimeConfig>,
}

impl RuntimeConfigPatchAppliedState {
    fn merge_patch(&mut self, patch: &RuntimeConfigPatch) {
        if let Some(system_prompt) = patch.system_prompt.clone() {
            self.system_prompt = Some(system_prompt);
        }
        if let Some(mcp) = patch.mcp.clone() {
            self.mcp = Some(mcp);
        }
        if let Some(skills) = patch.skills.clone() {
            self.skills = Some(skills);
        }
    }
}

struct PendingConfigRequest {
    task_id: String,
    response_tx: oneshot::Sender<ConfigChangeApproval>,
}

#[derive(Debug, Clone, Copy)]
struct ConfigChangeApproval {
    approved: bool,
    persist: bool,
}

#[derive(Clone)]
pub struct AgentService {
    runner: Arc<dyn Runner>,
    runtime_config: Option<AgentRuntimeConfig>,
    latest_governance_report: Option<GovernanceReport>,
    tasks: Arc<Mutex<HashMap<String, ActiveTask>>>,
    conversation_overrides: Arc<Mutex<HashMap<String, RuntimeConfigPatchAppliedState>>>,
    global_persisted_patch: Arc<Mutex<RuntimeConfigPatchAppliedState>>,
    pending_config_requests: Arc<Mutex<HashMap<String, PendingConfigRequest>>>,
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
        let _runtime_resources = build_runtime_resources(&config).await?;
        let latest_governance_report = Some(governance::scan_runtime_governance(&config).await);
        Ok(Self {
            runner: Arc::new(runner),
            runtime_config: Some(config),
            latest_governance_report,
            tasks: Arc::new(Mutex::new(HashMap::new())),
            conversation_overrides: Arc::new(Mutex::new(HashMap::new())),
            global_persisted_patch: Arc::new(Mutex::new(RuntimeConfigPatchAppliedState::default())),
            pending_config_requests: Arc::new(Mutex::new(HashMap::new())),
        })
    }

    pub fn with_runner(runner: Arc<dyn Runner>) -> Self {
        Self {
            runner,
            runtime_config: None,
            latest_governance_report: None,
            tasks: Arc::new(Mutex::new(HashMap::new())),
            conversation_overrides: Arc::new(Mutex::new(HashMap::new())),
            global_persisted_patch: Arc::new(Mutex::new(RuntimeConfigPatchAppliedState::default())),
            pending_config_requests: Arc::new(Mutex::new(HashMap::new())),
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
            self.chat_stream_protocol(task_id, input, config, emit)
                .await
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
        let conversation_id = normalize_optional(input.conversation_id.clone());
        let mut effective_config = self
            .build_effective_runtime_config(&config, conversation_id.as_deref())
            .await;
        let history_window_turns = effective_config.control.history_window_turns.max(1);
        let recent_history = trim_recent_history(&input.recent_messages, history_window_turns);
        let control_input = ControlInput {
            user_input: input.content.clone(),
            recent_messages: recent_history.clone(),
            effective_config: effective_config.clone(),
        };
        let control_fallback_model = if effective_config.control.model_fallback_enabled {
            build_control_fallback_model_client(&effective_config).ok()
        } else {
            None
        };

        let mut seq = 0_u64;
        let baseline_governance = governance::scan_runtime_governance(&effective_config).await;
        seq += 1;
        emit(AgentEvent::ProtocolEvent {
            task_id: task_id.clone(),
            seq,
            payload: ProtocolEventPayload::GovernanceReport {
                report: baseline_governance,
            },
        });

        if let Some(conversation_id_value) = conversation_id.as_ref() {
            if let Some(title) =
                control::suggest_conversation_title(&control_input, control_fallback_model.clone())
                    .await
            {
                seq += 1;
                emit(AgentEvent::ProtocolEvent {
                    task_id: task_id.clone(),
                    seq,
                    payload: ProtocolEventPayload::ConversationTitleSuggestion {
                        conversation_id: conversation_id_value.clone(),
                        title,
                    },
                });
            }
        }

        let mut prompt_directives: Option<PromptDirectives> = None;

        if effective_config.control.enabled {
            let mut control_decision = control::evaluate_rules(&control_input);

            if control_decision.patch.is_none()
                && effective_config.control.model_fallback_enabled
                && control::should_try_model_fallback(&input.content)
            {
                if let Some(fallback_model) = control_fallback_model.clone() {
                    if let Some(fallback_decision) =
                        control::evaluate_model_fallback(&control_input, fallback_model).await
                    {
                        if fallback_decision.patch.is_some() && fallback_decision.confidence >= 0.7
                        {
                            control_decision = fallback_decision;
                        } else if control_decision.patch.is_none() {
                            control_decision.source = fallback_decision.source;
                            control_decision.confidence = fallback_decision.confidence;
                            control_decision.summary = format!(
                                "model fallback inspected intent but did not produce an actionable patch (confidence={:.2})",
                                fallback_decision.confidence
                            );
                            if fallback_decision.developer_instructions.is_some() {
                                control_decision.developer_instructions =
                                    fallback_decision.developer_instructions;
                            }
                        }
                    }
                } else {
                    seq += 1;
                    emit(AgentEvent::ProtocolEvent {
                        task_id: task_id.clone(),
                        seq,
                        payload: ProtocolEventPayload::Warning {
                            message: "control model fallback skipped because model init failed"
                                .to_string(),
                        },
                    });
                }
            }

            if control_decision.developer_instructions.is_none() {
                control_decision.developer_instructions =
                    Some(control::assemble_developer_instructions(
                        &effective_config,
                        control_decision.patch.as_ref(),
                    ));
            }

            seq += 1;
            emit(AgentEvent::ProtocolEvent {
                task_id: task_id.clone(),
                seq,
                payload: ProtocolEventPayload::ControlDecision {
                    source: control_decision.source.clone(),
                    confidence: control_decision.confidence,
                    summary: control_decision.summary.clone(),
                    developer_instructions: control_decision.developer_instructions.clone(),
                    patch: control_decision
                        .patch
                        .as_ref()
                        .map(RuntimeConfigPatch::to_protocol_patch),
                },
            });

            if let Some(patch) = control_decision.patch.clone() {
                let request_id = uuid::Uuid::new_v4().to_string();
                let mut candidate_config = effective_config.clone();
                apply_runtime_patch_to_config(&mut candidate_config, &patch);
                let candidate_report = governance::scan_runtime_governance(&candidate_config).await;

                if candidate_report.blocker_count > 0 {
                    seq += 1;
                    emit(AgentEvent::ProtocolEvent {
                        task_id: task_id.clone(),
                        seq,
                        payload: ProtocolEventPayload::ConfigChangeResult {
                            request_id,
                            approved: false,
                            persisted: false,
                            applied: false,
                            reason: format!(
                                "rejected by governance blockers (count={})",
                                candidate_report.blocker_count
                            ),
                            patch: Some(patch.to_protocol_patch()),
                        },
                    });
                    seq += 1;
                    emit(AgentEvent::ProtocolEvent {
                        task_id: task_id.clone(),
                        seq,
                        payload: ProtocolEventPayload::GovernanceReport {
                            report: candidate_report,
                        },
                    });
                } else {
                    let approval_timeout_secs =
                        effective_config.control.approval_timeout_secs.max(1);
                    let expires_at_unix_ms =
                        current_time_millis() + approval_timeout_secs.saturating_mul(1000);

                    seq += 1;
                    emit(AgentEvent::ProtocolEvent {
                        task_id: task_id.clone(),
                        seq,
                        payload: ProtocolEventPayload::ConfigChangeRequest {
                            request_id: request_id.clone(),
                            summary: control_decision.summary.clone(),
                            source: control_decision.source,
                            confidence: control_decision.confidence,
                            patch: patch.to_protocol_patch(),
                            expires_at_unix_ms,
                        },
                    });

                    let (approval, reason) = self
                        .wait_for_config_change_approval(
                            &task_id,
                            &request_id,
                            approval_timeout_secs,
                        )
                        .await;
                    let approved = approval.approved;
                    let persisted = approval.approved && approval.persist;
                    let mut applied = false;

                    if approved {
                        apply_runtime_patch_to_config(&mut effective_config, &patch);
                        applied = true;

                        if let Some(conversation_id) = conversation_id.as_deref() {
                            self.merge_conversation_override(conversation_id, &patch)
                                .await;
                        }
                        if persisted {
                            self.merge_global_override(&patch).await;
                            let updated_report =
                                governance::scan_runtime_governance(&effective_config).await;
                            seq += 1;
                            emit(AgentEvent::ProtocolEvent {
                                task_id: task_id.clone(),
                                seq,
                                payload: ProtocolEventPayload::GovernanceReport {
                                    report: updated_report,
                                },
                            });
                        }
                    }

                    seq += 1;
                    emit(AgentEvent::ProtocolEvent {
                        task_id: task_id.clone(),
                        seq,
                        payload: ProtocolEventPayload::ConfigChangeResult {
                            request_id,
                            approved,
                            persisted,
                            applied,
                            reason,
                            patch: Some(patch.to_protocol_patch()),
                        },
                    });
                }
            }

            prompt_directives = Some(PromptDirectives {
                developer_instructions: Some(control::assemble_developer_instructions(
                    &effective_config,
                    None,
                )),
                user_instructions: None,
            });
        }

        let runtime_resources = build_runtime_resources(&effective_config).await?;
        let model = build_model_client(&effective_config)?;
        let default_cwd = std::env::current_dir()
            .unwrap_or_else(|_| PathBuf::from("."))
            .to_string_lossy()
            .to_string();
        let session_config = SessionConfig {
            model: Some(model),
            default_model: effective_config.model.clone(),
            default_cwd: Some(default_cwd),
            default_approval_policy: Some(ApprovalPolicy::NeverAsk),
            mcp_manager: runtime_resources.mcp_manager.clone(),
            tool_executor: runtime_resources.tool_executor.clone(),
            skill_config: Some(runtime_resources.skill_config.clone()),
            ..Default::default()
        };

        let (session, handle) = Session::with_config(64, session_config);
        for message in recent_history
            .iter()
            .filter_map(agent_history_to_session_message)
        {
            session.push_message(message).await;
        }

        let control_handle = handle.clone();
        let prepared_payload = prepare_image_fallback_payload(
            &input,
            &effective_config,
            runtime_resources.mcp_manager.clone(),
        )
        .await;
        let (op, op_payload, is_command_input) = build_stream_op(
            &input,
            &effective_config,
            prepared_payload.model_payload_text,
            prompt_directives,
        );

        for warning in prepared_payload.warnings {
            seq += 1;
            emit(AgentEvent::ProtocolEvent {
                task_id: task_id.clone(),
                seq,
                payload: ProtocolEventPayload::Warning { message: warning },
            });
        }

        seq += 1;
        emit(AgentEvent::OpSubmitted {
            task_id: task_id.clone(),
            seq,
            payload: op_payload,
        });

        handle.submit(op).await?;

        let tasks = Arc::clone(&self.tasks);
        let task_key = task_id.clone();
        let task_id_for_worker = task_id.clone();
        let worker_handle = handle;

        let join_handle = tauri::async_runtime::spawn(async move {
            let mut seq = seq;
            let mut completed_sent = false;
            let mut aggregated_output = String::new();

            loop {
                let next_event =
                    timeout(Duration::from_secs(120), worker_handle.next_event()).await;
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
                            message: format!(
                                "Turn aborted: {}",
                                turn_abort_reason_to_text(&reason)
                            ),
                        });
                        break;
                    }
                    Event::Error { error } => {
                        let error_code_value = error_code(&error);
                        let lower_text = error.to_string().to_lowercase();
                        if error_code_value == "mcp_error"
                            || (error_code_value == "tool_error" && lower_text.contains("mcp"))
                        {
                            seq += 1;
                            emit(AgentEvent::ProtocolEvent {
                                task_id: task_id_for_worker.clone(),
                                seq,
                                payload: ProtocolEventPayload::Warning {
                                    message: "MCP execution failed. Suggestion: verify MCP server reachability or submit a reviewed config patch."
                                        .to_string(),
                                },
                            });
                        }
                        if error_code_value == "tool_error"
                            && (lower_text.contains("skill") || lower_text.contains("skills"))
                        {
                            seq += 1;
                            emit(AgentEvent::ProtocolEvent {
                                task_id: task_id_for_worker.clone(),
                                seq,
                                payload: ProtocolEventPayload::Warning {
                                    message: "Skills execution failed. Suggestion: verify skills paths and auto-apply settings; no automatic config mutation was performed."
                                        .to_string(),
                                },
                            });
                        }
                        emit(AgentEvent::Error {
                            task_id: task_id_for_worker.clone(),
                            code: error_code_value,
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

    pub fn latest_governance_report(&self) -> Option<GovernanceReport> {
        self.latest_governance_report.clone()
    }

    pub async fn run_governance_scan(
        &self,
        config_override: Option<AgentRuntimeConfig>,
    ) -> Result<GovernanceReport, AppError> {
        let mut config = if let Some(config) = config_override {
            config
        } else if let Some(config) = self.runtime_config.clone() {
            config
        } else {
            return Err(AppError::InvalidConfig(
                "runtime config is unavailable for governance scan".to_string(),
            ));
        };
        let global_state = self.global_persisted_patch.lock().await.clone();
        apply_patch_state_to_config(&mut config, &global_state);

        Ok(governance::scan_runtime_governance(&config).await)
    }

    pub async fn cancel(&self, task_id: &str) -> Result<(), AppError> {
        let mut pending = self.pending_config_requests.lock().await;
        let request_ids = pending
            .iter()
            .filter(|(_, item)| item.task_id == task_id)
            .map(|(request_id, _)| request_id.clone())
            .collect::<Vec<_>>();
        for request_id in request_ids {
            if let Some(request) = pending.remove(&request_id) {
                let _ = request.response_tx.send(ConfigChangeApproval {
                    approved: false,
                    persist: false,
                });
            }
        }
        drop(pending);

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

    pub async fn resolve_config_change_request(
        &self,
        task_id: &str,
        request_id: &str,
        approved: bool,
        persist: bool,
    ) -> Result<(), AppError> {
        let mut pending = self.pending_config_requests.lock().await;
        let Some(request) = pending.remove(request_id) else {
            return Err(AppError::TaskNotFound(format!(
                "pending config request not found: {}",
                request_id
            )));
        };
        if request.task_id != task_id {
            let expected_task_id = request.task_id.clone();
            pending.insert(request_id.to_string(), request);
            return Err(AppError::InvalidConfig(format!(
                "task id mismatch for config request: expected {}, got {}",
                expected_task_id, task_id
            )));
        }
        drop(pending);

        request
            .response_tx
            .send(ConfigChangeApproval { approved, persist })
            .map_err(|_| {
                AppError::TaskNotFound(format!("config request receiver dropped: {}", request_id))
            })
    }

    async fn build_effective_runtime_config(
        &self,
        base: &AgentRuntimeConfig,
        conversation_id: Option<&str>,
    ) -> AgentRuntimeConfig {
        let mut effective = base.clone();

        let global_state = self.global_persisted_patch.lock().await.clone();
        apply_patch_state_to_config(&mut effective, &global_state);

        if let Some(conversation_id) = conversation_id {
            let conversation_state = self
                .conversation_overrides
                .lock()
                .await
                .get(conversation_id)
                .cloned();
            if let Some(conversation_state) = conversation_state {
                apply_patch_state_to_config(&mut effective, &conversation_state);
            }
        }

        effective
    }

    async fn merge_conversation_override(&self, conversation_id: &str, patch: &RuntimeConfigPatch) {
        let mut overrides = self.conversation_overrides.lock().await;
        let entry = overrides
            .entry(conversation_id.to_string())
            .or_insert_with(RuntimeConfigPatchAppliedState::default);
        entry.merge_patch(patch);
    }

    async fn merge_global_override(&self, patch: &RuntimeConfigPatch) {
        let mut state = self.global_persisted_patch.lock().await;
        state.merge_patch(patch);
    }

    async fn wait_for_config_change_approval(
        &self,
        task_id: &str,
        request_id: &str,
        timeout_secs: u64,
    ) -> (ConfigChangeApproval, String) {
        let (response_tx, response_rx) = oneshot::channel::<ConfigChangeApproval>();
        self.pending_config_requests.lock().await.insert(
            request_id.to_string(),
            PendingConfigRequest {
                task_id: task_id.to_string(),
                response_tx,
            },
        );

        let wait_result = timeout(Duration::from_secs(timeout_secs.max(1)), response_rx).await;
        self.pending_config_requests.lock().await.remove(request_id);

        match wait_result {
            Ok(Ok(approval)) if approval.approved => (approval, "approved".to_string()),
            Ok(Ok(_)) => (
                ConfigChangeApproval {
                    approved: false,
                    persist: false,
                },
                "rejected by user".to_string(),
            ),
            Ok(Err(_)) => (
                ConfigChangeApproval {
                    approved: false,
                    persist: false,
                },
                "approval channel closed".to_string(),
            ),
            Err(_) => (
                ConfigChangeApproval {
                    approved: false,
                    persist: false,
                },
                "approval timeout".to_string(),
            ),
        }
    }
}

struct RuntimeResources {
    mcp_manager: Option<Arc<McpManager>>,
    tool_executor: Option<Arc<ToolExecutor>>,
    skill_config: SkillConfig,
}

pub async fn run_governance_scan_with_config(config: AgentRuntimeConfig) -> GovernanceReport {
    governance::scan_runtime_governance(&config).await
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
        match loader
            .load_from_directory(&dir, &SkillSource::Personal)
            .await
        {
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

async fn build_runtime_resources(
    config: &AgentRuntimeConfig,
) -> Result<RuntimeResources, AppError> {
    let skill_config = map_runtime_skill_config(&config.skills);
    let mut registry = ToolRegistry::new();
    registry.register(Arc::new(ShellTool::new()));
    registry.register(Arc::new(FileSystemTool::new()));
    registry.register(Arc::new(NetworkTool::new()));
    registry.register(Arc::new(CodeExecTool::new()));

    let mut mcp_manager = None;
    if config.mcp.enabled {
        let default_timeout_secs = config.mcp.default_timeout_secs.unwrap_or(30).max(1);
        let max_retries = config.mcp.max_retries.unwrap_or(3);
        let manager = McpManager::with_timeout_and_retries(
            Duration::from_secs(default_timeout_secs),
            max_retries,
        );

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
        mcp_manager = Some(manager);
    }

    Ok(RuntimeResources {
        mcp_manager,
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
        McpTransportKind::Tcp => Err(AppError::InvalidConfig(unsupported_transport_message(
            "tcp",
        ))),
        McpTransportKind::Websocket => Err(AppError::InvalidConfig(unsupported_transport_message(
            "websocket",
        ))),
        McpTransportKind::Wss => Err(AppError::InvalidConfig(unsupported_transport_message(
            "wss",
        ))),
        McpTransportKind::Sse => Err(AppError::InvalidConfig(unsupported_transport_message(
            "sse",
        ))),
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
        client_id: resolve_secret_env(
            config.client_id_env.as_deref(),
            server_name,
            "client_id_env",
        )?,
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
            let has_flow = auth.token_url.is_some()
                && auth.client_id.is_some()
                && auth.client_secret.is_some();
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

        let output = serde_json::to_value(result).map_err(|err| {
            AgentError::Tool(format!("Failed to encode MCP tool result: {}", err))
        })?;

        Ok(ToolResult { output })
    }
}

fn apply_patch_state_to_config(
    config: &mut AgentRuntimeConfig,
    state: &RuntimeConfigPatchAppliedState,
) {
    if let Some(system_prompt) = state.system_prompt.clone() {
        config.system_prompt = system_prompt;
    }
    if let Some(mcp) = state.mcp.clone() {
        config.mcp = mcp;
    }
    if let Some(skills) = state.skills.clone() {
        config.skills = skills;
    }
}

fn apply_runtime_patch_to_config(config: &mut AgentRuntimeConfig, patch: &RuntimeConfigPatch) {
    if let Some(system_prompt) = patch.system_prompt.clone() {
        config.system_prompt = system_prompt;
    }
    if let Some(mcp) = patch.mcp.clone() {
        config.mcp = mcp;
    }
    if let Some(skills) = patch.skills.clone() {
        config.skills = skills;
    }
}

fn trim_recent_history(
    messages: &[AgentHistoryMessage],
    history_window_turns: usize,
) -> Vec<AgentHistoryMessage> {
    let max_messages = history_window_turns
        .max(1)
        .saturating_mul(2)
        .max(history_window_turns);
    if messages.len() <= max_messages {
        return messages.to_vec();
    }
    messages[messages.len() - max_messages..].to_vec()
}

fn agent_history_to_session_message(message: &AgentHistoryMessage) -> Option<Message> {
    if message.content.trim().is_empty() {
        return None;
    }
    match message.role {
        AgentHistoryRole::User => Some(Message::user(message.content.clone())),
        AgentHistoryRole::Assistant => Some(Message::assistant(message.content.clone())),
        AgentHistoryRole::System => Some(Message::system(message.content.clone())),
    }
}

fn build_control_fallback_model_client(
    config: &AgentRuntimeConfig,
) -> Result<Arc<dyn ModelClient>, AppError> {
    let mut fallback_config = config.clone();
    let same_provider = std::mem::discriminant(&config.provider)
        == std::mem::discriminant(&config.control.model_fallback_provider);

    fallback_config.provider = config.control.model_fallback_provider.clone();
    fallback_config.model = normalize_optional(Some(config.control.model_fallback_model.clone()))
        .unwrap_or_else(|| default_model_for_provider(&fallback_config.provider).to_string());

    if !same_provider {
        fallback_config.api_key_env =
            default_api_env_for_provider(&fallback_config.provider).to_string();
        if !matches!(fallback_config.provider, AgentProvider::Local) {
            fallback_config.api_key = None;
        }
    }

    build_model_client(&fallback_config)
}

fn current_time_millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or_default()
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
        Event::McpResourceContent { uri, content } => {
            Some(ProtocolEventPayload::McpResourceContent {
                uri: uri.clone(),
                content: content.clone(),
            })
        }
        Event::McpListPromptsResponse { prompts } => {
            Some(ProtocolEventPayload::McpListPromptsResponse {
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
            })
        }
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
        Event::SkillApplied { name } => {
            Some(ProtocolEventPayload::SkillApplied { name: name.clone() })
        }
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

#[derive(Debug, Default)]
struct PreparedImageFallbackPayload {
    model_payload_text: Option<String>,
    warnings: Vec<String>,
}

async fn prepare_image_fallback_payload(
    input: &AgentStreamInput,
    config: &AgentRuntimeConfig,
    mcp_manager: Option<Arc<McpManager>>,
) -> PreparedImageFallbackPayload {
    let mut prepared = PreparedImageFallbackPayload::default();

    let ParsedStreamInput::Text(text) = parse_slash_input(&input.content) else {
        return prepared;
    };

    if input.images.is_empty() || config.model_supports_image_input {
        return prepared;
    }

    let fallback_note_text = append_image_note(&text, &input.images);
    let image_recognition = &config.mcp.image_recognition;
    if !image_recognition.enabled {
        prepared
            .warnings
            .push("Image recognition fallback is disabled; using image filename note.".to_string());
        prepared.model_payload_text = Some(fallback_note_text);
        return prepared;
    }

    let server_name = image_recognition.server_name.trim();
    let tool_name = image_recognition.tool_name.trim();
    if server_name.is_empty() || tool_name.is_empty() {
        prepared.warnings.push(
            "Image recognition fallback config missing serverName/toolName; using image filename note."
                .to_string(),
        );
        prepared.model_payload_text = Some(fallback_note_text);
        return prepared;
    }

    let Some(args_template) = image_recognition.args_template.as_object() else {
        prepared.warnings.push(
            "Image recognition argsTemplate must be a JSON object; using image filename note."
                .to_string(),
        );
        prepared.model_payload_text = Some(fallback_note_text);
        return prepared;
    };

    let Some(manager) = mcp_manager else {
        prepared
            .warnings
            .push("MCP manager is unavailable, skipping image recognition fallback.".to_string());
        prepared.model_payload_text = Some(fallback_note_text);
        return prepared;
    };

    let Some((client, tools)) = manager.get_server_info(server_name).await else {
        prepared.warnings.push(format!(
            "MCP server '{}' not found, skipping image recognition fallback.",
            server_name
        ));
        prepared.model_payload_text = Some(fallback_note_text);
        return prepared;
    };

    if !tools.iter().any(|tool| tool.name.to_string() == tool_name) {
        prepared.warnings.push(format!(
            "MCP tool '{}:{}' not found, skipping image recognition fallback.",
            server_name, tool_name
        ));
        prepared.model_payload_text = Some(fallback_note_text);
        return prepared;
    }

    prepared.warnings.push(format!(
        "Using MCP image recognition fallback via '{}:{}' for {} image(s).",
        server_name,
        tool_name,
        input.images.len()
    ));

    let mut recognized_sections = Vec::<(String, String)>::new();

    for image in &input.images {
        let local_path = match persist_image_for_recognition(image) {
            Ok(path) => Some(path.to_string_lossy().to_string()),
            Err(err) => {
                prepared.warnings.push(format!(
                    "Failed to persist image '{}' to local path for recognition: {}",
                    image.name, err
                ));
                None
            }
        };
        let rendered_template = render_image_recognition_args_template(
            &Value::Object(args_template.clone()),
            image,
            local_path.as_deref(),
        );
        let arguments = match rendered_template {
            Value::Object(arguments) => arguments,
            _ => {
                prepared.warnings.push(format!(
                    "Failed to render image recognition args template for '{}'.",
                    image.name
                ));
                continue;
            }
        };
        let mut arguments_value = Value::Object(arguments);
        if let Some(local_path_value) = local_path.as_deref() {
            let rewritten = rewrite_image_like_arguments_to_local_path(
                &mut arguments_value,
                &image.name,
                local_path_value,
            );
            if rewritten > 0 {
                prepared.warnings.push(format!(
                    "Rewrote {} image argument field(s) to local path for '{}': {}",
                    rewritten, image.name, local_path_value
                ));
            }
        }
        let arguments = match arguments_value {
            Value::Object(arguments) => arguments,
            _ => {
                prepared.warnings.push(format!(
                    "Image recognition args rendered to non-object for '{}'.",
                    image.name
                ));
                continue;
            }
        };

        match client
            .call_tool(CallToolRequestParams {
                meta: None,
                name: tool_name.to_string().into(),
                arguments: Some(arguments),
                task: None,
            })
            .await
        {
            Ok(result) => match serde_json::to_value(result) {
                Ok(value) => {
                    if let Some(text) = extract_text_from_call_tool_result(&value) {
                        recognized_sections.push((image.name.clone(), text));
                    } else {
                        prepared.warnings.push(format!(
                            "Image recognition returned no readable text for '{}'.",
                            image.name
                        ));
                    }
                }
                Err(err) => prepared.warnings.push(format!(
                    "Failed to parse image recognition result for '{}': {}",
                    image.name, err
                )),
            },
            Err(err) => prepared.warnings.push(format!(
                "Image recognition tool failed for '{}': {}",
                image.name, err
            )),
        }
    }

    if recognized_sections.is_empty() {
        prepared.warnings.push(
            "All image recognition attempts failed; falling back to image filename note."
                .to_string(),
        );
        prepared.model_payload_text = Some(fallback_note_text);
    } else {
        prepared.model_payload_text = Some(append_image_recognition_results(
            &text,
            &recognized_sections,
        ));
    }

    prepared
}

fn append_image_recognition_results(
    text: &str,
    recognized_sections: &[(String, String)],
) -> String {
    let mut lines = Vec::new();
    if !text.is_empty() {
        lines.push(text.to_string());
        lines.push(String::new());
    }

    lines.push("Image recognition results (via MCP):".to_string());
    for (name, recognized_text) in recognized_sections {
        lines.push(format!("- {}:", name));
        lines.push(recognized_text.clone());
    }
    lines.join("\n")
}

fn render_image_recognition_args_template(
    template: &Value,
    image: &AgentInputImage,
    local_path: Option<&str>,
) -> Value {
    match template {
        Value::String(value) => Value::String(replace_image_template_placeholders(
            value, image, local_path,
        )),
        Value::Array(items) => Value::Array(
            items
                .iter()
                .map(|item| render_image_recognition_args_template(item, image, local_path))
                .collect(),
        ),
        Value::Object(object) => {
            let mut next = Map::new();
            for (key, value) in object {
                next.insert(
                    key.clone(),
                    render_image_recognition_args_template(value, image, local_path),
                );
            }
            Value::Object(next)
        }
        _ => template.clone(),
    }
}

fn replace_image_template_placeholders(
    template: &str,
    image: &AgentInputImage,
    local_path: Option<&str>,
) -> String {
    let base64 = extract_base64_data_from_data_url(&image.data_url).unwrap_or_default();
    template
        .replace("{{data_url}}", &image.data_url)
        .replace("{{base64}}", &base64)
        .replace("{{mime_type}}", &image.mime_type)
        .replace("{{name}}", &image.name)
        .replace("{{path}}", local_path.unwrap_or_default())
}

fn extract_base64_data_from_data_url(data_url: &str) -> Option<String> {
    data_url
        .split_once(',')
        .map(|(_, payload)| payload.to_string())
}

fn persist_image_for_recognition(image: &AgentInputImage) -> Result<PathBuf, String> {
    let base64_payload = extract_base64_data_from_data_url(&image.data_url)
        .ok_or_else(|| "invalid data url: missing payload".to_string())?;
    let normalized_payload = base64_payload
        .chars()
        .filter(|ch| !ch.is_ascii_whitespace())
        .collect::<String>();
    let image_bytes = base64::engine::general_purpose::STANDARD
        .decode(&normalized_payload)
        .or_else(|_| base64::engine::general_purpose::URL_SAFE.decode(&normalized_payload))
        .map_err(|err| format!("base64 decode failed: {}", err))?;

    let directory = image_fallback_directory();
    std::fs::create_dir_all(&directory)
        .map_err(|err| format!("failed to create image fallback directory: {}", err))?;

    let safe_stem = sanitize_image_file_stem(&image.name);
    let extension = extension_from_mime_type(&image.mime_type);
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis())
        .unwrap_or_default();
    let filename = format!(
        "{safe_stem}-{timestamp}-{}.{}",
        uuid::Uuid::new_v4(),
        extension
    );

    let path = directory.join(filename);
    std::fs::write(&path, image_bytes)
        .map_err(|err| format!("failed to write image fallback file: {}", err))?;
    Ok(path)
}

fn image_fallback_directory() -> PathBuf {
    std::env::temp_dir()
        .join("ai-desktop-assistant")
        .join("image-recognition")
}

fn sanitize_image_file_stem(name: &str) -> String {
    let stem = Path::new(name)
        .file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or("image");
    let sanitized = stem
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' {
                ch
            } else {
                '_'
            }
        })
        .collect::<String>();

    if sanitized.trim_matches('_').is_empty() {
        "image".to_string()
    } else {
        sanitized
    }
}

fn extension_from_mime_type(mime_type: &str) -> &'static str {
    match mime_type {
        "image/png" => "png",
        "image/jpeg" | "image/jpg" => "jpg",
        "image/webp" => "webp",
        "image/gif" => "gif",
        "image/bmp" => "bmp",
        "image/tiff" => "tiff",
        _ => "bin",
    }
}

fn rewrite_image_like_arguments_to_local_path(
    value: &mut Value,
    original_name: &str,
    local_path: &str,
) -> usize {
    match value {
        Value::Object(map) => {
            let mut rewritten = 0_usize;
            for (key, item) in map {
                if is_image_like_field_name(key) {
                    if let Value::String(text) = item {
                        if is_bare_image_filename_value(text, original_name) {
                            *text = local_path.to_string();
                            rewritten += 1;
                            continue;
                        }
                    }
                }
                rewritten +=
                    rewrite_image_like_arguments_to_local_path(item, original_name, local_path);
            }
            rewritten
        }
        Value::Array(items) => items
            .iter_mut()
            .map(|item| rewrite_image_like_arguments_to_local_path(item, original_name, local_path))
            .sum(),
        _ => 0,
    }
}

fn is_image_like_field_name(key: &str) -> bool {
    let normalized = key.trim().to_ascii_lowercase();
    matches!(
        normalized.as_str(),
        "image" | "image_path" | "path" | "file" | "file_path" | "filepath"
    ) || normalized.contains("image")
        || normalized.contains("path")
}

fn is_bare_image_filename_value(value: &str, original_name: &str) -> bool {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return false;
    }

    if trimmed.eq_ignore_ascii_case(original_name.trim()) {
        return true;
    }

    // Bare filename should not include path separators or a drive prefix.
    let has_separator = trimmed.contains('/') || trimmed.contains('\\');
    let has_drive_prefix = trimmed.len() > 1 && trimmed.as_bytes()[1] == b':';
    if has_separator || has_drive_prefix {
        return false;
    }

    let ext = Path::new(trimmed)
        .extension()
        .and_then(|item| item.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();

    matches!(
        ext.as_str(),
        "png" | "jpg" | "jpeg" | "webp" | "gif" | "bmp" | "tiff" | "tif"
    )
}

fn extract_text_from_call_tool_result(result: &Value) -> Option<String> {
    if let Some(content_items) = result.get("content").and_then(Value::as_array) {
        let mut content_texts = Vec::new();
        for item in content_items {
            match item {
                Value::String(text) if !text.trim().is_empty() => {
                    content_texts.push(text.trim().to_string());
                }
                Value::Object(map) => {
                    if let Some(text) = map.get("text").and_then(Value::as_str) {
                        if !text.trim().is_empty() {
                            content_texts.push(text.trim().to_string());
                        }
                    }
                }
                _ => {}
            }
        }

        if !content_texts.is_empty() {
            return Some(content_texts.join("\n"));
        }
    }

    for key in [
        "structuredContent",
        "structured_content",
        "output",
        "result",
        "data",
    ] {
        if let Some(candidate) = result.get(key) {
            if let Some(text) = extract_text_from_json_value(candidate) {
                return Some(text);
            }
        }
    }

    extract_text_from_json_value(result)
}

fn extract_text_from_json_value(value: &Value) -> Option<String> {
    let mut chunks = Vec::<String>::new();
    collect_text_chunks(value, &mut chunks);
    if chunks.is_empty() {
        None
    } else {
        Some(chunks.join("\n"))
    }
}

fn collect_text_chunks(value: &Value, chunks: &mut Vec<String>) {
    match value {
        Value::String(text) => {
            let trimmed = text.trim();
            if !trimmed.is_empty() {
                chunks.push(trimmed.to_string());
            }
        }
        Value::Array(items) => {
            for item in items {
                collect_text_chunks(item, chunks);
            }
        }
        Value::Object(map) => {
            if let Some(text) = map.get("text").and_then(Value::as_str) {
                let trimmed = text.trim();
                if !trimmed.is_empty() {
                    chunks.push(trimmed.to_string());
                }
            }

            for (key, item) in map {
                if key == "text" {
                    continue;
                }
                collect_text_chunks(item, chunks);
            }
        }
        _ => {}
    }
}

fn build_stream_op(
    input: &AgentStreamInput,
    config: &AgentRuntimeConfig,
    prepared_model_payload_text: Option<String>,
    prompt_directives: Option<PromptDirectives>,
) -> (Op, ProtocolOpPayload, bool) {
    match parse_slash_input(&input.content) {
        ParsedStreamInput::Command(command) => (
            Op::RunUserShellCommand {
                command: command.clone(),
            },
            ProtocolOpPayload::RunUserShellCommand { command },
            true,
        ),
        ParsedStreamInput::Text(text) => {
            let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
            let protocol_images = input
                .images
                .iter()
                .map(|image| ProtocolInputImage {
                    name: image.name.clone(),
                    mime_type: image.mime_type.clone(),
                })
                .collect::<Vec<_>>();

            let model_payload_text = if let Some(prepared) = prepared_model_payload_text {
                prepared
            } else if input.images.is_empty() {
                text.clone()
            } else if config.model_supports_image_input
                && matches!(config.provider, AgentProvider::OpenAi)
            {
                encode_multimodal_text(&text, &input.images)
            } else {
                append_image_note(&text, &input.images)
            };

            (
                Op::UserTurn {
                    items: vec![UserInputItem::Text {
                        text: model_payload_text,
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
                    prompt_directives,
                    final_output_json_schema: None,
                    collaboration_mode: None,
                },
                ProtocolOpPayload::UserTurn {
                    model: config.model.clone(),
                    cwd: cwd.to_string_lossy().to_string(),
                    approval_policy: "never_ask".to_string(),
                    sandbox_policy: "persistent".to_string(),
                    text,
                    images: protocol_images,
                },
                false,
            )
        }
    }
}

#[derive(Debug)]
enum ParsedStreamInput {
    Text(String),
    Command(String),
}

const MULTIMODAL_MARKER: &str = "__AI_HELPER_MM_V1__";

fn parse_slash_input(raw_content: &str) -> ParsedStreamInput {
    let trimmed = raw_content.trim();
    if trimmed.is_empty() || !trimmed.starts_with('/') {
        return ParsedStreamInput::Text(trimmed.to_string());
    }

    if let Some(escaped) = trimmed.strip_prefix("//") {
        return ParsedStreamInput::Text(format!("/{escaped}"));
    }

    let command = trimmed.trim_start_matches('/').trim();
    if command.is_empty() {
        ParsedStreamInput::Text(trimmed.to_string())
    } else {
        ParsedStreamInput::Command(command.to_string())
    }
}

fn encode_multimodal_text(text: &str, images: &[AgentInputImage]) -> String {
    let payload = serde_json::json!({
        "text": text,
        "images": images
            .iter()
            .map(|image| {
                serde_json::json!({
                    "name": image.name,
                    "mime_type": image.mime_type,
                    "data_url": image.data_url,
                })
            })
            .collect::<Vec<_>>(),
    });

    format!("{MULTIMODAL_MARKER}{payload}")
}

fn append_image_note(text: &str, images: &[AgentInputImage]) -> String {
    let mut lines = Vec::new();
    if !text.is_empty() {
        lines.push(text.to_string());
        lines.push(String::new());
    }
    lines.push("User attached image files:".to_string());
    for image in images {
        lines.push(format!("- {} ({})", image.name, image.mime_type));
    }
    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slash_prefixed_text_becomes_command() {
        let parsed = parse_slash_input("/pwd");
        assert!(matches!(parsed, ParsedStreamInput::Command(command) if command == "pwd"));
    }

    #[test]
    fn double_slash_is_escaped_text() {
        let parsed = parse_slash_input("//pwd");
        assert!(matches!(parsed, ParsedStreamInput::Text(text) if text == "/pwd"));
    }

    #[test]
    fn build_stream_op_creates_shell_command_from_slash_input() {
        let input = AgentStreamInput {
            content: "/echo hello".to_string(),
            images: Vec::new(),
            conversation_id: None,
            recent_messages: Vec::new(),
        };
        let config = AgentRuntimeConfig::default();
        let (op, payload, is_command) = build_stream_op(&input, &config, None, None);

        assert!(is_command);
        assert!(matches!(
            op,
            Op::RunUserShellCommand { command } if command == "echo hello"
        ));
        assert!(matches!(
            payload,
            ProtocolOpPayload::RunUserShellCommand { command } if command == "echo hello"
        ));
    }

    #[test]
    fn build_stream_op_encodes_images_for_model_payload() {
        let input = AgentStreamInput {
            content: "describe this".to_string(),
            images: vec![AgentInputImage {
                name: "example.png".to_string(),
                mime_type: "image/png".to_string(),
                data_url: "data:image/png;base64,AAAA".to_string(),
                size_bytes: 4,
            }],
            conversation_id: None,
            recent_messages: Vec::new(),
        };
        let config = AgentRuntimeConfig::default();
        let (op, payload, is_command) = build_stream_op(&input, &config, None, None);

        assert!(!is_command);
        match op {
            Op::UserTurn { items, .. } => {
                assert_eq!(items.len(), 1);
                match &items[0] {
                    UserInputItem::Text { text } => {
                        assert!(text.starts_with(MULTIMODAL_MARKER));
                    }
                    _ => panic!("expected text item"),
                }
            }
            _ => panic!("expected user turn op"),
        }

        match payload {
            ProtocolOpPayload::UserTurn { text, images, .. } => {
                assert_eq!(text, "describe this");
                assert_eq!(images.len(), 1);
                assert_eq!(images[0].name, "example.png");
                assert_eq!(images[0].mime_type, "image/png");
            }
            _ => panic!("expected user turn payload"),
        }
    }

    #[test]
    fn build_stream_op_uses_prepared_model_payload_text() {
        let input = AgentStreamInput {
            content: "describe this".to_string(),
            images: vec![AgentInputImage {
                name: "example.png".to_string(),
                mime_type: "image/png".to_string(),
                data_url: "data:image/png;base64,AAAA".to_string(),
                size_bytes: 4,
            }],
            conversation_id: None,
            recent_messages: Vec::new(),
        };
        let mut config = AgentRuntimeConfig::default();
        config.model_supports_image_input = false;

        let (op, _payload, _is_command) =
            build_stream_op(&input, &config, Some("prepared text".to_string()), None);

        match op {
            Op::UserTurn { items, .. } => match &items[0] {
                UserInputItem::Text { text } => assert_eq!(text, "prepared text"),
                _ => panic!("expected text item"),
            },
            _ => panic!("expected user turn op"),
        }
    }

    #[test]
    fn render_image_recognition_args_template_replaces_placeholders() {
        let image = AgentInputImage {
            name: "sample.png".to_string(),
            mime_type: "image/png".to_string(),
            data_url: "data:image/png;base64,QUJD".to_string(),
            size_bytes: 4,
        };

        let template = serde_json::json!({
            "image": "{{data_url}}",
            "payload": {
                "base64": "{{base64}}",
                "mime": "{{mime_type}}",
                "name": "{{name}}",
                "path": "{{path}}"
            }
        });

        let rendered =
            render_image_recognition_args_template(&template, &image, Some("C:\\temp\\sample.png"));
        assert_eq!(rendered["image"], "data:image/png;base64,QUJD");
        assert_eq!(rendered["payload"]["base64"], "QUJD");
        assert_eq!(rendered["payload"]["mime"], "image/png");
        assert_eq!(rendered["payload"]["name"], "sample.png");
        assert_eq!(rendered["payload"]["path"], "C:\\temp\\sample.png");
    }

    #[test]
    fn extract_text_from_call_tool_result_prefers_content_text() {
        let result = serde_json::json!({
            "content": [
                { "type": "text", "text": "Line A" },
                { "type": "text", "text": "Line B" }
            ],
            "structuredContent": {
                "text": "Ignored fallback"
            }
        });

        let text = extract_text_from_call_tool_result(&result).unwrap_or_default();
        assert_eq!(text, "Line A\nLine B");
    }

    #[test]
    fn rewrite_image_like_arguments_to_local_path_updates_bare_filename() {
        let mut args = serde_json::json!({
            "image": "image.png",
            "nested": {
                "file_path": "image.png"
            }
        });

        let rewritten = rewrite_image_like_arguments_to_local_path(
            &mut args,
            "image.png",
            "C:\\temp\\image-123.png",
        );

        assert_eq!(rewritten, 2);
        assert_eq!(args["image"], "C:\\temp\\image-123.png");
        assert_eq!(args["nested"]["file_path"], "C:\\temp\\image-123.png");
    }

    #[tokio::test]
    async fn build_runtime_resources_without_mcp_registers_builtin_tools() {
        let config = AgentRuntimeConfig::default();

        let runtime = build_runtime_resources(&config)
            .await
            .expect("runtime resources should build");

        assert!(runtime.mcp_manager.is_none());

        let executor = runtime
            .tool_executor
            .expect("tool executor should be available");
        let names = executor
            .list()
            .into_iter()
            .map(|tool| tool.name)
            .collect::<Vec<_>>();

        assert!(names.iter().any(|name| name == "shell"));
        assert!(names.iter().any(|name| name == "filesystem"));
        assert!(names.iter().any(|name| name == "network"));
        assert!(names.iter().any(|name| name == "code_exec"));
    }

    #[tokio::test]
    async fn prepare_image_fallback_payload_without_mcp_manager_falls_back_to_image_note() {
        let input = AgentStreamInput {
            content: "describe this".to_string(),
            images: vec![AgentInputImage {
                name: "example.png".to_string(),
                mime_type: "image/png".to_string(),
                data_url: "data:image/png;base64,AAAA".to_string(),
                size_bytes: 4,
            }],
            conversation_id: None,
            recent_messages: Vec::new(),
        };

        let mut config = AgentRuntimeConfig::default();
        config.model_supports_image_input = false;
        config.mcp.image_recognition.enabled = true;
        config.mcp.image_recognition.server_name = "demo".to_string();
        config.mcp.image_recognition.tool_name = "ocr".to_string();

        let prepared = prepare_image_fallback_payload(&input, &config, None).await;
        let payload_text = prepared.model_payload_text.unwrap_or_default();

        assert!(payload_text.contains("User attached image files:"));
        assert!(!prepared.warnings.is_empty());
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
