use std::sync::{Arc, Mutex};
use std::time::Duration;

use ai_desktop_assistant_lib::agent_service::types::{
    AgentEvent, AgentProvider, AgentRuntimeConfig, AgentStreamInput, McpRuntimeConfig,
    McpServerRuntimeConfig, McpTransportKind, ProtocolEventPayload, SkillsRuntimeConfig,
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

    let result = scan_skills_runtime_config(SkillsRuntimeConfig {
        enabled: true,
        personal_dir: Some(root.to_string_lossy().to_string()),
        project_dirs: vec![],
        auto_apply: false,
    })
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
        system_prompt: "local prompt".to_string(),
        mcp: McpRuntimeConfig::default(),
        skills: SkillsRuntimeConfig::default(),
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
async fn governance_scan_flags_duplicate_mcp_server_names() {
    let mut config = AgentRuntimeConfig {
        provider: AgentProvider::Local,
        model: "qwen2.5-coder:7b".to_string(),
        model_supports_image_input: true,
        api_key_env: "LOCAL_API_KEY".to_string(),
        api_key: None,
        base_url: None,
        max_tokens: None,
        system_prompt: "local prompt".to_string(),
        mcp: McpRuntimeConfig::default(),
        skills: SkillsRuntimeConfig::default(),
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
