mod control;
mod governance;
pub mod types;

use std::collections::{BTreeSet, HashMap};
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
    WorkspaceRuntimeConfig,
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

const AH_WORKSPACE_DIR_ENV: &str = "AH_WORKSPACE_DIR";
const AH_DIR_ENV: &str = "AH_DIR";
const AH_CONFIG_DIR_ENV: &str = "AH_CONFIG_DIR";
const AH_SCREENSHOTS_DIR_ENV: &str = "AH_SCREENSHOTS_DIR";
const AH_TMP_DIR_ENV: &str = "AH_TMP_DIR";
const AH_IMAGE_RECOGNITION_DIR_ENV: &str = "AH_IMAGE_RECOGNITION_DIR";

#[derive(Debug, Clone)]
struct WorkspaceContext {
    root_dir: PathBuf,
    ah_dir: PathBuf,
    config_dir: PathBuf,
    screenshots_dir: PathBuf,
    tmp_dir: PathBuf,
    image_recognition_dir: PathBuf,
    warnings: Vec<String>,
}

impl WorkspaceContext {
    fn from_root(root_dir: PathBuf) -> Self {
        let root_dir = normalize_path_lexical(&root_dir);
        let ah_dir = root_dir.join(".ah");
        let config_dir = ah_dir.join("config");
        let screenshots_dir = ah_dir.join("screenshots");
        let tmp_dir = ah_dir.join("tmp");
        let image_recognition_dir = ah_dir.join("image-recognition");

        Self {
            root_dir,
            ah_dir,
            config_dir,
            screenshots_dir,
            tmp_dir,
            image_recognition_dir,
            warnings: Vec::new(),
        }
    }

    fn resolve(config: &WorkspaceRuntimeConfig) -> Self {
        let mut warnings = Vec::new();
        let configured_root = config.root_dir.trim();
        let default_root = default_workspace_root();
        let fallback_cwd =
            absolute_path(&std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));

        let initial_root = if configured_root.is_empty() {
            default_root.clone()
        } else {
            absolute_path(&PathBuf::from(configured_root))
        };

        let mut context = Self::from_root(initial_root.clone());
        if let Err(err) = context.ensure_layout() {
            if configured_root.is_empty() {
                warnings.push(format!(
                    "Failed to initialize default workspace '{}' ({err}); falling back to current directory '{}'.",
                    initial_root.display(),
                    fallback_cwd.display()
                ));
            } else {
                warnings.push(format!(
                    "Failed to initialize configured workspace '{}' ({err}); falling back to default workspace '{}'.",
                    initial_root.display(),
                    default_root.display()
                ));
            }

            context = Self::from_root(default_root.clone());
            if let Err(default_err) = context.ensure_layout() {
                warnings.push(format!(
                    "Failed to initialize default workspace '{}' ({default_err}); falling back to current directory '{}'.",
                    default_root.display(),
                    fallback_cwd.display()
                ));

                context = Self::from_root(fallback_cwd.clone());
                if let Err(cwd_err) = context.ensure_layout() {
                    warnings.push(format!(
                        "Failed to initialize fallback workspace directories under '{}' ({cwd_err}).",
                        fallback_cwd.display()
                    ));
                }
            }
        }

        context.warnings = warnings;
        context
    }

    fn ensure_layout(&self) -> Result<(), String> {
        for dir in [
            &self.root_dir,
            &self.ah_dir,
            &self.config_dir,
            &self.screenshots_dir,
            &self.tmp_dir,
            &self.image_recognition_dir,
        ] {
            std::fs::create_dir_all(dir).map_err(|err| {
                format!("failed to create directory '{}': {}", dir.display(), err)
            })?;
        }
        Ok(())
    }

    fn root_dir_string(&self) -> String {
        self.root_dir.to_string_lossy().to_string()
    }

    fn inject_mcp_env(&self, env: &mut HashMap<String, String>) {
        env.insert(AH_WORKSPACE_DIR_ENV.to_string(), self.root_dir_string());
        env.insert(
            AH_DIR_ENV.to_string(),
            self.ah_dir.to_string_lossy().to_string(),
        );
        env.insert(
            AH_CONFIG_DIR_ENV.to_string(),
            self.config_dir.to_string_lossy().to_string(),
        );
        env.insert(
            AH_SCREENSHOTS_DIR_ENV.to_string(),
            self.screenshots_dir.to_string_lossy().to_string(),
        );
        env.insert(
            AH_TMP_DIR_ENV.to_string(),
            self.tmp_dir.to_string_lossy().to_string(),
        );
        env.insert(
            AH_IMAGE_RECOGNITION_DIR_ENV.to_string(),
            self.image_recognition_dir.to_string_lossy().to_string(),
        );
    }
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
        let workspace = WorkspaceContext::resolve(&config.workspace);
        for warning in &workspace.warnings {
            log::warn!("workspace: {}", warning);
        }
        let runner = AgentLibRunner::new(config.clone())?;
        let _runtime_resources = build_runtime_resources(&config, &workspace).await?;
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

        let workspace_context = WorkspaceContext::resolve(&effective_config.workspace);
        for warning in &workspace_context.warnings {
            seq += 1;
            emit(AgentEvent::ProtocolEvent {
                task_id: task_id.clone(),
                seq,
                payload: ProtocolEventPayload::Warning {
                    message: warning.clone(),
                },
            });
        }

        let runtime_resources =
            build_runtime_resources(&effective_config, &workspace_context).await?;
        let guidance_snapshot = collect_stage2_guidance_snapshot(&runtime_resources).await;
        let guidance_focus =
            build_stage2_guidance_focus(&input.content, &runtime_resources, &guidance_snapshot);
        let guidance_context = build_stage2_guidance_context(
            &workspace_context,
            &runtime_resources,
            &guidance_snapshot,
            &guidance_focus,
        );
        prompt_directives = Some(merge_prompt_directives_with_guidance(
            &effective_config,
            prompt_directives,
            &guidance_context,
        ));
        seq += 1;
        emit(AgentEvent::ProtocolEvent {
            task_id: task_id.clone(),
            seq,
            payload: ProtocolEventPayload::Warning {
                message: guidance_context_summary(
                    &workspace_context,
                    &runtime_resources,
                    &guidance_snapshot,
                    &guidance_focus,
                ),
            },
        });
        seq += 1;
        emit(AgentEvent::ProtocolEvent {
            task_id: task_id.clone(),
            seq,
            payload: ProtocolEventPayload::Warning {
                message: guidance_context_audit(
                    &runtime_resources,
                    &guidance_snapshot,
                    &guidance_focus,
                ),
            },
        });
        let model = build_model_client(&effective_config)?;
        let default_cwd = workspace_context.root_dir_string();
        let session_config = SessionConfig {
            model: Some(model),
            default_model: effective_config.model.clone(),
            default_cwd: Some(default_cwd.clone()),
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
            &workspace_context,
        )
        .await;
        let (op, op_payload, is_command_input) = build_stream_op(
            &input,
            &effective_config,
            prepared_payload.model_payload_text,
            prompt_directives,
            &workspace_context.root_dir,
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
        let workspace_root_for_protocol = workspace_context.root_dir.clone();

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

                if let Some(payload) =
                    map_protocol_event(&event, Some(&workspace_root_for_protocol))
                {
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

    pub fn current_workspace_root(&self) -> PathBuf {
        if let Some(config) = self.runtime_config.as_ref() {
            return WorkspaceContext::resolve(&config.workspace).root_dir;
        }
        WorkspaceContext::resolve(&WorkspaceRuntimeConfig::default()).root_dir
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
    mcp_tool_contracts: Vec<McpToolContract>,
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

    let workspace_context = WorkspaceContext::resolve(&WorkspaceRuntimeConfig::default());
    let fallback_base_dir =
        absolute_path(&std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));
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

        let server_config = match map_runtime_mcp_server_config(
            server,
            default_timeout_secs,
            &workspace_context,
            &fallback_base_dir,
        ) {
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

pub async fn scan_skills_runtime_config(
    config: SkillsRuntimeConfig,
    workspace: Option<WorkspaceRuntimeConfig>,
) -> SkillScanResult {
    if !config.enabled {
        return SkillScanResult {
            success: true,
            message: "Skills 已禁用".to_string(),
            warnings: vec![],
            skills: vec![],
        };
    }

    let workspace_runtime = workspace.unwrap_or_default();
    let workspace_context = WorkspaceContext::resolve(&workspace_runtime);
    let fallback_base_dir =
        absolute_path(&std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));
    let skill_config = map_runtime_skill_config(&config, &workspace_context, &fallback_base_dir);
    let mut warnings = Vec::new();
    let mut skills = Vec::new();
    let loader = SkillLoader::new();
    warnings.extend(workspace_context.warnings.clone());

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
    workspace_context: &WorkspaceContext,
) -> Result<RuntimeResources, AppError> {
    workspace_context
        .ensure_layout()
        .map_err(AppError::InvalidConfig)?;
    let fallback_base_dir =
        absolute_path(&std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));
    let skill_config =
        map_runtime_skill_config(&config.skills, workspace_context, &fallback_base_dir);
    let mut registry = ToolRegistry::new();
    registry.register(Arc::new(ShellTool::new()));
    registry.register(Arc::new(FileSystemTool::new()));
    registry.register(Arc::new(NetworkTool::new()));
    registry.register(Arc::new(CodeExecTool::new()));

    let mut mcp_manager = None;
    let mut mcp_tool_contracts = Vec::<McpToolContract>::new();
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

            let server_config = map_runtime_mcp_server_config(
                server,
                default_timeout_secs,
                workspace_context,
                &fallback_base_dir,
            )?;
            manager
                .add_server_with_config(server_config)
                .await
                .map_err(|err| AppError::InvalidConfig(err.to_string()))?;
        }

        let tools = manager.get_all_tools().await;
        for (server_name, tool_def, client) in tools {
            let call_name = tool_def.name.to_string();
            let schema = Value::Object((*tool_def.input_schema).clone());
            let contract = McpToolContract::compile(&server_name, &call_name, &schema);
            mcp_tool_contracts.push(contract.clone());
            let tool = PrefixedMcpTool::new(
                server_name,
                call_name,
                tool_def
                    .description
                    .as_deref()
                    .unwrap_or_default()
                    .to_string(),
                schema,
                contract,
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
        mcp_tool_contracts,
    })
}

#[derive(Debug, Clone, Default)]
struct Stage2GuidanceSnapshot {
    mcp_servers: Vec<Stage2McpServerSnapshot>,
    skills: Vec<Stage2SkillSnapshot>,
    warnings: Vec<String>,
}

#[derive(Debug, Clone)]
struct Stage2McpServerSnapshot {
    server_name: String,
    tool_count: usize,
    resource_count: usize,
    prompt_count: usize,
    resource_names: Vec<String>,
    prompts: Vec<Stage2McpPromptSnapshot>,
}

#[derive(Debug, Clone)]
struct Stage2McpPromptSnapshot {
    name: String,
    arguments: Vec<Stage2McpPromptArgumentSnapshot>,
}

#[derive(Debug, Clone)]
struct Stage2McpPromptArgumentSnapshot {
    name: String,
    description: String,
    required: bool,
}

#[derive(Debug, Clone, Default)]
struct Stage2GuidanceFocus {
    selected_contract_names: BTreeSet<String>,
    selected_server_names: BTreeSet<String>,
    selected_prompt_keys: BTreeSet<String>,
    signals: BTreeSet<String>,
}

#[derive(Debug, Clone)]
struct Stage2SkillSnapshot {
    name: String,
    source: String,
    path: String,
}

async fn collect_stage2_guidance_snapshot(
    runtime_resources: &RuntimeResources,
) -> Stage2GuidanceSnapshot {
    let (mcp_servers, mut warnings) =
        collect_stage2_mcp_runtime_snapshot(runtime_resources.mcp_manager.clone()).await;
    let (skills, mut skill_warnings) =
        collect_stage2_skills_runtime_snapshot(&runtime_resources.skill_config).await;
    warnings.append(&mut skill_warnings);
    Stage2GuidanceSnapshot {
        mcp_servers,
        skills,
        warnings,
    }
}

fn build_stage2_guidance_focus(
    user_input: &str,
    runtime_resources: &RuntimeResources,
    snapshot: &Stage2GuidanceSnapshot,
) -> Stage2GuidanceFocus {
    let normalized = user_input.trim().to_ascii_lowercase();
    let mut focus = Stage2GuidanceFocus::default();
    let image_intent = contains_any(
        &normalized,
        &[
            "image",
            "img",
            "picture",
            "screenshot",
            "ocr",
            "vision",
            "图片",
            "图像",
            "截图",
            "识别",
        ],
    );
    let path_intent = contains_any(
        &normalized,
        &[
            "path",
            "file",
            "folder",
            "directory",
            "workspace",
            "cwd",
            "路径",
            "文件",
            "目录",
            "工作区",
        ],
    );
    let prompt_intent = contains_any(&normalized, &["prompt", "提示词", "模板"]);
    let resource_intent = contains_any(&normalized, &["resource", "资源", "uri"]);
    let skills_intent = contains_any(&normalized, &["skill", "skills", "技能"]);

    if image_intent {
        focus.signals.insert("image_intent".to_string());
    }
    if path_intent {
        focus.signals.insert("path_intent".to_string());
    }
    if prompt_intent {
        focus.signals.insert("prompt_intent".to_string());
    }
    if resource_intent {
        focus.signals.insert("resource_intent".to_string());
    }
    if skills_intent {
        focus.signals.insert("skills_intent".to_string());
    }

    for contract in &runtime_resources.mcp_tool_contracts {
        if normalized.contains(&contract.server_name.to_ascii_lowercase())
            || normalized.contains(&contract.call_name.to_ascii_lowercase())
            || normalized.contains(&contract.public_name.to_ascii_lowercase())
        {
            focus
                .selected_contract_names
                .insert(contract.public_name.clone());
            focus
                .selected_server_names
                .insert(contract.server_name.clone());
            continue;
        }

        if image_intent && contract_matches_image_intent(contract) {
            focus
                .selected_contract_names
                .insert(contract.public_name.clone());
            focus
                .selected_server_names
                .insert(contract.server_name.clone());
            continue;
        }

        if path_intent && contract_matches_path_intent(contract) {
            focus
                .selected_contract_names
                .insert(contract.public_name.clone());
            focus
                .selected_server_names
                .insert(contract.server_name.clone());
        }
    }

    for server in &snapshot.mcp_servers {
        let server_key = server.server_name.to_ascii_lowercase();
        if normalized.contains(&server_key) {
            focus
                .selected_server_names
                .insert(server.server_name.clone());
            continue;
        }
        if resource_intent && server.resource_count > 0 {
            focus
                .selected_server_names
                .insert(server.server_name.clone());
            continue;
        }
        if prompt_intent && server.prompt_count > 0 {
            focus
                .selected_server_names
                .insert(server.server_name.clone());
            continue;
        }
    }

    for server in &snapshot.mcp_servers {
        for prompt in &server.prompts {
            let prompt_key = stage2_prompt_key(&server.server_name, &prompt.name);
            if normalized.contains(&prompt.name.to_ascii_lowercase()) {
                focus.selected_prompt_keys.insert(prompt_key.clone());
                focus
                    .selected_server_names
                    .insert(server.server_name.clone());
                continue;
            }

            let prompt_image_intent = prompt
                .arguments
                .iter()
                .any(stage2_prompt_argument_matches_image_intent);
            let prompt_path_intent = prompt
                .arguments
                .iter()
                .any(stage2_prompt_argument_matches_path_intent);

            if image_intent && prompt_image_intent {
                focus.selected_prompt_keys.insert(prompt_key.clone());
                focus
                    .selected_server_names
                    .insert(server.server_name.clone());
                continue;
            }
            if path_intent && prompt_path_intent {
                focus.selected_prompt_keys.insert(prompt_key.clone());
                focus
                    .selected_server_names
                    .insert(server.server_name.clone());
                continue;
            }
            if prompt_intent && !prompt.arguments.is_empty() {
                focus.selected_prompt_keys.insert(prompt_key.clone());
                focus
                    .selected_server_names
                    .insert(server.server_name.clone());
            }
        }
    }

    if focus.selected_contract_names.is_empty() {
        let mut fallback = runtime_resources
            .mcp_tool_contracts
            .iter()
            .filter(|contract| {
                contract_matches_image_intent(contract) || contract_matches_path_intent(contract)
            })
            .collect::<Vec<_>>();
        if fallback.is_empty() {
            fallback = runtime_resources
                .mcp_tool_contracts
                .iter()
                .collect::<Vec<_>>();
        }
        for contract in fallback.into_iter().take(8) {
            focus
                .selected_contract_names
                .insert(contract.public_name.clone());
            focus
                .selected_server_names
                .insert(contract.server_name.clone());
        }
    }

    if focus.selected_server_names.is_empty() {
        for server in snapshot.mcp_servers.iter().take(4) {
            focus
                .selected_server_names
                .insert(server.server_name.clone());
        }
    }

    if focus.selected_prompt_keys.is_empty() {
        for server in snapshot
            .mcp_servers
            .iter()
            .filter(|server| focus.selected_server_names.contains(&server.server_name))
        {
            for prompt in server.prompts.iter().take(6) {
                focus
                    .selected_prompt_keys
                    .insert(stage2_prompt_key(&server.server_name, &prompt.name));
            }
        }
    }

    focus
}

fn contains_any(text: &str, keywords: &[&str]) -> bool {
    keywords.iter().any(|keyword| text.contains(keyword))
}

fn contract_matches_image_intent(contract: &McpToolContract) -> bool {
    contract.is_image_tool
        || contract
            .schema_rules
            .iter()
            .any(|rule| rule.kind == McpPathFieldKind::Image)
}

fn contract_matches_path_intent(contract: &McpToolContract) -> bool {
    contract
        .schema_rules
        .iter()
        .any(|rule| rule.kind == McpPathFieldKind::Generic)
}

fn stage2_prompt_argument_matches_image_intent(argument: &Stage2McpPromptArgumentSnapshot) -> bool {
    let name = argument.name.to_ascii_lowercase();
    let description = argument.description.to_ascii_lowercase();
    let desc_image_hint = description.contains("image")
        || description.contains("img")
        || description.contains("图片")
        || description.contains("图像");
    let desc_path_hint = description.contains("path")
        || description.contains("file")
        || description.contains("uri")
        || description.contains("source")
        || description.contains("路径")
        || description.contains("文件");
    is_explicit_image_field_name(&name) || (desc_image_hint && desc_path_hint)
}

fn stage2_prompt_argument_matches_path_intent(argument: &Stage2McpPromptArgumentSnapshot) -> bool {
    let name = argument.name.to_ascii_lowercase();
    let description = argument.description.to_ascii_lowercase();
    is_path_like_field_name(&name)
        || description.contains("path")
        || description.contains("file")
        || description.contains("uri")
        || description.contains("路径")
        || description.contains("文件")
}

fn stage2_prompt_key(server_name: &str, prompt_name: &str) -> String {
    format!("{}:{}", server_name.trim(), prompt_name.trim())
}

async fn collect_stage2_mcp_runtime_snapshot(
    mcp_manager: Option<Arc<McpManager>>,
) -> (Vec<Stage2McpServerSnapshot>, Vec<String>) {
    let Some(manager) = mcp_manager else {
        return (Vec::new(), Vec::new());
    };

    let mut snapshots = Vec::<Stage2McpServerSnapshot>::new();
    let mut warnings = Vec::<String>::new();
    let list_timeout = std::cmp::min(
        manager.default_timeout().unwrap_or(Duration::from_secs(5)),
        Duration::from_secs(8),
    );

    for (server_name, client, tools) in manager.get_all_servers().await {
        let (resource_count, resource_names) =
            match timeout(list_timeout, client.list_resources()).await {
                Ok(Ok(resources)) => {
                    let names = resources
                        .iter()
                        .map(|resource| {
                            let name = resource.name.trim();
                            if name.is_empty() {
                                resource.uri.trim().to_string()
                            } else {
                                name.to_string()
                            }
                        })
                        .filter(|name| !name.is_empty())
                        .collect::<Vec<_>>();
                    (resources.len(), names)
                }
                Ok(Err(err)) => {
                    warnings.push(format!(
                        "guidance mcp resources failed for server '{}': {}",
                        server_name, err
                    ));
                    (0, Vec::new())
                }
                Err(_) => {
                    warnings.push(format!(
                        "guidance mcp resources timed out for server '{}' after {:?}",
                        server_name, list_timeout
                    ));
                    (0, Vec::new())
                }
            };

        let (prompt_count, prompts) = match timeout(list_timeout, client.list_prompts()).await {
            Ok(Ok(prompts)) => {
                let prompt_snapshots = prompts
                    .into_iter()
                    .filter_map(|prompt| {
                        let prompt_name = prompt.name.trim().to_string();
                        if prompt_name.is_empty() {
                            return None;
                        }
                        let arguments = prompt
                            .arguments
                            .unwrap_or_default()
                            .into_iter()
                            .filter_map(|argument| {
                                let arg_name = argument.name.trim().to_string();
                                if arg_name.is_empty() {
                                    return None;
                                }
                                Some(Stage2McpPromptArgumentSnapshot {
                                    name: arg_name,
                                    description: argument
                                        .description
                                        .unwrap_or_default()
                                        .trim()
                                        .to_string(),
                                    required: argument.required.unwrap_or(false),
                                })
                            })
                            .collect::<Vec<_>>();
                        Some(Stage2McpPromptSnapshot {
                            name: prompt_name,
                            arguments,
                        })
                    })
                    .collect::<Vec<_>>();
                (prompt_snapshots.len(), prompt_snapshots)
            }
            Ok(Err(err)) => {
                warnings.push(format!(
                    "guidance mcp prompts failed for server '{}': {}",
                    server_name, err
                ));
                (0, Vec::new())
            }
            Err(_) => {
                warnings.push(format!(
                    "guidance mcp prompts timed out for server '{}' after {:?}",
                    server_name, list_timeout
                ));
                (0, Vec::new())
            }
        };

        snapshots.push(Stage2McpServerSnapshot {
            server_name,
            tool_count: tools.len(),
            resource_count,
            prompt_count,
            resource_names,
            prompts,
        });
    }

    snapshots.sort_by(|left, right| left.server_name.cmp(&right.server_name));
    (snapshots, warnings)
}

async fn collect_stage2_skills_runtime_snapshot(
    skill_config: &SkillConfig,
) -> (Vec<Stage2SkillSnapshot>, Vec<String>) {
    if !skill_config.enabled {
        return (Vec::new(), Vec::new());
    }

    let loader = SkillLoader::new();
    let mut snapshots = Vec::<Stage2SkillSnapshot>::new();
    let mut warnings = Vec::<String>::new();

    if let Some(personal_dir) = &skill_config.personal_dir {
        collect_stage2_skills_from_directory(
            &loader,
            personal_dir,
            SkillSource::Personal,
            &mut snapshots,
            &mut warnings,
        )
        .await;
    } else if let Some(home) = skill_home_dir() {
        collect_stage2_skills_from_directory(
            &loader,
            &home.join(".cursor").join("skills"),
            SkillSource::Personal,
            &mut snapshots,
            &mut warnings,
        )
        .await;
    }

    if skill_config.project_dirs.is_empty() {
        collect_stage2_skills_from_directory(
            &loader,
            &PathBuf::from(".cursor").join("skills"),
            SkillSource::Project,
            &mut snapshots,
            &mut warnings,
        )
        .await;
    } else {
        for dir in &skill_config.project_dirs {
            collect_stage2_skills_from_directory(
                &loader,
                dir,
                SkillSource::Project,
                &mut snapshots,
                &mut warnings,
            )
            .await;
        }
    }

    snapshots.sort_by(|left, right| {
        left.name
            .cmp(&right.name)
            .then(left.source.cmp(&right.source))
            .then(left.path.cmp(&right.path))
    });
    snapshots.dedup_by(|left, right| left.path == right.path);
    (snapshots, warnings)
}

async fn collect_stage2_skills_from_directory(
    loader: &SkillLoader,
    dir: &Path,
    source: SkillSource,
    snapshots: &mut Vec<Stage2SkillSnapshot>,
    warnings: &mut Vec<String>,
) {
    let source_label = source.as_label().to_string();
    match loader.load_from_directory(dir, &source).await {
        Ok(skills) => {
            for skill in skills {
                snapshots.push(Stage2SkillSnapshot {
                    name: skill.metadata.name,
                    source: source_label.clone(),
                    path: skill.path.to_string_lossy().to_string(),
                });
            }
        }
        Err(err) => warnings.push(format!(
            "guidance skills scan failed for '{}' ({}) : {}",
            source_label,
            dir.display(),
            err
        )),
    }
}

fn build_stage2_guidance_context(
    workspace_context: &WorkspaceContext,
    runtime_resources: &RuntimeResources,
    snapshot: &Stage2GuidanceSnapshot,
    focus: &Stage2GuidanceFocus,
) -> String {
    let mut lines = Vec::<String>::new();
    lines.push("[Guidance agent context]".to_string());
    lines.push(format!(
        "- Workspace root (authoritative cwd): {}",
        workspace_context.root_dir.display()
    ));
    lines.push(format!(
        "- AH runtime dir: {}",
        workspace_context.ah_dir.display()
    ));
    lines.push(format!(
        "- AH screenshots dir: {}",
        workspace_context.screenshots_dir.display()
    ));
    lines.push(format!(
        "- AH image recognition dir: {}",
        workspace_context.image_recognition_dir.display()
    ));
    lines.push(
        "- MCP path rules: use absolute local paths only. Never put natural language into path-like fields."
            .to_string(),
    );
    lines.push(format!(
        "- Image path guard: image fields must resolve under '{}' or '{}'.",
        workspace_context.image_recognition_dir.display(),
        workspace_context.screenshots_dir.display()
    ));

    if !focus.signals.is_empty() {
        lines.push(format!(
            "- Guidance focus signals: {}",
            focus.signals.iter().cloned().collect::<Vec<_>>().join(", ")
        ));
    }

    let contract_lines =
        build_guidance_contract_lines(&runtime_resources.mcp_tool_contracts, focus);
    if contract_lines.is_empty() {
        lines.push("- MCP contracts: no path-like MCP contracts detected.".to_string());
    } else {
        lines.push("- MCP contracts with path-like fields:".to_string());
        lines.extend(contract_lines.into_iter().map(|line| format!("  {line}")));
    }

    if snapshot.mcp_servers.is_empty() {
        lines.push("- MCP runtime inventory: no connected servers.".to_string());
    } else {
        lines.push("- MCP runtime inventory:".to_string());
        for server in snapshot
            .mcp_servers
            .iter()
            .filter(|server| {
                focus.selected_server_names.is_empty()
                    || focus.selected_server_names.contains(&server.server_name)
            })
            .take(8)
        {
            let prompt_names = server
                .prompts
                .iter()
                .map(|prompt| prompt.name.clone())
                .collect::<Vec<_>>();
            lines.push(format!(
                "  - {} | tools={} | resources={} [{}] | prompts={} [{}]",
                server.server_name,
                server.tool_count,
                server.resource_count,
                format_guidance_name_list(&server.resource_names, 6),
                server.prompt_count,
                format_guidance_name_list(&prompt_names, 6)
            ));
        }
    }

    let tool_template_lines = build_guidance_tool_argument_template_lines(
        workspace_context,
        &runtime_resources.mcp_tool_contracts,
        focus,
    );
    if tool_template_lines.is_empty() {
        lines.push("- MCP tool argument templates: none".to_string());
    } else {
        lines.push("- MCP tool argument templates:".to_string());
        lines.extend(
            tool_template_lines
                .into_iter()
                .map(|line| format!("  {line}")),
        );
    }

    let prompt_template_lines =
        build_guidance_prompt_argument_template_lines(workspace_context, snapshot, focus);
    if prompt_template_lines.is_empty() {
        lines.push("- MCP prompt argument templates: none".to_string());
    } else {
        lines.push("- MCP prompt argument templates:".to_string());
        lines.extend(
            prompt_template_lines
                .into_iter()
                .map(|line| format!("  {line}")),
        );
    }

    if runtime_resources.skill_config.enabled {
        let personal = runtime_resources
            .skill_config
            .personal_dir
            .as_ref()
            .map(|path| path.display().to_string())
            .unwrap_or_else(|| "<default>".to_string());
        let projects = if runtime_resources.skill_config.project_dirs.is_empty() {
            "<default>".to_string()
        } else {
            runtime_resources
                .skill_config
                .project_dirs
                .iter()
                .map(|path| path.display().to_string())
                .collect::<Vec<_>>()
                .join("; ")
        };
        lines.push(format!("- Skills enabled: true (personal={personal})"));
        lines.push(format!("- Skills project dirs: {projects}"));
        if snapshot.skills.is_empty() {
            lines.push("- Skills discovered at runtime: none".to_string());
        } else {
            lines.push(format!(
                "- Skills discovered at runtime: {}",
                snapshot.skills.len()
            ));
            for skill in snapshot.skills.iter().take(12) {
                lines.push(format!(
                    "  - {} [{}] ({})",
                    skill.name, skill.source, skill.path
                ));
            }
            if snapshot.skills.len() > 12 {
                lines.push(format!(
                    "  - ... and {} more skills",
                    snapshot.skills.len() - 12
                ));
            }
        }
    } else {
        lines.push("- Skills enabled: false".to_string());
    }

    if !snapshot.warnings.is_empty() {
        lines.push("- Guidance collection warnings:".to_string());
        for warning in snapshot.warnings.iter().take(8) {
            lines.push(format!("  - {warning}"));
        }
        if snapshot.warnings.len() > 8 {
            lines.push(format!(
                "  - ... and {} more warnings",
                snapshot.warnings.len() - 8
            ));
        }
    }

    lines.join("\n")
}

fn format_guidance_name_list(items: &[String], limit: usize) -> String {
    if items.is_empty() {
        return "none".to_string();
    }
    let mut preview = items
        .iter()
        .map(|item| item.trim().to_string())
        .filter(|item| !item.is_empty())
        .take(limit)
        .collect::<Vec<_>>();
    if preview.is_empty() {
        return "none".to_string();
    }
    if items.len() > limit {
        preview.push(format!("+{} more", items.len() - limit));
    }
    preview.join(", ")
}

fn build_guidance_tool_argument_template_lines(
    workspace_context: &WorkspaceContext,
    contracts: &[McpToolContract],
    focus: &Stage2GuidanceFocus,
) -> Vec<String> {
    let mut lines = Vec::<String>::new();
    for contract in contracts.iter().take(30) {
        if !focus.selected_contract_names.is_empty()
            && !focus
                .selected_contract_names
                .contains(&contract.public_name)
        {
            continue;
        }
        let mut template = BTreeSet::<(String, String)>::new();
        for rule in &contract.schema_rules {
            let field = format_field_path(&rule.path);
            if field.is_empty() {
                continue;
            }
            let value = match rule.kind {
                McpPathFieldKind::Image => guidance_image_path_template(workspace_context),
                McpPathFieldKind::Generic => guidance_workspace_path_template(workspace_context),
            };
            template.insert((field, value));
        }

        if template.is_empty() && contract.is_image_tool {
            template.insert((
                "image_source".to_string(),
                guidance_image_path_template(workspace_context),
            ));
        }
        if template.is_empty() {
            continue;
        }

        lines.push(format!(
            "- {} -> {}",
            contract.public_name,
            format_guidance_template_object(&template)
        ));
    }
    lines
}

fn build_guidance_prompt_argument_template_lines(
    workspace_context: &WorkspaceContext,
    snapshot: &Stage2GuidanceSnapshot,
    focus: &Stage2GuidanceFocus,
) -> Vec<String> {
    let mut lines = Vec::<String>::new();
    for server in snapshot.mcp_servers.iter().take(20) {
        if !focus.selected_server_names.is_empty()
            && !focus.selected_server_names.contains(&server.server_name)
        {
            continue;
        }
        for prompt in server.prompts.iter().take(20) {
            let prompt_key = stage2_prompt_key(&server.server_name, &prompt.name);
            if !focus.selected_prompt_keys.is_empty()
                && !focus.selected_prompt_keys.contains(&prompt_key)
            {
                continue;
            }
            if prompt.arguments.is_empty() {
                continue;
            }

            let mut template = BTreeSet::<(String, String)>::new();
            for arg in &prompt.arguments {
                let value = guidance_prompt_argument_template_value(workspace_context, arg);
                template.insert((arg.name.clone(), value));
            }
            if template.is_empty() {
                continue;
            }

            lines.push(format!(
                "- {}:{} -> {}",
                server.server_name,
                prompt.name,
                format_guidance_template_object(&template)
            ));
            if lines.len() >= 20 {
                return lines;
            }
        }
    }
    lines
}

fn guidance_prompt_argument_template_value(
    workspace_context: &WorkspaceContext,
    argument: &Stage2McpPromptArgumentSnapshot,
) -> String {
    let normalized_name = argument.name.trim().to_ascii_lowercase();
    let normalized_desc = argument.description.trim().to_ascii_lowercase();
    let description_image_hint =
        normalized_desc.contains("image") || normalized_desc.contains("img");
    let description_path_hint = normalized_desc.contains("path")
        || normalized_desc.contains("file")
        || normalized_desc.contains("uri")
        || normalized_desc.contains("source");
    let looks_like_image = is_explicit_image_field_name(&normalized_name)
        || (description_image_hint && description_path_hint);
    if looks_like_image {
        return guidance_image_path_template(workspace_context);
    }

    let looks_like_path = is_path_like_field_name(&normalized_name)
        || normalized_desc.contains("path")
        || normalized_desc.contains("file");
    if looks_like_path {
        return guidance_workspace_path_template(workspace_context);
    }

    if argument.required {
        "<text-required>".to_string()
    } else {
        "<text-optional>".to_string()
    }
}

fn guidance_image_path_template(workspace_context: &WorkspaceContext) -> String {
    format!(
        "{}\\<image-file>",
        workspace_context
            .image_recognition_dir
            .to_string_lossy()
            .to_string()
    )
}

fn guidance_workspace_path_template(workspace_context: &WorkspaceContext) -> String {
    format!(
        "{}\\<relative-path>",
        workspace_context.root_dir.to_string_lossy().to_string()
    )
}

fn format_guidance_template_object(template: &BTreeSet<(String, String)>) -> String {
    let mut map = Map::new();
    for (key, value) in template {
        if key.trim().is_empty() {
            continue;
        }
        map.insert(key.clone(), Value::String(value.clone()));
    }
    Value::Object(map).to_string()
}

fn build_guidance_contract_lines(
    contracts: &[McpToolContract],
    focus: &Stage2GuidanceFocus,
) -> Vec<String> {
    let mut lines = Vec::<String>::new();
    for contract in contracts {
        if !focus.selected_contract_names.is_empty()
            && !focus
                .selected_contract_names
                .contains(&contract.public_name)
        {
            continue;
        }
        let mut image_fields = BTreeSet::<String>::new();
        let mut generic_fields = BTreeSet::<String>::new();
        for rule in &contract.schema_rules {
            let field = format_field_path(&rule.path);
            if field.is_empty() {
                continue;
            }
            match rule.kind {
                McpPathFieldKind::Image => {
                    image_fields.insert(field);
                }
                McpPathFieldKind::Generic => {
                    generic_fields.insert(field);
                }
            }
        }

        if image_fields.is_empty() && generic_fields.is_empty() && !contract.is_image_tool {
            continue;
        }

        let image_text = if image_fields.is_empty() {
            "none".to_string()
        } else {
            image_fields.into_iter().collect::<Vec<_>>().join(", ")
        };
        let generic_text = if generic_fields.is_empty() {
            "none".to_string()
        } else {
            generic_fields.into_iter().collect::<Vec<_>>().join(", ")
        };

        lines.push(format!(
            "- {} | image_tool={} | image_fields=[{}] | path_fields=[{}]",
            contract.public_name, contract.is_image_tool, image_text, generic_text
        ));
    }

    lines
}

fn merge_prompt_directives_with_guidance(
    config: &AgentRuntimeConfig,
    prompt_directives: Option<PromptDirectives>,
    guidance_context: &str,
) -> PromptDirectives {
    let mut directives = prompt_directives.unwrap_or(PromptDirectives {
        developer_instructions: Some(control::assemble_developer_instructions(config, None)),
        user_instructions: None,
    });

    let base_developer = directives
        .developer_instructions
        .clone()
        .unwrap_or_else(|| control::assemble_developer_instructions(config, None));
    directives.developer_instructions = Some(format!("{base_developer}\n\n{guidance_context}"));
    directives
}

fn guidance_context_summary(
    workspace_context: &WorkspaceContext,
    runtime_resources: &RuntimeResources,
    snapshot: &Stage2GuidanceSnapshot,
    focus: &Stage2GuidanceFocus,
) -> String {
    format!(
        "Guidance context injected: workspace='{}', mcp_contracts={}/{}, mcp_servers={}/{}, prompts={}, skills_discovered={}, warnings={}",
        workspace_context.root_dir.display(),
        focus.selected_contract_names.len(),
        runtime_resources.mcp_tool_contracts.len(),
        focus.selected_server_names.len(),
        snapshot.mcp_servers.len(),
        focus.selected_prompt_keys.len(),
        snapshot.skills.len().min(12),
        snapshot.warnings.len()
    )
}

fn guidance_context_audit(
    runtime_resources: &RuntimeResources,
    snapshot: &Stage2GuidanceSnapshot,
    focus: &Stage2GuidanceFocus,
) -> String {
    let mut payload = Map::new();
    payload.insert(
        "event".to_string(),
        Value::String("guidance_injected".to_string()),
    );
    payload.insert(
        "signals".to_string(),
        Value::Array(
            focus
                .signals
                .iter()
                .map(|item| Value::String(item.clone()))
                .collect(),
        ),
    );
    payload.insert(
        "selected_contracts".to_string(),
        Value::Array(
            focus
                .selected_contract_names
                .iter()
                .map(|item| Value::String(item.clone()))
                .collect(),
        ),
    );
    payload.insert(
        "selected_servers".to_string(),
        Value::Array(
            focus
                .selected_server_names
                .iter()
                .map(|item| Value::String(item.clone()))
                .collect(),
        ),
    );
    payload.insert(
        "selected_prompts".to_string(),
        Value::Array(
            focus
                .selected_prompt_keys
                .iter()
                .take(16)
                .map(|item| Value::String(item.clone()))
                .collect(),
        ),
    );
    payload.insert(
        "contracts_total".to_string(),
        Value::from(runtime_resources.mcp_tool_contracts.len() as u64),
    );
    payload.insert(
        "servers_total".to_string(),
        Value::from(snapshot.mcp_servers.len() as u64),
    );
    payload.insert(
        "warnings_count".to_string(),
        Value::from(snapshot.warnings.len() as u64),
    );
    format!("GuidanceAudit {}", Value::Object(payload))
}

fn map_runtime_skill_config(
    config: &SkillsRuntimeConfig,
    workspace_context: &WorkspaceContext,
    fallback_base_dir: &Path,
) -> SkillConfig {
    let personal_dir = normalize_optional(config.personal_dir.clone()).map(|value| {
        resolve_preferred_path(&value, &workspace_context.root_dir, fallback_base_dir)
    });
    let project_dirs = config
        .project_dirs
        .iter()
        .filter_map(|dir| {
            let trimmed = dir.trim();
            if trimmed.is_empty() {
                None
            } else {
                Some(resolve_preferred_path(
                    trimmed,
                    &workspace_context.root_dir,
                    fallback_base_dir,
                ))
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
    workspace_context: &WorkspaceContext,
    fallback_base_dir: &Path,
) -> Result<AgentMcpServerConfig, AppError> {
    let name = normalize_optional(Some(server.name.clone()))
        .ok_or_else(|| AppError::InvalidConfig("MCP server name cannot be empty".to_string()))?;
    let transport = map_runtime_transport(server.transport)?;
    let endpoint = normalize_optional(server.endpoint.clone()).unwrap_or_default();
    let command = normalize_optional(server.command.clone()).map(|value| {
        rewrite_mcp_command_or_arg(&value, &workspace_context.root_dir, fallback_base_dir)
    });
    let args = server
        .args
        .iter()
        .filter_map(|arg| {
            let trimmed = arg.trim();
            if trimmed.is_empty() {
                None
            } else {
                Some(rewrite_mcp_command_or_arg(
                    trimmed,
                    &workspace_context.root_dir,
                    fallback_base_dir,
                ))
            }
        })
        .collect::<Vec<_>>();
    let auth = map_runtime_mcp_auth(server.auth.as_ref(), &name)?;
    let tls = map_runtime_mcp_tls(server.tls.as_ref());
    let mut env = normalize_map(&server.env);
    workspace_context.inject_mcp_env(&mut env);

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
        env,
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

fn default_workspace_root() -> PathBuf {
    let home = if cfg!(windows) {
        std::env::var("USERPROFILE")
            .ok()
            .or_else(|| std::env::var("HOME").ok())
    } else {
        std::env::var("HOME")
            .ok()
            .or_else(|| std::env::var("USERPROFILE").ok())
    };

    if let Some(home_dir) = home {
        return normalize_path_lexical(
            &PathBuf::from(home_dir)
                .join(".ai-helper")
                .join("workspaces")
                .join("default"),
        );
    }

    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    normalize_path_lexical(&cwd.join(".ai-helper").join("workspaces").join("default"))
}

fn absolute_path(path: &Path) -> PathBuf {
    if path.is_absolute() {
        return normalize_path_lexical(path);
    }
    if let Ok(cwd) = std::env::current_dir() {
        return normalize_path_lexical(&cwd.join(path));
    }
    normalize_path_lexical(path)
}

fn normalize_path_lexical(path: &Path) -> PathBuf {
    use std::path::Component;

    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::Prefix(prefix) => normalized.push(prefix.as_os_str()),
            Component::RootDir => normalized.push(component.as_os_str()),
            Component::CurDir => {}
            Component::ParentDir => {
                normalized.pop();
            }
            Component::Normal(part) => normalized.push(part),
        }
    }

    normalized
}

fn resolve_preferred_path(raw: &str, workspace_root: &Path, fallback_base: &Path) -> PathBuf {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return absolute_path(workspace_root);
    }

    let raw_path = PathBuf::from(trimmed);
    if raw_path.is_absolute() {
        return absolute_path(&raw_path);
    }

    let workspace_candidate = absolute_path(&workspace_root.join(trimmed));
    if workspace_candidate.exists() {
        return workspace_candidate;
    }

    let fallback_candidate = absolute_path(&fallback_base.join(trimmed));
    if fallback_candidate.exists() {
        return fallback_candidate;
    }

    workspace_candidate
}

fn resolve_existing_path(
    raw: &str,
    workspace_root: &Path,
    fallback_base: &Path,
) -> Option<PathBuf> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }

    let raw_path = PathBuf::from(trimmed);
    if raw_path.is_absolute() {
        return Some(absolute_path(&raw_path));
    }

    let workspace_candidate = absolute_path(&workspace_root.join(trimmed));
    if workspace_candidate.exists() {
        return Some(workspace_candidate);
    }

    let fallback_candidate = absolute_path(&fallback_base.join(trimmed));
    if fallback_candidate.exists() {
        return Some(fallback_candidate);
    }

    None
}

fn looks_like_path_token(value: &str) -> bool {
    let trimmed = value.trim();
    if trimmed.is_empty() || trimmed.contains("://") {
        return false;
    }

    trimmed.starts_with("./")
        || trimmed.starts_with(".\\")
        || trimmed.starts_with("../")
        || trimmed.starts_with("..\\")
        || trimmed.contains('/')
        || trimmed.contains('\\')
}

fn rewrite_mcp_command_or_arg(raw: &str, workspace_root: &Path, fallback_base: &Path) -> String {
    let trimmed = raw.trim();
    if !looks_like_path_token(trimmed) {
        return trimmed.to_string();
    }

    if let Some(resolved) = resolve_existing_path(trimmed, workspace_root, fallback_base) {
        return resolved.to_string_lossy().to_string();
    }

    trimmed.to_string()
}

fn skill_home_dir() -> Option<PathBuf> {
    if let Ok(home) = std::env::var("USERPROFILE") {
        return Some(PathBuf::from(home));
    }
    std::env::var("HOME").ok().map(PathBuf::from)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum McpPathFieldKind {
    Generic,
    Image,
}

#[derive(Debug, Clone)]
struct McpSchemaPathRule {
    path: Vec<String>,
    kind: McpPathFieldKind,
}

#[derive(Debug, Clone)]
struct McpToolContract {
    public_name: String,
    server_name: String,
    call_name: String,
    is_image_tool: bool,
    schema_rules: Vec<McpSchemaPathRule>,
}

impl McpToolContract {
    fn compile(server_name: &str, call_name: &str, schema: &Value) -> Self {
        let mut schema_rules = Vec::new();
        collect_schema_path_rules(schema, &mut Vec::new(), &mut schema_rules);
        dedupe_schema_rules(&mut schema_rules);

        Self {
            public_name: format!("mcp:{server_name}:{call_name}"),
            server_name: server_name.to_string(),
            call_name: call_name.to_string(),
            is_image_tool: is_image_tool_name(call_name),
            schema_rules,
        }
    }

    fn path_kind_for(&self, path: &[String]) -> Option<McpPathFieldKind> {
        self.schema_rules
            .iter()
            .find(|rule| path_matches_rule(path, &rule.path))
            .map(|rule| rule.kind)
    }
}

#[derive(Debug, Default, Clone, Copy)]
struct McpPathNormalizationReport {
    rewritten_count: usize,
}

#[derive(Debug, Clone)]
struct McpPathGatewayContext {
    workspace_root: PathBuf,
    image_recognition_dir: PathBuf,
    screenshots_dir: PathBuf,
}

#[derive(Debug, Clone)]
struct McpPathGatewayReject {
    field_path: String,
    reason: String,
}

struct McpPathGateway;

impl McpPathGateway {
    fn from_tool_context(ctx: &ToolContext) -> Result<McpPathGatewayContext, AgentError> {
        let Some(cwd) = ctx.cwd.as_deref().map(str::trim) else {
            return Err(AgentError::Tool(
                "mcp_path_gateway_rejected: missing tool context cwd".to_string(),
            ));
        };
        if cwd.is_empty() {
            return Err(AgentError::Tool(
                "mcp_path_gateway_rejected: empty tool context cwd".to_string(),
            ));
        }

        let workspace_root = absolute_path(Path::new(cwd));
        Ok(McpPathGatewayContext {
            image_recognition_dir: workspace_root.join(".ah").join("image-recognition"),
            screenshots_dir: workspace_root.join(".ah").join("screenshots"),
            workspace_root,
        })
    }
}

fn normalize_mcp_tool_args(
    contract: &McpToolContract,
    args: Map<String, Value>,
    ctx: &ToolContext,
) -> Result<Map<String, Value>, AgentError> {
    normalize_mcp_tool_args_with_report(contract, args, ctx).map(|(value, _)| value)
}

fn normalize_mcp_tool_args_with_report(
    contract: &McpToolContract,
    mut args: Map<String, Value>,
    ctx: &ToolContext,
) -> Result<(Map<String, Value>, McpPathNormalizationReport), AgentError> {
    let gateway = McpPathGateway::from_tool_context(ctx)?;
    let mut report = McpPathNormalizationReport::default();

    for (field_name, value) in args.iter_mut() {
        let field_path = vec![field_name.clone()];
        normalize_mcp_tool_arg_value(
            contract,
            &gateway,
            value,
            &field_path,
            &mut report,
        )
        .map_err(|reject| {
            log::warn!(
                "mcp_path_gateway rejected arguments | tool={} server={} rejected_field={} reason={}",
                contract.public_name,
                contract.server_name,
                reject.field_path,
                reject.reason
            );
            AgentError::Tool(format!(
                "mcp_path_gateway_rejected: field '{}' for tool '{}' ({})",
                reject.field_path, contract.public_name, reject.reason
            ))
        })?;
    }

    Ok((args, report))
}

fn normalize_mcp_tool_arg_value(
    contract: &McpToolContract,
    gateway: &McpPathGatewayContext,
    value: &mut Value,
    current_path: &[String],
    report: &mut McpPathNormalizationReport,
) -> Result<(), McpPathGatewayReject> {
    match value {
        Value::Object(map) => {
            for (key, item) in map {
                let mut next_path = current_path.to_vec();
                next_path.push(key.clone());
                normalize_mcp_tool_arg_value(contract, gateway, item, &next_path, report)?;
            }
            Ok(())
        }
        Value::Array(items) => {
            for item in items {
                normalize_mcp_tool_arg_value(contract, gateway, item, current_path, report)?;
            }
            Ok(())
        }
        Value::String(text) => {
            let original = text.clone();
            let current_key = current_path.last().map(String::as_str).unwrap_or_default();
            if let Some(normalized) =
                normalize_mcp_path_string(contract, gateway, current_path, current_key, &original)?
            {
                if normalized != original {
                    *text = normalized;
                    report.rewritten_count += 1;
                }
            }
            Ok(())
        }
        _ => Ok(()),
    }
}

fn normalize_mcp_path_string(
    contract: &McpToolContract,
    gateway: &McpPathGatewayContext,
    current_path: &[String],
    current_key: &str,
    raw_value: &str,
) -> Result<Option<String>, McpPathGatewayReject> {
    let raw_trimmed = raw_value.trim();
    if raw_trimmed.is_empty() {
        return Ok(None);
    }

    let hint_kind = contract.path_kind_for(current_path);
    let key_path_like = is_path_like_field_name(current_key);
    let key_image_like = is_explicit_image_field_name(current_key);
    let value_path_like = looks_like_path_value(raw_trimmed);
    let value_image_basename = looks_like_image_basename(raw_trimmed);

    let is_image_field = matches!(hint_kind, Some(McpPathFieldKind::Image))
        || key_image_like
        || (contract.is_image_tool && value_image_basename);
    let is_path_field = hint_kind.is_some()
        || key_path_like
        || value_path_like
        || (contract.is_image_tool && value_image_basename);

    if !is_path_field {
        return Ok(None);
    }
    if raw_trimmed.starts_with("data:") {
        return Err(McpPathGatewayReject {
            field_path: format_field_path(current_path),
            reason: "data URL is not allowed for path field; expected local absolute path"
                .to_string(),
        });
    }

    let resolved = resolve_mcp_path_value(raw_trimmed, is_image_field, gateway, current_path)?;
    validate_mcp_path_value(
        &resolved.path,
        is_image_field,
        gateway,
        current_path,
        raw_trimmed,
        resolved.requires_existence_check,
    )?;

    Ok(Some(resolved.path.to_string_lossy().to_string()))
}

struct ResolvedMcpPath {
    path: PathBuf,
    requires_existence_check: bool,
}

fn resolve_mcp_path_value(
    raw_trimmed: &str,
    is_image_field: bool,
    gateway: &McpPathGatewayContext,
    current_path: &[String],
) -> Result<ResolvedMcpPath, McpPathGatewayReject> {
    let raw_path = PathBuf::from(raw_trimmed);
    if raw_path.is_absolute() {
        return Ok(ResolvedMcpPath {
            path: absolute_path(&raw_path),
            requires_existence_check: is_image_field,
        });
    }

    if looks_like_path_token(raw_trimmed) {
        return Ok(ResolvedMcpPath {
            path: absolute_path(&gateway.workspace_root.join(raw_trimmed)),
            requires_existence_check: is_image_field,
        });
    }

    if is_image_field {
        let resolved = resolve_unique_image_path(raw_trimmed, gateway).map_err(|reason| {
            McpPathGatewayReject {
                field_path: format_field_path(current_path),
                reason,
            }
        })?;
        return Ok(ResolvedMcpPath {
            path: resolved,
            requires_existence_check: true,
        });
    }

    let candidate = absolute_path(&gateway.workspace_root.join(raw_trimmed));
    if !candidate.exists() {
        return Err(McpPathGatewayReject {
            field_path: format_field_path(current_path),
            reason: format!(
                "path '{}' was resolved to '{}' but does not exist",
                raw_trimmed,
                candidate.display()
            ),
        });
    }
    Ok(ResolvedMcpPath {
        path: candidate,
        requires_existence_check: true,
    })
}

fn validate_mcp_path_value(
    candidate: &Path,
    is_image_field: bool,
    gateway: &McpPathGatewayContext,
    current_path: &[String],
    raw_value: &str,
    require_exists: bool,
) -> Result<(), McpPathGatewayReject> {
    let absolute_candidate = absolute_path(candidate);
    let workspace_root = absolute_path(&gateway.workspace_root);
    let image_recognition_dir = absolute_path(&gateway.image_recognition_dir);
    let screenshots_dir = absolute_path(&gateway.screenshots_dir);

    if is_image_field {
        if require_exists && !absolute_candidate.exists() {
            return Err(McpPathGatewayReject {
                field_path: format_field_path(current_path),
                reason: format!(
                    "image path '{}' resolved to '{}' but file does not exist",
                    raw_value,
                    absolute_candidate.display()
                ),
            });
        }
        let allowed = is_path_within(&absolute_candidate, &image_recognition_dir)
            || is_path_within(&absolute_candidate, &screenshots_dir);
        if !allowed {
            return Err(McpPathGatewayReject {
                field_path: format_field_path(current_path),
                reason: format!(
                    "image path '{}' resolved to '{}' outside allowed dirs ('{}' or '{}')",
                    raw_value,
                    absolute_candidate.display(),
                    image_recognition_dir.display(),
                    screenshots_dir.display()
                ),
            });
        }
        return Ok(());
    }

    if !is_path_within(&absolute_candidate, &workspace_root) {
        return Err(McpPathGatewayReject {
            field_path: format_field_path(current_path),
            reason: format!(
                "path '{}' resolved to '{}' outside workspace '{}'",
                raw_value,
                absolute_candidate.display(),
                workspace_root.display()
            ),
        });
    }

    Ok(())
}

fn resolve_unique_image_path(
    raw_value: &str,
    gateway: &McpPathGatewayContext,
) -> Result<PathBuf, String> {
    let mut candidates = Vec::<PathBuf>::new();
    let normalized = raw_value.trim().to_ascii_lowercase();
    let normalized_stem = Path::new(raw_value.trim())
        .file_stem()
        .and_then(|item| item.to_str())
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase();

    for dir in [&gateway.image_recognition_dir, &gateway.screenshots_dir] {
        collect_matching_image_paths(dir, &normalized, &normalized_stem, &mut candidates).map_err(
            |err| {
                format!(
                    "failed to search image directory '{}': {}",
                    dir.display(),
                    err
                )
            },
        )?;
    }

    candidates.sort();
    candidates.dedup();
    if candidates.is_empty() && is_generic_image_reference(raw_value) {
        let mut fallback_candidates = Vec::<PathBuf>::new();
        for dir in [&gateway.image_recognition_dir, &gateway.screenshots_dir] {
            collect_all_file_paths(dir, &mut fallback_candidates).map_err(|err| {
                format!(
                    "failed to search fallback image directory '{}': {}",
                    dir.display(),
                    err
                )
            })?;
        }

        fallback_candidates.sort();
        fallback_candidates.dedup();
        if fallback_candidates.len() == 1 {
            return Ok(absolute_path(&fallback_candidates[0]));
        }
    }

    match candidates.len() {
        1 => Ok(candidates[0].clone()),
        0 => Err(format!(
            "image '{}' was not found in '{}' or '{}'",
            raw_value,
            gateway.image_recognition_dir.display(),
            gateway.screenshots_dir.display()
        )),
        _ => Err(format!(
            "image '{}' is ambiguous: {} matches found",
            raw_value,
            candidates.len()
        )),
    }
}

fn is_generic_image_reference(raw_value: &str) -> bool {
    let trimmed = raw_value.trim();
    if trimmed.is_empty()
        || trimmed.contains('/')
        || trimmed.contains('\\')
        || (trimmed.len() > 1 && trimmed.as_bytes()[1] == b':')
    {
        return false;
    }

    let path = Path::new(trimmed);
    let stem = path
        .file_stem()
        .and_then(|item| item.to_str())
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase();
    let ext = path
        .extension()
        .and_then(|item| item.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    let has_image_ext = ext.is_empty()
        || matches!(
            ext.as_str(),
            "png" | "jpg" | "jpeg" | "webp" | "gif" | "bmp" | "tiff" | "tif" | "svg"
        );
    if !has_image_ext {
        return false;
    }

    stem == "image"
        || stem.starts_with("image-")
        || stem.starts_with("image_")
        || stem == "img"
        || stem.starts_with("screenshot")
        || stem.starts_with("screen")
        || stem.starts_with("photo")
        || stem.starts_with("picture")
}

fn collect_matching_image_paths(
    root_dir: &Path,
    normalized_raw: &str,
    normalized_stem: &str,
    out: &mut Vec<PathBuf>,
) -> std::io::Result<()> {
    if !root_dir.exists() {
        return Ok(());
    }

    let mut stack = vec![root_dir.to_path_buf()];
    while let Some(dir) = stack.pop() {
        for entry_result in std::fs::read_dir(&dir)? {
            let entry = entry_result?;
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
                continue;
            }
            if !path.is_file() || !has_image_extension(&path) {
                continue;
            }

            let Some(file_name) = path.file_name().and_then(|name| name.to_str()) else {
                continue;
            };
            let file_name_lower = file_name.to_ascii_lowercase();
            let file_stem_lower = Path::new(file_name)
                .file_stem()
                .and_then(|item| item.to_str())
                .unwrap_or_default()
                .to_ascii_lowercase();

            let matched = file_name_lower == normalized_raw
                || (!normalized_stem.is_empty() && file_stem_lower == normalized_stem)
                || (!normalized_stem.is_empty()
                    && file_stem_lower.starts_with(&format!("{normalized_stem}-")));

            if matched {
                out.push(absolute_path(&path));
            }
        }
    }

    Ok(())
}

fn collect_all_file_paths(root_dir: &Path, out: &mut Vec<PathBuf>) -> std::io::Result<()> {
    if !root_dir.exists() {
        return Ok(());
    }

    let mut stack = vec![root_dir.to_path_buf()];
    while let Some(dir) = stack.pop() {
        for entry_result in std::fs::read_dir(&dir)? {
            let entry = entry_result?;
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
                continue;
            }
            if path.is_file() {
                out.push(absolute_path(&path));
            }
        }
    }
    Ok(())
}

fn has_image_extension(path: &Path) -> bool {
    let ext = path
        .extension()
        .and_then(|item| item.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    matches!(
        ext.as_str(),
        "png" | "jpg" | "jpeg" | "webp" | "gif" | "bmp" | "tiff" | "tif" | "svg"
    )
}

fn is_path_within(candidate: &Path, root: &Path) -> bool {
    let candidate = absolute_path(candidate);
    let root = absolute_path(root);
    candidate.starts_with(&root)
}

fn format_field_path(path: &[String]) -> String {
    path.iter()
        .map(|segment| segment.trim())
        .filter(|segment| !segment.is_empty())
        .collect::<Vec<_>>()
        .join(".")
}

fn collect_schema_path_rules(
    schema: &Value,
    current_path: &mut Vec<String>,
    out: &mut Vec<McpSchemaPathRule>,
) {
    let Some(schema_object) = schema.as_object() else {
        return;
    };

    if let Some(properties) = schema_object.get("properties").and_then(Value::as_object) {
        for (key, child_schema) in properties {
            current_path.push(key.clone());
            if let Some(kind) = classify_schema_path_field(key, child_schema) {
                out.push(McpSchemaPathRule {
                    path: current_path.clone(),
                    kind,
                });
            }
            collect_schema_path_rules(child_schema, current_path, out);
            current_path.pop();
        }
    }

    if let Some(items) = schema_object.get("items") {
        collect_schema_path_rules(items, current_path, out);
    }

    for key in ["anyOf", "oneOf", "allOf"] {
        if let Some(variants) = schema_object.get(key).and_then(Value::as_array) {
            for variant in variants {
                collect_schema_path_rules(variant, current_path, out);
            }
        }
    }
}

fn dedupe_schema_rules(rules: &mut Vec<McpSchemaPathRule>) {
    let mut deduped = Vec::<McpSchemaPathRule>::new();

    for rule in rules.drain(..) {
        if let Some(existing) = deduped
            .iter_mut()
            .find(|item| path_matches_rule(&item.path, &rule.path))
        {
            if rule.kind == McpPathFieldKind::Image {
                existing.kind = McpPathFieldKind::Image;
            }
            continue;
        }
        deduped.push(rule);
    }

    *rules = deduped;
}

fn path_matches_rule(current_path: &[String], rule_path: &[String]) -> bool {
    current_path.len() == rule_path.len()
        && current_path
            .iter()
            .zip(rule_path.iter())
            .all(|(left, right)| left.eq_ignore_ascii_case(right))
}

fn classify_schema_path_field(key: &str, schema: &Value) -> Option<McpPathFieldKind> {
    let normalized_key = key.trim().to_ascii_lowercase();
    let description = schema
        .as_object()
        .and_then(|value| value.get("description"))
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_ascii_lowercase();
    let title = schema
        .as_object()
        .and_then(|value| value.get("title"))
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_ascii_lowercase();
    let metadata = format!("{description} {title}");

    let key_image_hint = is_explicit_image_field_name(&normalized_key);
    let key_path_hint = is_path_like_field_name(&normalized_key);
    let metadata_path_hint = metadata.contains("path")
        || metadata.contains("filepath")
        || metadata.contains("file path")
        || metadata.contains("local file")
        || metadata.contains("absolute path")
        || metadata.contains("full path");
    let metadata_image_hint = metadata.contains("image") || metadata.contains("img");
    let image_hint = key_image_hint || (metadata_image_hint && metadata_path_hint);
    let path_hint = key_path_hint || metadata_path_hint;

    if image_hint {
        return Some(McpPathFieldKind::Image);
    }
    if path_hint {
        return Some(McpPathFieldKind::Generic);
    }
    None
}

fn is_image_tool_name(tool_name: &str) -> bool {
    let normalized = tool_name.trim().to_ascii_lowercase();
    normalized.contains("analyze_image")
        || normalized.contains("ocr")
        || normalized.contains("vision")
}

fn is_path_like_field_name(key: &str) -> bool {
    let normalized = key.trim().to_ascii_lowercase();
    matches!(
        normalized.as_str(),
        "path"
            | "paths"
            | "filepath"
            | "file_path"
            | "file"
            | "files"
            | "input_file"
            | "output_file"
            | "image"
            | "image_path"
            | "images"
    ) || normalized.contains("path")
        || normalized.ends_with("_file")
        || normalized.ends_with("_files")
        || normalized.ends_with("_filepath")
        || normalized.ends_with("_file_path")
        || normalized.starts_with("file_")
        || normalized.starts_with("image_")
        || normalized.ends_with("_image")
        || normalized.ends_with("_image_path")
}

fn is_explicit_image_field_name(key: &str) -> bool {
    let normalized = key.trim().to_ascii_lowercase();
    matches!(
        normalized.as_str(),
        "image"
            | "images"
            | "image_path"
            | "image_source"
            | "source_image"
            | "input_image"
            | "output_image"
            | "image_file"
            | "img"
    ) || normalized.starts_with("image_")
        || normalized.starts_with("img_")
        || normalized.ends_with("_image")
        || normalized.ends_with("_image_path")
        || normalized.ends_with("_image_file")
        || normalized.ends_with("_img")
}

fn looks_like_path_value(value: &str) -> bool {
    let trimmed = value.trim();
    if trimmed.is_empty()
        || trimmed.contains('\n')
        || trimmed.contains('\r')
        || trimmed.contains("://")
    {
        return false;
    }
    if trimmed.starts_with("./")
        || trimmed.starts_with(".\\")
        || trimmed.starts_with("../")
        || trimmed.starts_with("..\\")
        || trimmed.starts_with('/')
        || trimmed.starts_with('\\')
    {
        return true;
    }
    if trimmed.len() > 1 && trimmed.as_bytes()[1] == b':' {
        return true;
    }

    let has_separator = trimmed.contains('/') || trimmed.contains('\\');
    if has_separator {
        return !trimmed.contains(' ') && !trimmed.contains('{') && !trimmed.contains('}');
    }

    looks_like_image_basename(trimmed)
}

fn looks_like_image_basename(value: &str) -> bool {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return false;
    }
    if trimmed.contains('/')
        || trimmed.contains('\\')
        || (trimmed.len() > 1 && trimmed.as_bytes()[1] == b':')
    {
        return false;
    }

    let ext = Path::new(trimmed)
        .extension()
        .and_then(|item| item.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    if ext.is_empty() {
        return trimmed
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' || ch == '.');
    }

    matches!(
        ext.as_str(),
        "png" | "jpg" | "jpeg" | "webp" | "gif" | "bmp" | "tiff" | "tif" | "svg"
    )
}

#[derive(Debug, Clone)]
struct PrefixedMcpTool {
    public_name: String,
    server_name: String,
    call_name: String,
    description: String,
    schema: Value,
    contract: McpToolContract,
    client: Arc<McpClient>,
}

impl PrefixedMcpTool {
    fn new(
        server_name: String,
        call_name: String,
        description: String,
        schema: Value,
        contract: McpToolContract,
        client: Arc<McpClient>,
    ) -> Self {
        Self {
            public_name: format!("mcp:{server_name}:{call_name}"),
            server_name,
            call_name,
            description,
            schema,
            contract,
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

    async fn execute(&self, args: Value, ctx: &ToolContext) -> AgentResult<ToolResult> {
        let arguments = match args {
            Value::Object(arguments) => arguments,
            _ => {
                return Err(AgentError::Tool(
                    "MCP tool arguments must be a JSON object".to_string(),
                ));
            }
        };
        let (normalized_arguments, report) =
            normalize_mcp_tool_args_with_report(&self.contract, arguments, ctx)?;
        if report.rewritten_count > 0 {
            log::info!(
                "mcp_path_gateway normalized arguments | tool={} server={} call_name={} rewritten_count={}",
                self.public_name,
                self.server_name,
                self.contract.call_name,
                report.rewritten_count
            );
        }

        let result = self
            .client
            .call_tool(CallToolRequestParams {
                meta: None,
                name: self.call_name.clone().into(),
                arguments: Some(normalized_arguments),
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

fn normalize_tool_call_requested_args_for_display(
    tool: &str,
    args: &Value,
    workspace_root: Option<&Path>,
) -> Value {
    let Some(workspace_root) = workspace_root else {
        return args.clone();
    };
    if !tool.starts_with("mcp:") {
        return args.clone();
    }
    let Some(arguments) = args.as_object().cloned() else {
        return args.clone();
    };

    let tool_name = tool.trim();
    let stripped = tool_name.trim_start_matches("mcp:");
    let Some((server_name, call_name)) = stripped.split_once(':') else {
        return args.clone();
    };

    let contract = McpToolContract {
        public_name: tool_name.to_string(),
        server_name: server_name.trim().to_string(),
        call_name: call_name.trim().to_string(),
        is_image_tool: is_image_tool_name(call_name),
        schema_rules: Vec::new(),
    };
    let workspace_root = absolute_path(workspace_root).to_string_lossy().to_string();
    let ctx = ToolContext {
        cwd: Some(workspace_root.clone()),
        sandbox_root: Some(workspace_root),
    };

    match normalize_mcp_tool_args(&contract, arguments, &ctx) {
        Ok(normalized) => Value::Object(normalized),
        Err(err) => {
            log::debug!(
                "mcp_path_gateway preview normalization failed | tool={} reason={}",
                tool_name,
                err
            );
            args.clone()
        }
    }
}

fn map_protocol_event(
    event: &Event,
    workspace_root: Option<&Path>,
) -> Option<ProtocolEventPayload> {
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
            args: normalize_tool_call_requested_args_for_display(tool, args, workspace_root),
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
    workspace_context: &WorkspaceContext,
) -> PreparedImageFallbackPayload {
    let mut prepared = PreparedImageFallbackPayload::default();

    let ParsedStreamInput::Text(text) = parse_slash_input(&input.content) else {
        return prepared;
    };

    if input.images.is_empty() {
        return prepared;
    }

    let mut persisted_local_paths = Vec::with_capacity(input.images.len());
    for image in &input.images {
        match persist_image_for_recognition(image, &workspace_context.image_recognition_dir) {
            Ok(path) => persisted_local_paths.push(Some(path.to_string_lossy().to_string())),
            Err(err) => {
                prepared.warnings.push(format!(
                    "Failed to persist image '{}' to local path for recognition: {}",
                    image.name, err
                ));
                persisted_local_paths.push(None);
            }
        }
    }

    if config.model_supports_image_input {
        return prepared;
    }

    let fallback_note_text =
        append_image_note_with_local_paths(&text, &input.images, &persisted_local_paths);
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

    for (index, image) in input.images.iter().enumerate() {
        let local_path = persisted_local_paths
            .get(index)
            .and_then(|item| item.as_deref());
        let rendered_template = render_image_recognition_args_template(
            &Value::Object(args_template.clone()),
            image,
            local_path,
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
        if let Some(local_path_value) = local_path {
            let rewritten_image_like = rewrite_image_like_arguments_to_local_path(
                &mut arguments_value,
                &image.name,
                local_path_value,
            );
            let rewritten_filename_refs = rewrite_filename_references_to_local_path(
                &mut arguments_value,
                &image.name,
                local_path_value,
            );
            let rewritten_total = rewritten_image_like + rewritten_filename_refs;
            if rewritten_total > 0 {
                prepared.warnings.push(format!(
                    "Rewrote {} image argument field(s) to local path for '{}': {}",
                    rewritten_total, image.name, local_path_value
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

fn persist_image_for_recognition(
    image: &AgentInputImage,
    directory: &Path,
) -> Result<PathBuf, String> {
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

    std::fs::create_dir_all(directory)
        .map_err(|err| format!("failed to create image fallback directory: {}", err))?;

    let safe_stem = sanitize_image_file_stem(&image.name);
    let extension = extension_for_persisted_image(image);
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

fn extension_for_persisted_image(image: &AgentInputImage) -> String {
    if let Some(mapped) = extension_from_mime_type(&image.mime_type) {
        return mapped.to_string();
    }
    if let Some(from_name) = extension_from_filename(&image.name) {
        return from_name;
    }
    "bin".to_string()
}

fn extension_from_mime_type(mime_type: &str) -> Option<&'static str> {
    match mime_type {
        "image/png" => Some("png"),
        "image/jpeg" | "image/jpg" => Some("jpg"),
        "image/webp" => Some("webp"),
        "image/gif" => Some("gif"),
        "image/bmp" => Some("bmp"),
        "image/tiff" => Some("tiff"),
        "image/svg+xml" => Some("svg"),
        _ => None,
    }
}

fn extension_from_filename(name: &str) -> Option<String> {
    let ext = Path::new(name)
        .extension()
        .and_then(|item| item.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    if matches!(
        ext.as_str(),
        "png" | "jpg" | "jpeg" | "webp" | "gif" | "bmp" | "tiff" | "tif" | "svg"
    ) {
        return Some(ext);
    }
    None
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

    let original_stem = Path::new(original_name.trim())
        .file_stem()
        .and_then(|item| item.to_str())
        .unwrap_or_default()
        .trim();
    if !original_stem.is_empty() && trimmed.eq_ignore_ascii_case(original_stem) {
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

fn rewrite_filename_references_to_local_path(
    value: &mut Value,
    original_name: &str,
    local_path: &str,
) -> usize {
    match value {
        Value::Object(map) => map
            .values_mut()
            .map(|item| rewrite_filename_references_to_local_path(item, original_name, local_path))
            .sum(),
        Value::Array(items) => items
            .iter_mut()
            .map(|item| rewrite_filename_references_to_local_path(item, original_name, local_path))
            .sum(),
        Value::String(text) => {
            if is_bare_image_filename_value(text, original_name) {
                *text = local_path.to_string();
                1
            } else {
                0
            }
        }
        _ => 0,
    }
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
    workspace_root: &Path,
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
            let cwd = workspace_root.to_path_buf();
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
    append_image_note_with_local_paths(text, images, &[])
}

fn append_image_note_with_local_paths(
    text: &str,
    images: &[AgentInputImage],
    local_paths: &[Option<String>],
) -> String {
    let mut lines = Vec::new();
    if !text.is_empty() {
        lines.push(text.to_string());
        lines.push(String::new());
    }
    lines.push("User attached image files:".to_string());
    for (index, image) in images.iter().enumerate() {
        if let Some(path) = local_paths.get(index).and_then(|item| item.as_deref()) {
            lines.push(format!(
                "- {} ({}) | local_path={}",
                image.name, image.mime_type, path
            ));
        } else {
            lines.push(format!("- {} ({})", image.name, image.mime_type));
        }
    }
    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn unique_temp_dir(prefix: &str) -> PathBuf {
        std::env::temp_dir().join(format!("{prefix}-{}", uuid::Uuid::new_v4()))
    }

    fn tool_context_for_workspace(workspace_root: &Path) -> ToolContext {
        let cwd = absolute_path(workspace_root).to_string_lossy().to_string();
        ToolContext {
            cwd: Some(cwd.clone()),
            sandbox_root: Some(cwd),
        }
    }

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
        let (op, payload, is_command) =
            build_stream_op(&input, &config, None, None, Path::new("."));

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
        let (op, payload, is_command) =
            build_stream_op(&input, &config, None, None, Path::new("."));

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

        let (op, _payload, _is_command) = build_stream_op(
            &input,
            &config,
            Some("prepared text".to_string()),
            None,
            Path::new("."),
        );

        match op {
            Op::UserTurn { items, .. } => match &items[0] {
                UserInputItem::Text { text } => assert_eq!(text, "prepared text"),
                _ => panic!("expected text item"),
            },
            _ => panic!("expected user turn op"),
        }
    }

    #[test]
    fn build_stream_op_uses_workspace_cwd_for_payload() {
        let input = AgentStreamInput {
            content: "hello".to_string(),
            images: Vec::new(),
            conversation_id: None,
            recent_messages: Vec::new(),
        };
        let config = AgentRuntimeConfig::default();
        let workspace_root = absolute_path(&unique_temp_dir("workspace-cwd"));

        let (_op, payload, is_command) =
            build_stream_op(&input, &config, None, None, &workspace_root);

        assert!(!is_command);
        match payload {
            ProtocolOpPayload::UserTurn { cwd, .. } => {
                assert_eq!(cwd, workspace_root.to_string_lossy().to_string());
            }
            _ => panic!("expected user turn payload"),
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

    #[test]
    fn rewrite_filename_references_to_local_path_updates_non_image_like_key() {
        let mut args = serde_json::json!({
            "input": "image.png",
            "nested": {
                "source": "image"
            }
        });

        let rewritten = rewrite_filename_references_to_local_path(
            &mut args,
            "image.png",
            "C:\\temp\\image-123.png",
        );

        assert_eq!(rewritten, 2);
        assert_eq!(args["input"], "C:\\temp\\image-123.png");
        assert_eq!(args["nested"]["source"], "C:\\temp\\image-123.png");
    }

    #[test]
    fn is_bare_image_filename_value_accepts_original_stem() {
        assert!(is_bare_image_filename_value("image", "image.png"));
        assert!(is_bare_image_filename_value("IMAGE", "image.png"));
    }

    #[test]
    fn normalize_mcp_tool_args_rewrites_analyze_image_filename_to_absolute_path() {
        let workspace = unique_temp_dir("mcp-path-gateway-image-single");
        let image_dir = workspace.join(".ah").join("image-recognition");
        fs::create_dir_all(&image_dir).expect("should create image-recognition dir");
        let image_path = image_dir.join("demo.png");
        fs::write(&image_path, "img").expect("should write image file");

        let contract = McpToolContract::compile(
            "zai-mcp-server",
            "analyze_image",
            &serde_json::json!({
                "type": "object",
                "properties": {
                    "image": { "type": "string", "description": "Image file path" }
                }
            }),
        );
        let ctx = tool_context_for_workspace(&workspace);
        let args = serde_json::json!({ "image": "demo.png" })
            .as_object()
            .cloned()
            .expect("args should be object");

        let normalized =
            normalize_mcp_tool_args(&contract, args, &ctx).expect("normalization should succeed");
        let expected = absolute_path(&image_path).to_string_lossy().to_string();
        assert_eq!(
            normalized.get("image").and_then(Value::as_str),
            Some(expected.as_str())
        );

        let _ = fs::remove_dir_all(workspace);
    }

    #[test]
    fn normalize_mcp_tool_args_rejects_when_image_filename_not_found() {
        let workspace = unique_temp_dir("mcp-path-gateway-image-missing");
        fs::create_dir_all(workspace.join(".ah").join("image-recognition"))
            .expect("should create image-recognition dir");

        let contract = McpToolContract::compile(
            "zai-mcp-server",
            "analyze_image",
            &serde_json::json!({
                "type": "object",
                "properties": { "image": { "type": "string" } }
            }),
        );
        let ctx = tool_context_for_workspace(&workspace);
        let args = serde_json::json!({ "image": "missing.png" })
            .as_object()
            .cloned()
            .expect("args should be object");

        let err =
            normalize_mcp_tool_args(&contract, args, &ctx).expect_err("normalization should fail");
        assert!(err.to_string().contains("mcp_path_gateway_rejected"));
        assert!(err.to_string().contains("not found"));

        let _ = fs::remove_dir_all(workspace);
    }

    #[test]
    fn normalize_mcp_tool_args_rejects_ambiguous_image_filename() {
        let workspace = unique_temp_dir("mcp-path-gateway-image-ambiguous");
        let image_dir = workspace.join(".ah").join("image-recognition");
        let screenshots_dir = workspace.join(".ah").join("screenshots");
        fs::create_dir_all(&image_dir).expect("should create image-recognition dir");
        fs::create_dir_all(&screenshots_dir).expect("should create screenshots dir");
        fs::write(image_dir.join("dup.png"), "img").expect("should write image file");
        fs::write(screenshots_dir.join("dup.png"), "img").expect("should write screenshot file");

        let contract = McpToolContract::compile(
            "zai-mcp-server",
            "analyze_image",
            &serde_json::json!({
                "type": "object",
                "properties": { "image": { "type": "string" } }
            }),
        );
        let ctx = tool_context_for_workspace(&workspace);
        let args = serde_json::json!({ "image": "dup.png" })
            .as_object()
            .cloned()
            .expect("args should be object");

        let err =
            normalize_mcp_tool_args(&contract, args, &ctx).expect_err("normalization should fail");
        assert!(err.to_string().contains("ambiguous"));

        let _ = fs::remove_dir_all(workspace);
    }

    #[test]
    fn normalize_mcp_tool_args_rewrites_non_standard_key_for_image_tool() {
        let workspace = unique_temp_dir("mcp-path-gateway-image-nonstandard-key");
        let image_dir = workspace.join(".ah").join("image-recognition");
        fs::create_dir_all(&image_dir).expect("should create image-recognition dir");
        let image_path = image_dir.join("vision.png");
        fs::write(&image_path, "img").expect("should write image file");

        let contract = McpToolContract::compile(
            "zai-mcp-server",
            "vision_scan",
            &serde_json::json!({
                "type": "object",
                "properties": { "input": { "type": "string" } }
            }),
        );
        let ctx = tool_context_for_workspace(&workspace);
        let args = serde_json::json!({ "input": "vision.png" })
            .as_object()
            .cloned()
            .expect("args should be object");

        let normalized =
            normalize_mcp_tool_args(&contract, args, &ctx).expect("normalization should succeed");
        let expected = absolute_path(&image_path).to_string_lossy().to_string();
        assert_eq!(
            normalized.get("input").and_then(Value::as_str),
            Some(expected.as_str())
        );

        let _ = fs::remove_dir_all(workspace);
    }

    #[test]
    fn normalize_mcp_tool_args_does_not_treat_prompt_field_as_image_path() {
        let workspace = unique_temp_dir("mcp-path-gateway-image-prompt-not-path");
        let image_dir = workspace.join(".ah").join("image-recognition");
        fs::create_dir_all(&image_dir).expect("should create image-recognition dir");
        fs::write(image_dir.join("image.png"), "img").expect("should write image file");

        let contract = McpToolContract::compile(
            "zai-mcp-server",
            "analyze_image",
            &serde_json::json!({
                "type": "object",
                "properties": {
                    "image_source": { "type": "string", "description": "Image file path to analyze." },
                    "prompt": { "type": "string", "description": "Prompt text for image analysis." }
                }
            }),
        );
        let ctx = tool_context_for_workspace(&workspace);
        let prompt_text = "Please describe all visible details in this image.";
        let args = serde_json::json!({
            "image_source": "image.png",
            "prompt": prompt_text
        })
        .as_object()
        .cloned()
        .expect("args should be object");

        let normalized =
            normalize_mcp_tool_args(&contract, args, &ctx).expect("normalization should succeed");
        assert_eq!(
            normalized.get("prompt").and_then(Value::as_str),
            Some(prompt_text)
        );

        let _ = fs::remove_dir_all(workspace);
    }

    #[test]
    fn normalize_mcp_tool_args_falls_back_to_single_generic_image_candidate() {
        let workspace = unique_temp_dir("mcp-path-gateway-image-single-fallback");
        let image_dir = workspace.join(".ah").join("image-recognition");
        fs::create_dir_all(&image_dir).expect("should create image-recognition dir");
        let image_path = image_dir.join("capture-raw.bin");
        fs::write(&image_path, "img").expect("should write fallback image file");

        let contract = McpToolContract::compile(
            "zai-mcp-server",
            "analyze_image",
            &serde_json::json!({
                "type": "object",
                "properties": { "image_source": { "type": "string" } }
            }),
        );
        let ctx = tool_context_for_workspace(&workspace);
        let args = serde_json::json!({ "image_source": "image.png" })
            .as_object()
            .cloned()
            .expect("args should be object");

        let normalized =
            normalize_mcp_tool_args(&contract, args, &ctx).expect("normalization should succeed");
        assert_eq!(
            normalized.get("image_source").and_then(Value::as_str),
            Some(absolute_path(&image_path).to_string_lossy().as_ref())
        );

        let _ = fs::remove_dir_all(workspace);
    }

    #[test]
    fn normalize_mcp_tool_args_resolves_relative_path_to_workspace_absolute() {
        let workspace = unique_temp_dir("mcp-path-gateway-relative");
        fs::create_dir_all(&workspace).expect("should create workspace dir");

        let contract = McpToolContract::compile(
            "zai-mcp-server",
            "read_file",
            &serde_json::json!({
                "type": "object",
                "properties": { "path": { "type": "string" } }
            }),
        );
        let ctx = tool_context_for_workspace(&workspace);
        let args = serde_json::json!({ "path": "./a/b.png" })
            .as_object()
            .cloned()
            .expect("args should be object");

        let normalized =
            normalize_mcp_tool_args(&contract, args, &ctx).expect("normalization should succeed");
        let expected = absolute_path(&workspace.join("a").join("b.png"))
            .to_string_lossy()
            .to_string();
        assert_eq!(
            normalized.get("path").and_then(Value::as_str),
            Some(expected.as_str())
        );

        let _ = fs::remove_dir_all(workspace);
    }

    #[test]
    fn normalize_mcp_tool_args_rejects_absolute_path_outside_workspace() {
        let workspace = unique_temp_dir("mcp-path-gateway-outside-workspace");
        let outside_dir = unique_temp_dir("mcp-path-gateway-outside-source");
        fs::create_dir_all(&workspace).expect("should create workspace dir");
        fs::create_dir_all(&outside_dir).expect("should create outside dir");
        let outside_path = outside_dir.join("outside.txt");
        fs::write(&outside_path, "x").expect("should write outside file");

        let contract = McpToolContract::compile(
            "zai-mcp-server",
            "read_file",
            &serde_json::json!({
                "type": "object",
                "properties": { "path": { "type": "string" } }
            }),
        );
        let ctx = tool_context_for_workspace(&workspace);
        let args = serde_json::json!({ "path": outside_path.to_string_lossy().to_string() })
            .as_object()
            .cloned()
            .expect("args should be object");

        let err =
            normalize_mcp_tool_args(&contract, args, &ctx).expect_err("normalization should fail");
        assert!(err.to_string().contains("outside workspace"));

        let _ = fs::remove_dir_all(workspace);
        let _ = fs::remove_dir_all(outside_dir);
    }

    #[test]
    fn normalize_mcp_tool_args_does_not_rewrite_non_path_text() {
        let workspace = unique_temp_dir("mcp-path-gateway-non-path-text");
        fs::create_dir_all(&workspace).expect("should create workspace dir");

        let contract = McpToolContract::compile(
            "zai-mcp-server",
            "search_docs",
            &serde_json::json!({
                "type": "object",
                "properties": { "query": { "type": "string" } }
            }),
        );
        let ctx = tool_context_for_workspace(&workspace);
        let args = serde_json::json!({ "query": "user guide / troubleshooting steps" })
            .as_object()
            .cloned()
            .expect("args should be object");

        let normalized =
            normalize_mcp_tool_args(&contract, args, &ctx).expect("normalization should succeed");
        assert_eq!(
            normalized.get("query").and_then(Value::as_str),
            Some("user guide / troubleshooting steps")
        );

        let _ = fs::remove_dir_all(workspace);
    }

    #[test]
    fn normalize_mcp_tool_args_handles_backslash_relative_path() {
        let workspace = unique_temp_dir("mcp-path-gateway-backslash-path");
        fs::create_dir_all(&workspace).expect("should create workspace dir");

        let contract = McpToolContract::compile(
            "zai-mcp-server",
            "read_file",
            &serde_json::json!({
                "type": "object",
                "properties": { "path": { "type": "string" } }
            }),
        );
        let ctx = tool_context_for_workspace(&workspace);
        let args = serde_json::json!({ "path": "folder\\file.txt" })
            .as_object()
            .cloned()
            .expect("args should be object");

        let normalized =
            normalize_mcp_tool_args(&contract, args, &ctx).expect("normalization should succeed");
        let path = normalized
            .get("path")
            .and_then(Value::as_str)
            .map(PathBuf::from)
            .expect("path should be string");
        assert!(path.is_absolute());
        assert!(path.starts_with(absolute_path(&workspace)));

        let _ = fs::remove_dir_all(workspace);
    }

    #[test]
    fn normalize_mcp_tool_args_rewrites_nested_array_path_fields() {
        let workspace = unique_temp_dir("mcp-path-gateway-nested-array");
        fs::create_dir_all(&workspace).expect("should create workspace dir");

        let contract = McpToolContract::compile(
            "zai-mcp-server",
            "bulk_read",
            &serde_json::json!({
                "type": "object",
                "properties": {
                    "files": {
                        "type": "array",
                        "items": {
                            "type": "object",
                            "properties": {
                                "path": { "type": "string" }
                            }
                        }
                    }
                }
            }),
        );
        let ctx = tool_context_for_workspace(&workspace);
        let args = serde_json::json!({
            "files": [
                { "path": "./a.txt" },
                { "path": "./b.txt" }
            ]
        })
        .as_object()
        .cloned()
        .expect("args should be object");

        let normalized =
            normalize_mcp_tool_args(&contract, args, &ctx).expect("normalization should succeed");
        let first = normalized["files"][0]["path"]
            .as_str()
            .expect("first path should be string");
        let second = normalized["files"][1]["path"]
            .as_str()
            .expect("second path should be string");

        assert_eq!(
            first,
            absolute_path(&workspace.join("a.txt"))
                .to_string_lossy()
                .as_ref()
        );
        assert_eq!(
            second,
            absolute_path(&workspace.join("b.txt"))
                .to_string_lossy()
                .as_ref()
        );

        let _ = fs::remove_dir_all(workspace);
    }

    #[test]
    fn normalize_mcp_tool_args_rejects_image_path_outside_ah_dirs() {
        let workspace = unique_temp_dir("mcp-path-gateway-image-outside-ah");
        let image_dir = workspace.join("images");
        fs::create_dir_all(&image_dir).expect("should create image dir");
        let outside_image = image_dir.join("outside.png");
        fs::write(&outside_image, "img").expect("should write image file");

        let contract = McpToolContract::compile(
            "zai-mcp-server",
            "analyze_image",
            &serde_json::json!({
                "type": "object",
                "properties": { "image": { "type": "string" } }
            }),
        );
        let ctx = tool_context_for_workspace(&workspace);
        let args = serde_json::json!({ "image": outside_image.to_string_lossy().to_string() })
            .as_object()
            .cloned()
            .expect("args should be object");

        let err =
            normalize_mcp_tool_args(&contract, args, &ctx).expect_err("normalization should fail");
        assert!(err.to_string().contains("outside allowed dirs"));

        let _ = fs::remove_dir_all(workspace);
    }

    #[test]
    fn tool_call_requested_preview_normalizes_image_source_for_display() {
        let workspace = unique_temp_dir("mcp-path-gateway-preview-display");
        let image_dir = workspace.join(".ah").join("image-recognition");
        fs::create_dir_all(&image_dir).expect("should create image-recognition dir");
        let image_path = image_dir.join("image.png");
        fs::write(&image_path, "img").expect("should write image file");

        let raw_args = serde_json::json!({
            "image_source": "image.png",
            "prompt": "describe image"
        });

        let preview = normalize_tool_call_requested_args_for_display(
            "mcp:zai-mcp-server:analyze_image",
            &raw_args,
            Some(&workspace),
        );

        assert_eq!(
            preview["image_source"].as_str(),
            Some(absolute_path(&image_path).to_string_lossy().as_ref())
        );
        assert_eq!(preview["prompt"].as_str(), Some("describe image"));

        let _ = fs::remove_dir_all(workspace);
    }

    #[test]
    fn map_protocol_event_normalizes_tool_call_requested_payload_for_display() {
        let workspace = unique_temp_dir("mcp-path-gateway-map-protocol-event");
        let image_dir = workspace.join(".ah").join("image-recognition");
        fs::create_dir_all(&image_dir).expect("should create image-recognition dir");
        let image_path = image_dir.join("image.png");
        fs::write(&image_path, "img").expect("should write image file");

        let event = Event::ToolCallRequested {
            tool: "mcp:zai-mcp-server:analyze_image".to_string(),
            args: serde_json::json!({
                "image_source": "image.png",
                "prompt": "describe image"
            }),
        };

        let payload = map_protocol_event(&event, Some(&workspace)).expect("payload should exist");
        match payload {
            ProtocolEventPayload::ToolCallRequested { tool, args } => {
                assert_eq!(tool, "mcp:zai-mcp-server:analyze_image");
                assert_eq!(
                    args["image_source"].as_str(),
                    Some(absolute_path(&image_path).to_string_lossy().as_ref())
                );
                assert_eq!(args["prompt"].as_str(), Some("describe image"));
            }
            _ => panic!("expected tool_call_requested payload"),
        }

        let _ = fs::remove_dir_all(workspace);
    }

    #[test]
    fn default_workspace_root_uses_expected_suffix() {
        let root = default_workspace_root();
        assert!(root.is_absolute());
        assert!(root.ends_with(Path::new(".ai-helper").join("workspaces").join("default")));
    }

    #[test]
    fn workspace_context_falls_back_when_configured_path_is_file() {
        let temp_dir = unique_temp_dir("workspace-invalid");
        fs::create_dir_all(&temp_dir).expect("should create temp dir");
        let file_path = temp_dir.join("not-a-directory");
        fs::write(&file_path, "x").expect("should create file");

        let config = WorkspaceRuntimeConfig {
            root_dir: file_path.to_string_lossy().to_string(),
        };
        let context = WorkspaceContext::resolve(&config);

        assert!(!context.warnings.is_empty());
        assert_ne!(context.root_dir, absolute_path(&file_path));
        let _ = fs::remove_dir_all(temp_dir);
    }

    #[test]
    fn workspace_context_creates_ah_directories() {
        let root = unique_temp_dir("workspace-layout");
        let config = WorkspaceRuntimeConfig {
            root_dir: root.to_string_lossy().to_string(),
        };

        let context = WorkspaceContext::resolve(&config);
        assert!(context.ah_dir.exists());
        assert!(context.config_dir.exists());
        assert!(context.screenshots_dir.exists());
        assert!(context.tmp_dir.exists());
        assert!(context.image_recognition_dir.exists());

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn map_runtime_mcp_server_config_injects_workspace_env_and_rewrites_paths() {
        let root = unique_temp_dir("workspace-mcp");
        fs::create_dir_all(root.join("scripts")).expect("should create scripts dir");
        fs::create_dir_all(root.join("data")).expect("should create data dir");
        fs::write(root.join("scripts").join("mcp.cmd"), "@echo off")
            .expect("should create command");

        let context = WorkspaceContext::from_root(root.clone());
        context
            .ensure_layout()
            .expect("workspace layout should exist");

        let server = McpServerRuntimeConfig {
            name: "demo".to_string(),
            enabled: true,
            command: Some("./scripts/mcp.cmd".to_string()),
            args: vec!["./data".to_string(), "--safe".to_string()],
            ..Default::default()
        };

        let mapped = map_runtime_mcp_server_config(&server, 30, &context, Path::new("."))
            .expect("mapping should succeed");

        let command = mapped.command.unwrap_or_default();
        assert!(command.contains("scripts"));
        assert!(mapped.args[0].contains("data"));
        assert_eq!(
            mapped.env.get(AH_WORKSPACE_DIR_ENV).map(String::as_str),
            Some(context.root_dir.to_string_lossy().as_ref())
        );
        assert_eq!(
            mapped
                .env
                .get(AH_IMAGE_RECOGNITION_DIR_ENV)
                .map(String::as_str),
            Some(context.image_recognition_dir.to_string_lossy().as_ref())
        );

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn build_stage2_guidance_context_includes_workspace_and_mcp_contracts() {
        let root = unique_temp_dir("guidance-context");
        let workspace_context = WorkspaceContext::from_root(root.clone());
        workspace_context
            .ensure_layout()
            .expect("workspace layout should exist");

        let contract = McpToolContract::compile(
            "zai-mcp-server",
            "analyze_image",
            &serde_json::json!({
                "type": "object",
                "properties": {
                    "image_source": { "type": "string", "description": "Image file path." },
                    "prompt": { "type": "string", "description": "Prompt text for image analysis." },
                    "metadata_path": { "type": "string", "description": "metadata file path" }
                }
            }),
        );
        let runtime_resources = RuntimeResources {
            mcp_manager: None,
            tool_executor: None,
            skill_config: SkillConfig {
                enabled: true,
                personal_dir: Some(root.join("skills").join("personal")),
                project_dirs: vec![root.join("skills").join("project")],
                auto_apply: false,
            },
            mcp_tool_contracts: vec![contract],
        };
        let guidance_snapshot = Stage2GuidanceSnapshot {
            mcp_servers: vec![Stage2McpServerSnapshot {
                server_name: "zai-mcp-server".to_string(),
                tool_count: 3,
                resource_count: 2,
                prompt_count: 1,
                resource_names: vec!["workspace://repo/readme".to_string()],
                prompts: vec![Stage2McpPromptSnapshot {
                    name: "analyze_image".to_string(),
                    arguments: vec![
                        Stage2McpPromptArgumentSnapshot {
                            name: "image_source".to_string(),
                            description: "Local image path".to_string(),
                            required: true,
                        },
                        Stage2McpPromptArgumentSnapshot {
                            name: "prompt".to_string(),
                            description: "Prompt text for image analysis.".to_string(),
                            required: true,
                        },
                    ],
                }],
            }],
            skills: vec![Stage2SkillSnapshot {
                name: "workspace-helper".to_string(),
                source: "project".to_string(),
                path: root
                    .join(".cursor")
                    .join("skills")
                    .join("workspace-helper")
                    .join("SKILL.md")
                    .to_string_lossy()
                    .to_string(),
            }],
            warnings: vec!["sample guidance warning".to_string()],
        };
        let focus = Stage2GuidanceFocus::default();

        let guidance = build_stage2_guidance_context(
            &workspace_context,
            &runtime_resources,
            &guidance_snapshot,
            &focus,
        );
        assert!(guidance.contains(workspace_context.root_dir.to_string_lossy().as_ref()));
        assert!(guidance.contains("mcp:zai-mcp-server:analyze_image"));
        assert!(guidance.contains("image_source"));
        assert!(guidance.contains("metadata_path"));
        assert!(!guidance.contains("prompt]"));
        assert!(guidance.contains("zai-mcp-server"));
        assert!(guidance.contains("MCP tool argument templates"));
        assert!(guidance.contains("MCP prompt argument templates"));
        assert!(guidance.contains("image_source"));
        assert!(guidance.contains("<text-required>"));
        assert!(guidance.contains("workspace-helper"));
        assert!(guidance.contains("sample guidance warning"));

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn build_stage2_guidance_focus_selects_image_contracts_and_prompts() {
        let contract_image = McpToolContract::compile(
            "zai-mcp-server",
            "analyze_image",
            &serde_json::json!({
                "type": "object",
                "properties": {
                    "image_source": { "type": "string", "description": "Image file path." },
                    "prompt": { "type": "string", "description": "Prompt text for image analysis." }
                }
            }),
        );
        let contract_generic = McpToolContract::compile(
            "workspace-tools",
            "read_file",
            &serde_json::json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "workspace file path" }
                }
            }),
        );
        let runtime_resources = RuntimeResources {
            mcp_manager: None,
            tool_executor: None,
            skill_config: SkillConfig {
                enabled: false,
                personal_dir: None,
                project_dirs: Vec::new(),
                auto_apply: false,
            },
            mcp_tool_contracts: vec![contract_image, contract_generic],
        };
        let snapshot = Stage2GuidanceSnapshot {
            mcp_servers: vec![Stage2McpServerSnapshot {
                server_name: "zai-mcp-server".to_string(),
                tool_count: 1,
                resource_count: 0,
                prompt_count: 1,
                resource_names: Vec::new(),
                prompts: vec![Stage2McpPromptSnapshot {
                    name: "analyze_image".to_string(),
                    arguments: vec![
                        Stage2McpPromptArgumentSnapshot {
                            name: "image_source".to_string(),
                            description: "Absolute image file path".to_string(),
                            required: true,
                        },
                        Stage2McpPromptArgumentSnapshot {
                            name: "prompt".to_string(),
                            description: "Analysis prompt text".to_string(),
                            required: true,
                        },
                    ],
                }],
            }],
            skills: Vec::new(),
            warnings: Vec::new(),
        };

        let focus = build_stage2_guidance_focus(
            "请帮我分析 image.png 的截图内容",
            &runtime_resources,
            &snapshot,
        );
        assert!(focus.signals.contains("image_intent"));
        assert!(focus
            .selected_contract_names
            .contains("mcp:zai-mcp-server:analyze_image"));
        assert!(focus
            .selected_prompt_keys
            .contains("zai-mcp-server:analyze_image"));
    }

    #[test]
    fn build_stage2_guidance_context_filters_contracts_and_prompts_by_focus() {
        let root = unique_temp_dir("guidance-focus");
        let workspace_context = WorkspaceContext::from_root(root.clone());
        workspace_context
            .ensure_layout()
            .expect("workspace layout should exist");

        let contract_image = McpToolContract::compile(
            "zai-mcp-server",
            "analyze_image",
            &serde_json::json!({
                "type": "object",
                "properties": {
                    "image_source": { "type": "string", "description": "Image file path." }
                }
            }),
        );
        let contract_file = McpToolContract::compile(
            "workspace-tools",
            "read_file",
            &serde_json::json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "workspace file path" }
                }
            }),
        );
        let runtime_resources = RuntimeResources {
            mcp_manager: None,
            tool_executor: None,
            skill_config: SkillConfig {
                enabled: false,
                personal_dir: None,
                project_dirs: Vec::new(),
                auto_apply: false,
            },
            mcp_tool_contracts: vec![contract_image, contract_file],
        };
        let snapshot = Stage2GuidanceSnapshot {
            mcp_servers: vec![
                Stage2McpServerSnapshot {
                    server_name: "zai-mcp-server".to_string(),
                    tool_count: 1,
                    resource_count: 0,
                    prompt_count: 1,
                    resource_names: Vec::new(),
                    prompts: vec![Stage2McpPromptSnapshot {
                        name: "analyze_image".to_string(),
                        arguments: vec![Stage2McpPromptArgumentSnapshot {
                            name: "image_source".to_string(),
                            description: "Absolute image file path".to_string(),
                            required: true,
                        }],
                    }],
                },
                Stage2McpServerSnapshot {
                    server_name: "workspace-tools".to_string(),
                    tool_count: 1,
                    resource_count: 0,
                    prompt_count: 1,
                    resource_names: Vec::new(),
                    prompts: vec![Stage2McpPromptSnapshot {
                        name: "read_file".to_string(),
                        arguments: vec![Stage2McpPromptArgumentSnapshot {
                            name: "path".to_string(),
                            description: "workspace path".to_string(),
                            required: true,
                        }],
                    }],
                },
            ],
            skills: Vec::new(),
            warnings: Vec::new(),
        };
        let mut focus = Stage2GuidanceFocus::default();
        focus
            .selected_contract_names
            .insert("mcp:zai-mcp-server:analyze_image".to_string());
        focus
            .selected_server_names
            .insert("zai-mcp-server".to_string());
        focus
            .selected_prompt_keys
            .insert("zai-mcp-server:analyze_image".to_string());
        focus.signals.insert("image_intent".to_string());

        let guidance = build_stage2_guidance_context(
            &workspace_context,
            &runtime_resources,
            &snapshot,
            &focus,
        );
        assert!(guidance.contains("mcp:zai-mcp-server:analyze_image"));
        assert!(!guidance.contains("mcp:workspace-tools:read_file"));
        assert!(guidance.contains("zai-mcp-server:analyze_image"));
        assert!(!guidance.contains("workspace-tools:read_file"));
        assert!(guidance.contains("Guidance focus signals: image_intent"));

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn merge_prompt_directives_with_guidance_appends_context() {
        let config = AgentRuntimeConfig::default();
        let directives = Some(PromptDirectives {
            developer_instructions: Some("base instructions".to_string()),
            user_instructions: None,
        });

        let merged = merge_prompt_directives_with_guidance(
            &config,
            directives,
            "[Guidance]\nworkspace=demo",
        );
        let developer = merged.developer_instructions.unwrap_or_default();
        assert!(developer.contains("base instructions"));
        assert!(developer.contains("[Guidance]"));
        assert!(developer.contains("workspace=demo"));
    }

    #[test]
    fn resolve_preferred_path_uses_fallback_when_workspace_missing() {
        let workspace = unique_temp_dir("workspace-primary");
        let fallback = unique_temp_dir("workspace-fallback");
        fs::create_dir_all(fallback.join("skills").join("project"))
            .expect("should create fallback path");

        let resolved = resolve_preferred_path("skills/project", &workspace, &fallback);
        assert!(resolved.starts_with(&fallback));

        let _ = fs::remove_dir_all(workspace);
        let _ = fs::remove_dir_all(fallback);
    }

    #[test]
    fn persist_image_for_recognition_writes_inside_workspace_path() {
        let image_dir = unique_temp_dir("workspace-image");
        fs::create_dir_all(&image_dir).expect("should create image dir");
        let image = AgentInputImage {
            name: "sample.png".to_string(),
            mime_type: "image/png".to_string(),
            data_url: "data:image/png;base64,AAAA".to_string(),
            size_bytes: 4,
        };

        let saved =
            persist_image_for_recognition(&image, &image_dir).expect("image should be persisted");
        assert!(saved.starts_with(&image_dir));
        assert!(saved.exists());

        let _ = fs::remove_dir_all(image_dir);
    }

    #[test]
    fn persist_image_for_recognition_uses_filename_extension_when_mime_unknown() {
        let image_dir = unique_temp_dir("workspace-image-ext-fallback");
        fs::create_dir_all(&image_dir).expect("should create image dir");
        let image = AgentInputImage {
            name: "capture.png".to_string(),
            mime_type: "application/octet-stream".to_string(),
            data_url: "data:image/png;base64,AAAA".to_string(),
            size_bytes: 4,
        };

        let saved =
            persist_image_for_recognition(&image, &image_dir).expect("image should be persisted");
        assert_eq!(
            saved
                .extension()
                .and_then(|ext| ext.to_str())
                .map(str::to_ascii_lowercase),
            Some("png".to_string())
        );

        let _ = fs::remove_dir_all(image_dir);
    }

    #[tokio::test]
    async fn build_runtime_resources_without_mcp_registers_builtin_tools() {
        let config = AgentRuntimeConfig::default();

        let workspace_context = WorkspaceContext::resolve(&config.workspace);
        let runtime = build_runtime_resources(&config, &workspace_context)
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
        let workspace = unique_temp_dir("image-fallback-note-with-local-path");
        config.workspace.root_dir = workspace.to_string_lossy().to_string();

        let workspace_context = WorkspaceContext::resolve(&config.workspace);
        let prepared =
            prepare_image_fallback_payload(&input, &config, None, &workspace_context).await;
        let payload_text = prepared.model_payload_text.unwrap_or_default();

        assert!(payload_text.contains("User attached image files:"));
        assert!(payload_text.contains("local_path="));
        assert!(!prepared.warnings.is_empty());

        let _ = fs::remove_dir_all(workspace);
    }

    #[tokio::test]
    async fn prepare_image_fallback_payload_persists_images_even_when_model_supports_image_input() {
        let input = AgentStreamInput {
            content: "look at this".to_string(),
            images: vec![AgentInputImage {
                name: "image.png".to_string(),
                mime_type: "image/png".to_string(),
                data_url: "data:image/png;base64,AAAA".to_string(),
                size_bytes: 4,
            }],
            conversation_id: None,
            recent_messages: Vec::new(),
        };

        let mut config = AgentRuntimeConfig::default();
        config.model_supports_image_input = true;
        let workspace = unique_temp_dir("image-fallback-persist-when-mm-on");
        config.workspace.root_dir = workspace.to_string_lossy().to_string();

        let workspace_context = WorkspaceContext::resolve(&config.workspace);
        let prepared =
            prepare_image_fallback_payload(&input, &config, None, &workspace_context).await;

        assert!(prepared.model_payload_text.is_none());
        assert!(workspace_context.image_recognition_dir.exists());
        let saved_files = std::fs::read_dir(&workspace_context.image_recognition_dir)
            .expect("should read image-recognition dir")
            .filter_map(Result::ok)
            .count();
        assert!(saved_files > 0);

        let _ = fs::remove_dir_all(workspace);
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
    if let Ok(workspace_root) = std::env::var("AGENT_WORKSPACE_ROOT") {
        config.workspace.root_dir = workspace_root;
    }

    if let Ok(workspace_config_json) = std::env::var("AGENT_WORKSPACE_CONFIG_JSON") {
        match serde_json::from_str::<WorkspaceRuntimeConfig>(&workspace_config_json) {
            Ok(workspace_config) => {
                config.workspace = workspace_config;
            }
            Err(err) => {
                log::warn!("failed to parse AGENT_WORKSPACE_CONFIG_JSON: {}", err);
            }
        }
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
