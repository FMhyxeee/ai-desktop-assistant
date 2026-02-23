use std::sync::{Arc, Mutex};
use std::time::Duration;

use ai_desktop_assistant_lib::agent_service::types::{
    AgentEvent, AgentProvider, AgentRuntimeConfig, AgentStreamInput, McpRuntimeConfig,
    McpServerRuntimeConfig, McpTransportKind, ProtocolEventPayload, SkillsRuntimeConfig,
    WorkspaceRuntimeConfig,
};
use ai_desktop_assistant_lib::agent_service::{
    run_governance_scan_with_config, scan_skills_runtime_config, test_mcp_runtime_config,
    AgentService, Runner,
};

struct MockRunner {
    output: String,
    delay_ms: u64,
}

#[async_trait::async_trait]
impl Runner for MockRunner {
    async fn run(&self, _prompt: &str) -> agent_lib::AgentResult<String> {
        if self.delay_ms > 0 {
            tokio::time::sleep(Duration::from_millis(self.delay_ms)).await;
        }
        Ok(self.output.clone())
    }
}

fn default_local_runtime_config() -> AgentRuntimeConfig {
    AgentRuntimeConfig {
        provider: AgentProvider::Local,
        model: "qwen2.5-coder:7b".to_string(),
        model_supports_image_input: true,
        api_key_env: "LOCAL_API_KEY".to_string(),
        api_key: None,
        base_url: None,
        max_tokens: None,
        workspace: WorkspaceRuntimeConfig::default(),
        mcp: McpRuntimeConfig::default(),
        skills: SkillsRuntimeConfig::default(),
        memory: Default::default(),
        control: Default::default(),
    }
}

async fn collect_stream_events(
    service: &AgentService,
    task_id: &str,
    input: &str,
) -> Vec<AgentEvent> {
    let events: Arc<Mutex<Vec<AgentEvent>>> = Arc::new(Mutex::new(Vec::new()));
    let events_ref = Arc::clone(&events);

    service
        .chat_stream(
            task_id.to_string(),
            AgentStreamInput::text(input),
            move |event| {
                events_ref.lock().unwrap().push(event);
            },
        )
        .await
        .unwrap();

    tokio::time::sleep(Duration::from_millis(180)).await;
    let snapshot = events.lock().unwrap().clone();
    snapshot
}

#[tokio::test]
async fn chat_returns_runner_output() {
    let runner = Arc::new(MockRunner {
        output: "ok".to_string(),
        delay_ms: 0,
    });
    let service = AgentService::with_runner(runner);
    let result = service.chat("hello".to_string()).await.unwrap();
    assert_eq!(result, "ok");
}

#[tokio::test]
async fn chat_stream_emits_lifecycle_events() {
    let runner = Arc::new(MockRunner {
        output: "stream-output".to_string(),
        delay_ms: 0,
    });
    let service = AgentService::with_runner(runner);

    let events: Arc<Mutex<Vec<AgentEvent>>> = Arc::new(Mutex::new(Vec::new()));
    let events_ref = Arc::clone(&events);

    service
        .chat_stream(
            "task-1".to_string(),
            AgentStreamInput::text("hi"),
            move |event| {
                events_ref.lock().unwrap().push(event);
            },
        )
        .await
        .unwrap();

    tokio::time::sleep(Duration::from_millis(50)).await;

    let events = events.lock().unwrap();
    assert!(matches!(events.first(), Some(AgentEvent::Started { .. })));
    assert!(events
        .iter()
        .any(|event| matches!(event, AgentEvent::Completed { .. })));
}

#[tokio::test]
async fn cancel_existing_task_succeeds() {
    let runner = Arc::new(MockRunner {
        output: "late-output".to_string(),
        delay_ms: 800,
    });
    let service = AgentService::with_runner(runner);

    service
        .chat_stream(
            "task-2".to_string(),
            AgentStreamInput::text("hi"),
            |_event| {},
        )
        .await
        .unwrap();

    tokio::time::sleep(Duration::from_millis(30)).await;
    service.cancel("task-2").await.unwrap();
}

#[tokio::test]
async fn test_mcp_runtime_config_disabled_returns_success() {
    let result = test_mcp_runtime_config(McpRuntimeConfig::default()).await;
    assert!(result.success);
    assert!(result.server_results.is_empty());
}

#[tokio::test]
async fn test_mcp_runtime_config_invalid_server_returns_error() {
    let config = McpRuntimeConfig {
        enabled: true,
        default_timeout_secs: Some(1),
        max_retries: Some(0),
        image_recognition: Default::default(),
        servers: vec![McpServerRuntimeConfig {
            name: "bad-stdio".to_string(),
            enabled: true,
            command: None,
            endpoint: None,
            ..Default::default()
        }],
    };

    let result = test_mcp_runtime_config(config).await;
    assert!(!result.success);
    assert_eq!(result.server_results.len(), 1);
    assert!(!result.server_results[0].success);
    assert!(result.server_results[0].error.is_some());
}

#[tokio::test]
async fn test_mcp_runtime_config_legacy_transport_returns_migration_error() {
    let config = McpRuntimeConfig {
        enabled: true,
        default_timeout_secs: Some(1),
        max_retries: Some(0),
        image_recognition: Default::default(),
        servers: vec![McpServerRuntimeConfig {
            name: "legacy-tcp".to_string(),
            enabled: true,
            transport: McpTransportKind::Tcp,
            endpoint: Some("tcp://localhost:9000".to_string()),
            ..Default::default()
        }],
    };

    let result = test_mcp_runtime_config(config).await;
    assert!(!result.success);
    assert_eq!(result.server_results.len(), 1);
    let error_text = result.server_results[0].error.clone().unwrap_or_default();
    assert!(error_text.contains("Unsupported transport 'tcp'"));
    assert!(error_text.contains("Supported: stdio, streamable_http"));
    assert!(error_text.contains("http/https -> streamable_http"));
    assert!(error_text.contains("tcp/ws/wss/sse are removed in strict official mode"));
}

#[tokio::test]
async fn test_mcp_runtime_config_http_alias_is_not_unsupported_transport() {
    let config = McpRuntimeConfig {
        enabled: true,
        default_timeout_secs: Some(1),
        max_retries: Some(0),
        image_recognition: Default::default(),
        servers: vec![McpServerRuntimeConfig {
            name: "http-alias".to_string(),
            enabled: true,
            transport: McpTransportKind::Http,
            endpoint: Some("http://127.0.0.1:1/mcp".to_string()),
            ..Default::default()
        }],
    };

    let result = test_mcp_runtime_config(config).await;
    assert!(!result.success);
    assert_eq!(result.server_results.len(), 1);
    let error_text = result.server_results[0].error.clone().unwrap_or_default();
    assert!(!error_text.contains("Unsupported transport"));
}

#[tokio::test]
async fn scan_skills_runtime_config_reads_custom_personal_dir() {
    let root = std::env::temp_dir().join(format!("assistant_skill_scan_{}", uuid::Uuid::new_v4()));
    let skill_dir = root.join("demo-skill");
    std::fs::create_dir_all(&skill_dir).unwrap();
    std::fs::write(
        skill_dir.join("SKILL.md"),
        r#"---
name: demo-skill
description: demo skill description
---

Skill body
"#,
    )
    .unwrap();

    let result = scan_skills_runtime_config(
        SkillsRuntimeConfig {
            enabled: true,
            personal_dir: Some(root.to_string_lossy().to_string()),
            project_dirs: vec![],
            auto_apply: false,
        },
        None,
    )
    .await;

    assert!(result.success);
    assert!(result.skills.iter().any(|skill| skill.name == "demo-skill"));
}

#[tokio::test]
async fn startup_stream_emits_governance_protocol_event() {
    let mut config = AgentRuntimeConfig {
        provider: AgentProvider::Local,
        model: "qwen2.5-coder:7b".to_string(),
        model_supports_image_input: true,
        api_key_env: "LOCAL_API_KEY".to_string(),
        api_key: None,
        base_url: None,
        max_tokens: None,
        workspace: WorkspaceRuntimeConfig::default(),
        mcp: McpRuntimeConfig::default(),
        skills: SkillsRuntimeConfig::default(),
        memory: Default::default(),
        control: Default::default(),
    };
    config.mcp.enabled = false;

    let service = AgentService::new_with_config(config).await.unwrap();
    let events: Arc<Mutex<Vec<AgentEvent>>> = Arc::new(Mutex::new(Vec::new()));
    let events_ref = Arc::clone(&events);

    service
        .chat_stream(
            "gov-task".to_string(),
            AgentStreamInput::text("/echo hello"),
            move |event| {
                events_ref.lock().unwrap().push(event);
            },
        )
        .await
        .unwrap();

    tokio::time::sleep(Duration::from_millis(200)).await;

    let events = events.lock().unwrap();
    let has_governance_report = events.iter().any(|event| {
        matches!(
            event,
            AgentEvent::ProtocolEvent {
                payload: ProtocolEventPayload::GovernanceReport { .. },
                ..
            }
        )
    });
    assert!(has_governance_report);
}

#[tokio::test]
async fn stream_emits_guidance_context_and_governance() {
    let mut config = AgentRuntimeConfig {
        provider: AgentProvider::Local,
        model: "qwen2.5-coder:7b".to_string(),
        model_supports_image_input: true,
        api_key_env: "LOCAL_API_KEY".to_string(),
        api_key: None,
        base_url: None,
        max_tokens: None,
        workspace: WorkspaceRuntimeConfig::default(),
        mcp: McpRuntimeConfig::default(),
        skills: SkillsRuntimeConfig::default(),
        memory: Default::default(),
        control: Default::default(),
    };
    config.mcp.enabled = false;

    let service = AgentService::new_with_config(config).await.unwrap();
    let events: Arc<Mutex<Vec<AgentEvent>>> = Arc::new(Mutex::new(Vec::new()));
    let events_ref = Arc::clone(&events);

    service
        .chat_stream(
            "guidance-task".to_string(),
            AgentStreamInput::text("/echo hello"),
            move |event| {
                events_ref.lock().unwrap().push(event);
            },
        )
        .await
        .unwrap();

    tokio::time::sleep(Duration::from_millis(250)).await;
    let events = events.lock().unwrap();
    let has_governance_report = events.iter().any(|event| {
        matches!(
            event,
            AgentEvent::ProtocolEvent {
                payload: ProtocolEventPayload::GovernanceReport { .. },
                ..
            }
        )
    });
    let has_guidance_context = events.iter().any(|event| {
        matches!(
            event,
            AgentEvent::ProtocolEvent {
                payload: ProtocolEventPayload::GuidanceContext { .. },
                ..
            }
        )
    });
    assert!(has_governance_report);
    assert!(has_guidance_context);
}

#[tokio::test]
async fn warning_compat_still_present_with_guidance_context() {
    let mut config = AgentRuntimeConfig {
        provider: AgentProvider::Local,
        model: "qwen2.5-coder:7b".to_string(),
        model_supports_image_input: true,
        api_key_env: "LOCAL_API_KEY".to_string(),
        api_key: None,
        base_url: None,
        max_tokens: None,
        workspace: WorkspaceRuntimeConfig::default(),
        mcp: McpRuntimeConfig::default(),
        skills: SkillsRuntimeConfig::default(),
        memory: Default::default(),
        control: Default::default(),
    };
    config.mcp.enabled = false;

    let service = AgentService::new_with_config(config).await.unwrap();
    let events: Arc<Mutex<Vec<AgentEvent>>> = Arc::new(Mutex::new(Vec::new()));
    let events_ref = Arc::clone(&events);

    service
        .chat_stream(
            "guidance-warning-task".to_string(),
            AgentStreamInput::text("/echo hello"),
            move |event| {
                events_ref.lock().unwrap().push(event);
            },
        )
        .await
        .unwrap();

    tokio::time::sleep(Duration::from_millis(250)).await;
    let events = events.lock().unwrap();
    let warning_messages = events
        .iter()
        .filter_map(|event| match event {
            AgentEvent::ProtocolEvent {
                payload: ProtocolEventPayload::Warning { message },
                ..
            } => Some(message.clone()),
            _ => None,
        })
        .collect::<Vec<_>>();
    let has_summary_warning = warning_messages
        .iter()
        .any(|message| message.contains("Guidance context injected"));
    let has_audit_warning = warning_messages
        .iter()
        .any(|message| message.contains("GuidanceAudit"));

    assert!(has_summary_warning);
    assert!(has_audit_warning);
}

#[tokio::test]
async fn governance_scan_flags_duplicate_mcp_server_names() {
    let mut config = AgentRuntimeConfig {
        provider: AgentProvider::Local,
        model: "qwen2.5-coder:7b".to_string(),
        model_supports_image_input: true,
        api_key_env: "LOCAL_API_KEY".to_string(),
        api_key: None,
        base_url: None,
        max_tokens: None,
        workspace: WorkspaceRuntimeConfig::default(),
        mcp: McpRuntimeConfig::default(),
        skills: SkillsRuntimeConfig::default(),
        memory: Default::default(),
        control: Default::default(),
    };
    config.mcp.enabled = true;
    config.mcp.servers = vec![
        McpServerRuntimeConfig {
            name: "dup".to_string(),
            enabled: true,
            command: Some("npx".to_string()),
            ..Default::default()
        },
        McpServerRuntimeConfig {
            name: "dup".to_string(),
            enabled: true,
            command: Some("npx".to_string()),
            ..Default::default()
        },
    ];

    let report = run_governance_scan_with_config(config).await;
    assert!(report.blocker_count > 0);
    assert!(report
        .issues
        .iter()
        .any(|issue| issue.code == "mcp_server_name_duplicate"));
}

#[tokio::test]
async fn proc_commands_manage_process_lifecycle() {
    let mut config = default_local_runtime_config();
    config.mcp.enabled = false;
    let service = AgentService::new_with_config(config).await.unwrap();

    let command = if cfg!(windows) {
        "echo proc-start && ping -n 4 127.0.0.1 >nul && echo proc-done"
    } else {
        "echo proc-start && sleep 2 && echo proc-done"
    };

    let start_events = collect_stream_events(
        &service,
        "proc-start-task",
        &format!("/proc start {command}"),
    )
    .await;

    let process_id = start_events
        .iter()
        .find_map(|event| match event {
            AgentEvent::ProtocolEvent {
                payload: ProtocolEventPayload::ProcessStarted { process },
                ..
            } => Some(process.id.clone()),
            _ => None,
        })
        .expect("expected ProcessStarted event");

    let list_events = collect_stream_events(&service, "proc-list-task", "/proc list").await;
    let listed = list_events.iter().any(|event| {
        matches!(
            event,
            AgentEvent::ProtocolEvent {
                payload: ProtocolEventPayload::ProcessList { processes },
                ..
            } if processes.iter().any(|item| item.id == process_id)
        )
    });
    assert!(listed, "process should appear in /proc list");

    tokio::time::sleep(Duration::from_millis(300)).await;
    let logs_events = collect_stream_events(
        &service,
        "proc-logs-task",
        &format!("/proc logs {process_id} 50"),
    )
    .await;
    let has_logs = logs_events.iter().any(|event| {
        matches!(
            event,
            AgentEvent::ProtocolEvent {
                payload: ProtocolEventPayload::ProcessLogs { process_id: pid, logs },
                ..
            } if pid == &process_id && !logs.is_empty()
        )
    });
    assert!(has_logs, "process logs should be collected");

    let stop_events = collect_stream_events(
        &service,
        "proc-stop-task",
        &format!("/proc stop {process_id}"),
    )
    .await;
    let stopped = stop_events.iter().any(|event| {
        matches!(
            event,
            AgentEvent::ProtocolEvent {
                payload: ProtocolEventPayload::ProcessStopped { process },
                ..
            } if process.id == process_id
        )
    });
    assert!(stopped, "expected ProcessStopped event");
}
