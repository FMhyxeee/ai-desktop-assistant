use std::collections::{HashMap, HashSet};
use std::path::{Component, Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use super::scan_skills_runtime_config;
use super::test_mcp_runtime_config;
use super::types::{
    AgentProvider, AgentRuntimeConfig, GovernanceIssue, GovernanceReport, GovernanceSeverity,
    McpRuntimeConfig, McpTransportKind, SkillsRuntimeConfig, WorkspaceRuntimeConfig,
};

pub async fn scan_runtime_governance(config: &AgentRuntimeConfig) -> GovernanceReport {
    let mut issues = Vec::new();

    check_agent_runtime_config(config, &mut issues);
    check_mcp_runtime_config(&config.mcp, &config.model_supports_image_input, &mut issues).await;
    check_skills_runtime_config(&config.skills, &config.workspace, &mut issues).await;
    check_workspace_consistency(config, &mut issues);
    check_memory_runtime_dependencies(&mut issues);

    finalize_report("workspace+runtime".to_string(), issues)
}

fn check_memory_runtime_dependencies(issues: &mut Vec<GovernanceIssue>) {
    if !crate::storage::sqlite_vec_available() {
        push_issue(
            issues,
            GovernanceSeverity::Warning,
            "memory",
            "memory_sqlite_vec_unavailable",
            "sqlite-vec extension is unavailable; memory vector recall is degraded.",
            None,
            Some(
                "Verify sqlite-vec runtime initialization and bundled SQLite compatibility."
                    .to_string(),
            ),
        );
    }
}

fn check_agent_runtime_config(config: &AgentRuntimeConfig, issues: &mut Vec<GovernanceIssue>) {
    if config.model.trim().is_empty() {
        push_issue(
            issues,
            GovernanceSeverity::Blocker,
            "agent",
            "agent_model_missing",
            "Agent model is empty.",
            None,
            Some("Set a non-empty model before starting the session.".to_string()),
        );
    }

    if config.system_prompt.trim().is_empty() {
        push_issue(
            issues,
            GovernanceSeverity::Warning,
            "agent",
            "agent_system_prompt_empty",
            "System prompt is empty.",
            None,
            Some("Set a non-empty system prompt to keep behavior stable.".to_string()),
        );
    }

    let requires_api_key = !matches!(config.provider, AgentProvider::Local);
    let inline_api_key = normalize_optional(config.api_key.as_deref());
    let api_key_env_name = config.api_key_env.trim();
    let env_api_key = if api_key_env_name.is_empty() {
        None
    } else {
        std::env::var(api_key_env_name)
            .ok()
            .and_then(|value| normalize_optional(Some(value.as_str())))
    };

    if requires_api_key && inline_api_key.is_none() && env_api_key.is_none() {
        push_issue(
            issues,
            GovernanceSeverity::Blocker,
            "agent",
            "agent_provider_api_key_mismatch",
            "Provider requires an API key but none was found.",
            Some(format!(
                "provider={:?}, apiKeyEnv={}",
                config.provider, config.api_key_env
            )),
            Some(
                "Provide apiKey directly or set a valid apiKeyEnv with a non-empty value."
                    .to_string(),
            ),
        );
    }

    if !requires_api_key && inline_api_key.is_some() {
        push_issue(
            issues,
            GovernanceSeverity::Info,
            "agent",
            "agent_local_provider_with_key",
            "Local provider has a configured API key.",
            None,
            Some("Remove apiKey if your local provider does not require one.".to_string()),
        );
    }
}

async fn check_mcp_runtime_config(
    config: &McpRuntimeConfig,
    model_supports_image_input: &bool,
    issues: &mut Vec<GovernanceIssue>,
) {
    if !config.enabled && config.image_recognition.enabled {
        push_issue(
            issues,
            GovernanceSeverity::Blocker,
            "mcp",
            "mcp_image_recognition_without_mcp",
            "MCP image recognition is enabled while MCP is disabled.",
            None,
            Some("Enable MCP or disable imageRecognition fallback.".to_string()),
        );
    }

    let mut seen_names = HashSet::new();
    let mut duplicate_names = HashSet::new();
    let mut enabled_names = HashSet::new();

    for server in &config.servers {
        if !server.enabled {
            continue;
        }

        let name = server.name.trim().to_string();
        if name.is_empty() {
            push_issue(
                issues,
                GovernanceSeverity::Blocker,
                "mcp",
                "mcp_server_name_empty",
                "Enabled MCP server has an empty name.",
                None,
                Some("Set a unique non-empty server name.".to_string()),
            );
            continue;
        }

        let normalized_name = name.to_lowercase();
        if !seen_names.insert(normalized_name.clone()) {
            duplicate_names.insert(name);
        } else {
            enabled_names.insert(normalized_name);
        }

        match server.transport {
            McpTransportKind::Tcp
            | McpTransportKind::Websocket
            | McpTransportKind::Wss
            | McpTransportKind::Sse => {
                push_issue(
                    issues,
                    GovernanceSeverity::Blocker,
                    "mcp",
                    "mcp_legacy_transport",
                    format!(
                        "Server '{}' uses unsupported legacy transport '{:?}'.",
                        server.name, server.transport
                    ),
                    None,
                    Some("Use stdio or streamable_http transport.".to_string()),
                );
            }
            McpTransportKind::Http | McpTransportKind::Https => {
                push_issue(
                    issues,
                    GovernanceSeverity::Info,
                    "mcp",
                    "mcp_transport_alias",
                    format!(
                        "Server '{}' uses transport alias '{:?}', it will be normalized to streamable_http.",
                        server.name, server.transport
                    ),
                    None,
                    None,
                );
            }
            McpTransportKind::Stdio | McpTransportKind::StreamableHttp => {}
        }

        let command = normalize_optional(server.command.as_deref());
        let endpoint = normalize_optional(server.endpoint.as_deref());
        match server.transport {
            McpTransportKind::StreamableHttp | McpTransportKind::Http | McpTransportKind::Https => {
                if endpoint.is_none() {
                    push_issue(
                        issues,
                        GovernanceSeverity::Blocker,
                        "mcp",
                        "mcp_streamable_http_endpoint_missing",
                        format!(
                            "Server '{}' requires endpoint for streamable_http transport.",
                            server.name
                        ),
                        None,
                        Some("Set endpoint to an http:// or https:// MCP URL.".to_string()),
                    );
                } else if let Some(value) = endpoint {
                    if !value.starts_with("http://") && !value.starts_with("https://") {
                        push_issue(
                            issues,
                            GovernanceSeverity::Blocker,
                            "mcp",
                            "mcp_streamable_http_endpoint_invalid",
                            format!(
                                "Server '{}' has invalid streamable_http endpoint '{}'.",
                                server.name, value
                            ),
                            None,
                            Some("Endpoint must start with http:// or https://.".to_string()),
                        );
                    }
                }
            }
            _ => {
                if command.is_none() && endpoint.is_none() {
                    push_issue(
                        issues,
                        GovernanceSeverity::Blocker,
                        "mcp",
                        "mcp_stdio_command_missing",
                        format!(
                            "Server '{}' is enabled but command/endpoint is missing.",
                            server.name
                        ),
                        None,
                        Some(
                            "Set command (or endpoint for compatible stdio launcher).".to_string(),
                        ),
                    );
                }
            }
        }
    }

    if !duplicate_names.is_empty() {
        let mut names = duplicate_names.into_iter().collect::<Vec<_>>();
        names.sort();
        push_issue(
            issues,
            GovernanceSeverity::Blocker,
            "mcp",
            "mcp_server_name_duplicate",
            "Duplicate MCP server names found.",
            Some(names.join(", ")),
            Some("Rename duplicated MCP servers to unique names.".to_string()),
        );
    }

    let mut mcp_probe = None;
    if config.enabled {
        let probe = test_mcp_runtime_config(config.clone()).await;
        for result in &probe.server_results {
            if result.enabled && !result.success {
                push_issue(
                    issues,
                    GovernanceSeverity::Warning,
                    "mcp",
                    "mcp_server_probe_failed",
                    format!("MCP server '{}' probe failed.", result.name),
                    result.error.clone(),
                    Some("Fix server command/env/network and re-run MCP test.".to_string()),
                );
            }
        }
        mcp_probe = Some(probe);
    }

    if config.image_recognition.enabled {
        let server_name = config.image_recognition.server_name.trim();
        let tool_name = config.image_recognition.tool_name.trim();
        if server_name.is_empty() {
            push_issue(
                issues,
                GovernanceSeverity::Blocker,
                "mcp",
                "mcp_image_recognition_server_missing",
                "imageRecognition.serverName is empty.",
                None,
                Some("Set imageRecognition.serverName to an enabled MCP server.".to_string()),
            );
        } else if !enabled_names.contains(&server_name.to_lowercase()) {
            push_issue(
                issues,
                GovernanceSeverity::Blocker,
                "mcp",
                "mcp_image_recognition_server_not_found",
                format!(
                    "imageRecognition.serverName '{}' does not match an enabled MCP server.",
                    server_name
                ),
                None,
                Some("Use an enabled MCP server name.".to_string()),
            );
        }

        if tool_name.is_empty() {
            push_issue(
                issues,
                GovernanceSeverity::Blocker,
                "mcp",
                "mcp_image_recognition_tool_missing",
                "imageRecognition.toolName is empty.",
                None,
                Some(
                    "Set imageRecognition.toolName to a valid tool on the target server."
                        .to_string(),
                ),
            );
        } else if let Some(probe) = &mcp_probe {
            if let Some(server) = probe
                .server_results
                .iter()
                .find(|item| item.name.eq_ignore_ascii_case(server_name))
            {
                if server.success {
                    let found_tool = server
                        .tools
                        .iter()
                        .any(|tool| tool.eq_ignore_ascii_case(tool_name));
                    if !found_tool {
                        push_issue(
                            issues,
                            GovernanceSeverity::Blocker,
                            "mcp",
                            "mcp_image_recognition_tool_not_found",
                            format!(
                                "imageRecognition.toolName '{}:{}' was not found.",
                                server_name, tool_name
                            ),
                            Some(format!("available_tools={}", server.tools.join(", "))),
                            Some(
                                "Choose an existing MCP tool name from the target server."
                                    .to_string(),
                            ),
                        );
                    }
                } else {
                    push_issue(
                        issues,
                        GovernanceSeverity::Warning,
                        "mcp",
                        "mcp_image_recognition_tool_unverified",
                        format!(
                            "Unable to verify imageRecognition tool '{}:{}' because server probe failed.",
                            server_name, tool_name
                        ),
                        server.error.clone(),
                        Some("Fix server connectivity, then run governance scan again.".to_string()),
                    );
                }
            }
        }
    } else if !*model_supports_image_input {
        push_issue(
            issues,
            GovernanceSeverity::Info,
            "mcp",
            "mcp_image_recognition_disabled",
            "Model does not support image input and imageRecognition fallback is disabled.",
            None,
            Some("Enable MCP imageRecognition if image understanding is needed.".to_string()),
        );
    }
}

async fn check_skills_runtime_config(
    config: &SkillsRuntimeConfig,
    workspace: &WorkspaceRuntimeConfig,
    issues: &mut Vec<GovernanceIssue>,
) {
    if !config.enabled {
        return;
    }

    if let Some(personal_dir) = normalize_optional(config.personal_dir.as_deref()) {
        check_skill_path_risk(&personal_dir, "skills_personal_dir", issues);
        if !Path::new(&personal_dir).exists() {
            push_issue(
                issues,
                GovernanceSeverity::Warning,
                "skills",
                "skills_personal_dir_missing",
                format!("personalDir '{}' does not exist.", personal_dir),
                None,
                Some("Create the directory or clear personalDir.".to_string()),
            );
        }
    }

    for project_dir in &config.project_dirs {
        let trimmed = project_dir.trim();
        if trimmed.is_empty() {
            continue;
        }

        check_skill_path_risk(trimmed, "skills_project_dir", issues);
        if !Path::new(trimmed).exists() {
            push_issue(
                issues,
                GovernanceSeverity::Warning,
                "skills",
                "skills_project_dir_missing",
                format!("projectDir '{}' does not exist.", trimmed),
                None,
                Some("Create the directory or remove it from projectDirs.".to_string()),
            );
        }
    }

    let scan = scan_skills_runtime_config(config.clone(), Some(workspace.clone())).await;
    for warning in scan.warnings {
        push_issue(
            issues,
            GovernanceSeverity::Warning,
            "skills",
            "skills_scan_warning",
            "Skill scan returned warning.",
            Some(warning),
            Some("Fix the warning and rerun scan.".to_string()),
        );
    }

    let mut duplicates: HashMap<String, Vec<String>> = HashMap::new();
    for entry in scan.skills {
        duplicates
            .entry(entry.name.to_lowercase())
            .or_default()
            .push(entry.path);
    }

    let mut duplicate_names = Vec::new();
    for (name, paths) in duplicates {
        if paths.len() > 1 {
            duplicate_names.push(format!("{name} -> {}", paths.join(" | ")));
        }
    }

    if !duplicate_names.is_empty() {
        duplicate_names.sort();
        push_issue(
            issues,
            GovernanceSeverity::Warning,
            "skills",
            "skills_name_duplicate",
            "Duplicate skill names found across configured directories.",
            Some(duplicate_names.join("; ")),
            Some("Rename duplicated skills to avoid ambiguous resolution.".to_string()),
        );
    }
}

fn check_workspace_consistency(config: &AgentRuntimeConfig, issues: &mut Vec<GovernanceIssue>) {
    let runtime_workspace = super::WorkspaceContext::resolve(&config.workspace);
    if !runtime_workspace.warnings.is_empty() {
        push_issue(
            issues,
            GovernanceSeverity::Warning,
            "workspace",
            "workspace_resolution_fallback",
            "Workspace resolution required fallback.",
            Some(runtime_workspace.warnings.join(" | ")),
            Some("Check workspace.rootDir and filesystem permissions.".to_string()),
        );
    }

    if let Err(err) = runtime_workspace.ensure_layout() {
        push_issue(
            issues,
            GovernanceSeverity::Blocker,
            "workspace",
            "workspace_ah_unavailable",
            "Workspace .ah directory tree is unavailable.",
            Some(err),
            Some("Ensure workspace path is writable and retry.".to_string()),
        );
    }

    check_workspace_gitignore(&runtime_workspace.root_dir, issues);

    let current_dir = match std::env::current_dir() {
        Ok(path) => path,
        Err(err) => {
            push_issue(
                issues,
                GovernanceSeverity::Warning,
                "workspace",
                "workspace_cwd_unavailable",
                "Cannot resolve current working directory for workspace checks.",
                Some(err.to_string()),
                None,
            );
            return;
        }
    };

    let Some(desktop_root) = resolve_desktop_repo_root(&current_dir) else {
        push_issue(
            issues,
            GovernanceSeverity::Warning,
            "workspace",
            "workspace_desktop_root_missing",
            "Cannot locate ai-desktop-assistant repository root.",
            Some(format!("cwd={}", current_dir.display())),
            None,
        );
        return;
    };

    let cargo_toml = desktop_root.join("src-tauri").join("Cargo.toml");
    let content = match std::fs::read_to_string(&cargo_toml) {
        Ok(text) => text,
        Err(err) => {
            push_issue(
                issues,
                GovernanceSeverity::Blocker,
                "workspace",
                "workspace_tauri_cargo_missing",
                "Cannot read ai-desktop-assistant/src-tauri/Cargo.toml.",
                Some(err.to_string()),
                None,
            );
            return;
        }
    };

    let expected_path = "../../agent-lib";
    match extract_agent_lib_path_dependency(&content) {
        Some(path) if path == expected_path => {}
        Some(path) => {
            push_issue(
                issues,
                GovernanceSeverity::Blocker,
                "workspace",
                "workspace_agent_lib_path_mismatch",
                "agent-lib path dependency does not match local-development rule.",
                Some(format!("actual_path={}", path)),
                Some(format!(
                    "Set agent-lib dependency path to '{}'.",
                    expected_path
                )),
            );
        }
        None => {
            push_issue(
                issues,
                GovernanceSeverity::Blocker,
                "workspace",
                "workspace_agent_lib_path_missing",
                "Cannot find agent-lib path dependency in src-tauri/Cargo.toml.",
                None,
                Some(format!(
                    "Declare agent-lib as: agent-lib = {{ path = \"{}\" }}",
                    expected_path
                )),
            );
        }
    }

    let workspace_root = desktop_root
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| desktop_root.clone());
    let agent_lib_root = workspace_root.join("agent-lib");
    if !agent_lib_root.join("Cargo.toml").exists() {
        push_issue(
            issues,
            GovernanceSeverity::Warning,
            "workspace",
            "workspace_agent_lib_not_found",
            "Cannot locate sibling agent-lib repository.",
            Some(format!("checked={}", agent_lib_root.display())),
            None,
        );
        return;
    }

    let required_files = [
        "examples/mcp_config.toml",
        "examples/mcp_config.json",
        "src/mcp/config.rs",
    ];
    for relative in required_files {
        let path = agent_lib_root.join(relative);
        if !path.exists() {
            push_issue(
                issues,
                GovernanceSeverity::Warning,
                "workspace",
                "workspace_agent_lib_required_file_missing",
                format!("Expected agent-lib file '{}' is missing.", relative),
                Some(format!("path={}", path.display())),
                None,
            );
        }
    }
}

fn check_workspace_gitignore(workspace_root: &Path, issues: &mut Vec<GovernanceIssue>) {
    let Some(repo_root) = find_git_repo_root(workspace_root) else {
        return;
    };

    let gitignore_path = repo_root.join(".gitignore");
    let content = match std::fs::read_to_string(&gitignore_path) {
        Ok(content) => content,
        Err(_) => return,
    };

    if !contains_ah_ignore_rule(&content) {
        push_issue(
            issues,
            GovernanceSeverity::Warning,
            "workspace",
            "workspace_gitignore_ah_missing",
            "Git repository does not ignore .ah/ directory.",
            Some(format!("path={}", gitignore_path.display())),
            Some(
                "Add '.ah/' to .gitignore to avoid committing workspace runtime files.".to_string(),
            ),
        );
    }
}

fn find_git_repo_root(start: &Path) -> Option<PathBuf> {
    for ancestor in start.ancestors() {
        if ancestor.join(".git").exists() {
            return Some(ancestor.to_path_buf());
        }
    }
    None
}

fn contains_ah_ignore_rule(content: &str) -> bool {
    content.lines().any(|line| {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            return false;
        }

        let normalized = trimmed.replace('\\', "/");
        normalized == ".ah"
            || normalized == ".ah/"
            || normalized == "/.ah"
            || normalized == "/.ah/"
            || normalized.contains(".ah/")
            || normalized.ends_with("/.ah")
    })
}

fn resolve_desktop_repo_root(start: &Path) -> Option<PathBuf> {
    for ancestor in start.ancestors() {
        if ancestor.join("src-tauri").join("Cargo.toml").exists() {
            return Some(ancestor.to_path_buf());
        }
        if ancestor
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.eq_ignore_ascii_case("src-tauri"))
            && ancestor.join("Cargo.toml").exists()
        {
            if let Some(parent) = ancestor.parent() {
                return Some(parent.to_path_buf());
            }
        }
    }
    None
}

fn extract_agent_lib_path_dependency(content: &str) -> Option<String> {
    for raw_line in content.lines() {
        let line = raw_line.trim();
        if !line.starts_with("agent-lib") {
            continue;
        }
        let Some(path_index) = line.find("path") else {
            continue;
        };
        let tail = &line[path_index + "path".len()..];
        let first_quote = tail.find('"')?;
        let after_first = &tail[first_quote + 1..];
        let second_quote = after_first.find('"')?;
        return Some(after_first[..second_quote].to_string());
    }
    None
}

fn check_skill_path_risk(path: &str, code_prefix: &str, issues: &mut Vec<GovernanceIssue>) {
    let candidate = Path::new(path);
    if candidate
        .components()
        .any(|component| matches!(component, Component::ParentDir))
    {
        push_issue(
            issues,
            GovernanceSeverity::Warning,
            "skills",
            &format!("{code_prefix}_has_parent_dir"),
            format!(
                "Skill path '{}' contains '..' and may escape expected boundary.",
                path
            ),
            None,
            Some("Use normalized absolute path without '..' segments.".to_string()),
        );
    }
}

fn finalize_report(scope: String, mut issues: Vec<GovernanceIssue>) -> GovernanceReport {
    issues.sort_by_key(|issue| match issue.severity {
        GovernanceSeverity::Blocker => 0,
        GovernanceSeverity::Warning => 1,
        GovernanceSeverity::Info => 2,
    });

    let blocker_count = issues
        .iter()
        .filter(|issue| issue.severity == GovernanceSeverity::Blocker)
        .count();
    let warning_count = issues
        .iter()
        .filter(|issue| issue.severity == GovernanceSeverity::Warning)
        .count();
    let info_count = issues
        .iter()
        .filter(|issue| issue.severity == GovernanceSeverity::Info)
        .count();

    GovernanceReport {
        generated_at_unix_ms: current_time_ms(),
        scope,
        summary: format!(
            "Governance scan completed: blockers={}, warnings={}, info={}.",
            blocker_count, warning_count, info_count
        ),
        issues,
        blocker_count,
        warning_count,
        info_count,
    }
}

fn push_issue(
    issues: &mut Vec<GovernanceIssue>,
    severity: GovernanceSeverity,
    category: &str,
    code: &str,
    message: impl Into<String>,
    evidence: Option<String>,
    suggestion: Option<String>,
) {
    issues.push(GovernanceIssue {
        severity,
        category: category.to_string(),
        code: code.to_string(),
        message: message.into(),
        evidence,
        suggestion,
    });
}

fn normalize_optional(value: Option<&str>) -> Option<String> {
    value.and_then(|item| {
        let trimmed = item.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_string())
        }
    })
}

fn current_time_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent_service::types::{McpImageRecognitionRuntimeConfig, McpServerRuntimeConfig};

    fn make_config() -> AgentRuntimeConfig {
        AgentRuntimeConfig {
            provider: AgentProvider::Local,
            model: "qwen2.5-coder:7b".to_string(),
            model_supports_image_input: true,
            api_key_env: "LOCAL_API_KEY".to_string(),
            api_key: None,
            base_url: None,
            max_tokens: None,
            system_prompt: "prompt".to_string(),
            workspace: Default::default(),
            mcp: McpRuntimeConfig::default(),
            skills: SkillsRuntimeConfig::default(),
            control: Default::default(),
        }
    }

    #[tokio::test]
    async fn scan_runtime_governance_flags_duplicate_mcp_servers() {
        let mut config = make_config();
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
        config.mcp.image_recognition = McpImageRecognitionRuntimeConfig {
            enabled: false,
            ..Default::default()
        };

        let report = scan_runtime_governance(&config).await;
        assert!(report
            .issues
            .iter()
            .any(|issue| issue.code == "mcp_server_name_duplicate"));
    }

    #[tokio::test]
    async fn scan_runtime_governance_flags_missing_required_agent_model() {
        let mut config = make_config();
        config.model = "   ".to_string();

        let report = scan_runtime_governance(&config).await;
        assert!(report
            .issues
            .iter()
            .any(|issue| issue.code == "agent_model_missing"));
    }

    #[tokio::test]
    async fn scan_runtime_governance_warns_when_gitignore_missing_ah_rule() {
        let temp_root =
            std::env::temp_dir().join(format!("ai-helper-governance-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(temp_root.join(".git")).expect("should create .git");
        std::fs::write(temp_root.join(".gitignore"), "target/\nnode_modules/\n")
            .expect("should write .gitignore");

        let mut config = make_config();
        config.workspace.root_dir = temp_root.to_string_lossy().to_string();

        let report = scan_runtime_governance(&config).await;
        assert!(report
            .issues
            .iter()
            .any(|issue| issue.code == "workspace_gitignore_ah_missing"));

        let _ = std::fs::remove_dir_all(temp_root);
    }

    #[test]
    fn extract_agent_lib_path_dependency_reads_path() {
        let content = r#"
[dependencies]
agent-lib = { path = "../../agent-lib" }
"#;

        let path = extract_agent_lib_path_dependency(content);
        assert_eq!(path.as_deref(), Some("../../agent-lib"));
    }
}
