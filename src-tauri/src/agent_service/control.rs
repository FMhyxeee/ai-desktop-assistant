use std::sync::Arc;

use agent_lib::model::{Message, ModelClient};
use serde::{Deserialize, Serialize};

use super::types::{
    AgentHistoryMessage, AgentRuntimeConfig, McpRuntimeConfig, PatchDecisionSource,
    ProtocolRuntimeConfigPatch, SkillsRuntimeConfig,
};

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeConfigPatch {
    #[serde(default)]
    pub system_prompt: Option<String>,
    #[serde(default)]
    pub mcp: Option<McpRuntimeConfig>,
    #[serde(default)]
    pub skills: Option<SkillsRuntimeConfig>,
}

impl RuntimeConfigPatch {
    pub fn is_empty(&self) -> bool {
        self.system_prompt.is_none() && self.mcp.is_none() && self.skills.is_none()
    }

    pub fn to_protocol_patch(&self) -> ProtocolRuntimeConfigPatch {
        ProtocolRuntimeConfigPatch {
            system_prompt: self.system_prompt.clone(),
            mcp: self.mcp.clone(),
            skills: self.skills.clone(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct ControlInput {
    pub user_input: String,
    pub recent_messages: Vec<AgentHistoryMessage>,
    pub effective_config: AgentRuntimeConfig,
}

#[derive(Debug, Clone)]
pub struct ControlDecision {
    pub source: PatchDecisionSource,
    pub confidence: f32,
    pub summary: String,
    pub developer_instructions: Option<String>,
    pub patch: Option<RuntimeConfigPatch>,
}

#[derive(Debug, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct ModelFallbackResponse {
    confidence: Option<f32>,
    summary: Option<String>,
    developer_instructions: Option<String>,
    patch: Option<RuntimeConfigPatch>,
}

#[derive(Debug, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct TitleSuggestionResponse {
    title: Option<String>,
}

pub fn evaluate_rules(input: &ControlInput) -> ControlDecision {
    let text = input.user_input.trim();
    let normalized = text.to_lowercase();

    let mut patch = RuntimeConfigPatch::default();
    let mut summary_parts = Vec::<String>::new();

    if is_enable_mcp_intent(&normalized) {
        let mut mcp = input.effective_config.mcp.clone();
        mcp.enabled = true;
        patch.mcp = Some(mcp);
        summary_parts.push("rule: enable mcp".to_string());
    } else if is_disable_mcp_intent(&normalized) {
        let mut mcp = input.effective_config.mcp.clone();
        mcp.enabled = false;
        patch.mcp = Some(mcp);
        summary_parts.push("rule: disable mcp".to_string());
    }

    if is_enable_skills_intent(&normalized) {
        let mut skills = input.effective_config.skills.clone();
        skills.enabled = true;
        patch.skills = Some(skills);
        summary_parts.push("rule: enable skills".to_string());
    } else if is_disable_skills_intent(&normalized) {
        let mut skills = input.effective_config.skills.clone();
        skills.enabled = false;
        patch.skills = Some(skills);
        summary_parts.push("rule: disable skills".to_string());
    }

    if let Some(system_prompt) = parse_system_prompt_override(text) {
        patch.system_prompt = Some(system_prompt);
        summary_parts.push("rule: update system prompt".to_string());
    }

    let patch = if patch.is_empty() { None } else { Some(patch) };
    let summary = if summary_parts.is_empty() {
        "no config change intent detected by rules".to_string()
    } else {
        summary_parts.join("; ")
    };
    let confidence = if patch.is_some() { 0.92 } else { 0.30 };
    let developer_instructions = Some(assemble_developer_instructions(
        &input.effective_config,
        patch.as_ref(),
    ));

    ControlDecision {
        source: PatchDecisionSource::Rule,
        confidence,
        summary,
        developer_instructions,
        patch,
    }
}

pub fn should_try_model_fallback(input: &str) -> bool {
    let normalized = input.to_lowercase();
    let keywords = [
        "mcp",
        "skills",
        "skill",
        "system prompt",
        "system_prompt",
        "prompt",
        "config",
        "配置",
        "提示词",
    ];
    keywords.iter().any(|keyword| normalized.contains(keyword))
}

pub async fn evaluate_model_fallback(
    input: &ControlInput,
    model: Arc<dyn ModelClient>,
) -> Option<ControlDecision> {
    let config_snapshot = serde_json::json!({
        "systemPrompt": input.effective_config.system_prompt,
        "mcp": input.effective_config.mcp,
        "skills": input.effective_config.skills,
    });

    let recent_history = serde_json::to_string_pretty(&input.recent_messages).ok()?;
    let user_prompt = format!(
        "current_input:\n{}\n\nrecent_messages:\n{}\n\ncurrent_runtime:\n{}",
        input.user_input,
        recent_history,
        serde_json::to_string_pretty(&config_snapshot).ok()?
    );

    let messages = vec![
        Message::system(
            "You are a runtime control planner. Return only JSON with shape: \
            {\"confidence\":0..1,\"summary\":\"...\",\"developerInstructions\":\"...\",\
            \"patch\":{\"systemPrompt\":string|null,\"mcp\":object|null,\"skills\":object|null}}. \
            If no config change is needed, set patch to null.",
        ),
        Message::user(user_prompt),
    ];

    let response = model.chat(messages, Vec::new()).await.ok()?;
    let parsed = parse_model_fallback_response(&response.content)?;
    let patch = parsed.patch.filter(|item| !item.is_empty());

    Some(ControlDecision {
        source: PatchDecisionSource::ModelFallback,
        confidence: parsed.confidence.unwrap_or(0.0).clamp(0.0, 1.0),
        summary: parsed
            .summary
            .filter(|item| !item.trim().is_empty())
            .unwrap_or_else(|| "model fallback evaluated runtime intent".to_string()),
        developer_instructions: parsed.developer_instructions.or_else(|| {
            Some(assemble_developer_instructions(
                &input.effective_config,
                patch.as_ref(),
            ))
        }),
        patch,
    })
}

pub async fn suggest_conversation_title(
    input: &ControlInput,
    model: Option<Arc<dyn ModelClient>>,
) -> Option<String> {
    if let Some(model) = model {
        if let Some(title) = suggest_conversation_title_by_model(input, model).await {
            return Some(title);
        }
    }
    suggest_conversation_title_by_rules(input)
}

pub fn assemble_developer_instructions(
    config: &AgentRuntimeConfig,
    patch: Option<&RuntimeConfigPatch>,
) -> String {
    let active_system_prompt = patch
        .and_then(|item| item.system_prompt.clone())
        .unwrap_or_else(|| config.system_prompt.clone());
    let mcp_enabled = patch
        .and_then(|item| item.mcp.as_ref().map(|cfg| cfg.enabled))
        .unwrap_or(config.mcp.enabled);
    let skills_enabled = patch
        .and_then(|item| item.skills.as_ref().map(|cfg| cfg.enabled))
        .unwrap_or(config.skills.enabled);

    format!(
        "{}\n\n[Runtime control context]\n- MCP enabled: {}\n- Skills enabled: {}\n- Follow approved runtime changes only.\n- Never assume unapproved config mutations are active.",
        active_system_prompt, mcp_enabled, skills_enabled
    )
}

async fn suggest_conversation_title_by_model(
    input: &ControlInput,
    model: Arc<dyn ModelClient>,
) -> Option<String> {
    let user_prompt = format!(
        "Generate a concise conversation title (max 36 chars, plain text, no quotes).\n\ncurrent_input:\n{}\n\nrecent_messages:\n{}",
        input.user_input,
        serde_json::to_string_pretty(&input.recent_messages).ok()?
    );

    let messages = vec![
        Message::system("You generate conversation titles. Return JSON only: {\"title\":\"...\"}."),
        Message::user(user_prompt),
    ];

    let response = model.chat(messages, Vec::new()).await.ok()?;
    let parsed = parse_title_suggestion_response(&response.content)?;
    normalize_title(Some(parsed))
}

fn suggest_conversation_title_by_rules(input: &ControlInput) -> Option<String> {
    let merged = if input.recent_messages.is_empty() {
        input.user_input.clone()
    } else {
        let mut parts = input
            .recent_messages
            .iter()
            .rev()
            .take(2)
            .map(|m| m.content.trim())
            .filter(|text| !text.is_empty())
            .map(|text| text.to_string())
            .collect::<Vec<_>>();
        parts.reverse();
        parts.push(input.user_input.clone());
        parts.join(" ")
    };

    normalize_title(Some(heuristic_title_from_text(&merged)))
}

fn heuristic_title_from_text(raw: &str) -> String {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return String::new();
    }

    if let Some(command) = trimmed.strip_prefix('/') {
        let compact = command.split_whitespace().collect::<Vec<_>>().join(" ");
        return format!("Command: {}", compact);
    }

    let without_code_fence = trimmed.replace("```", " ");
    let without_lines = without_code_fence
        .lines()
        .take(4)
        .collect::<Vec<_>>()
        .join(" ");
    let compact = without_lines
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");

    let sentence = compact
        .split(|c| ['。', '.', '!', '?', ';', '；', '！', '？'].contains(&c))
        .map(str::trim)
        .find(|item| !item.is_empty())
        .unwrap_or_default();

    if sentence.is_empty() {
        compact
    } else {
        sentence.to_string()
    }
}

fn normalize_title(title: Option<String>) -> Option<String> {
    let raw = title?;
    let cleaned = raw
        .replace(['\n', '\r', '\t'], " ")
        .replace(['`', '"', '\'', '#'], "")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    let trimmed = cleaned.trim_matches(|c: char| c == '-' || c == ':' || c.is_whitespace());
    if trimmed.is_empty() {
        return None;
    }

    let mut out = String::new();
    for ch in trimmed.chars().take(36) {
        out.push(ch);
    }

    if out.is_empty() {
        None
    } else {
        Some(out)
    }
}

fn parse_title_suggestion_response(content: &str) -> Option<String> {
    let trimmed = content.trim();
    if trimmed.is_empty() {
        return None;
    }

    if let Ok(value) = serde_json::from_str::<TitleSuggestionResponse>(trimmed) {
        return value.title;
    }

    if let Some(extracted) = extract_first_json_object(trimmed) {
        if let Ok(value) = serde_json::from_str::<TitleSuggestionResponse>(&extracted) {
            if value.title.is_some() {
                return value.title;
            }
        }
    }

    trimmed.lines().next().map(|line| line.trim().to_string())
}

fn parse_model_fallback_response(content: &str) -> Option<ModelFallbackResponse> {
    let trimmed = content.trim();
    if trimmed.is_empty() {
        return None;
    }

    if let Ok(value) = serde_json::from_str::<ModelFallbackResponse>(trimmed) {
        return Some(value);
    }

    let extracted = extract_first_json_object(trimmed)?;
    serde_json::from_str::<ModelFallbackResponse>(&extracted).ok()
}

fn extract_first_json_object(text: &str) -> Option<String> {
    let mut start_idx = None;
    let mut depth = 0_i32;
    let mut in_string = false;
    let mut escaped = false;

    for (index, ch) in text.char_indices() {
        if in_string {
            if escaped {
                escaped = false;
                continue;
            }
            if ch == '\\' {
                escaped = true;
            } else if ch == '"' {
                in_string = false;
            }
            continue;
        }

        if ch == '"' {
            in_string = true;
            continue;
        }

        if ch == '{' {
            if start_idx.is_none() {
                start_idx = Some(index);
            }
            depth += 1;
            continue;
        }

        if ch == '}' {
            if depth == 0 {
                continue;
            }
            depth -= 1;
            if depth == 0 {
                let start = start_idx?;
                let end = index + ch.len_utf8();
                return Some(text[start..end].to_string());
            }
        }
    }

    None
}

fn parse_system_prompt_override(raw: &str) -> Option<String> {
    let markers = [
        "system prompt:",
        "system_prompt:",
        "系统提示词:",
        "系统提示:",
        "/system ",
    ];

    let lower = raw.to_lowercase();
    for marker in markers {
        if let Some(index) = lower.find(marker) {
            let value = raw[index + marker.len()..].trim();
            if !value.is_empty() {
                return Some(value.to_string());
            }
        }
    }

    None
}

fn is_enable_mcp_intent(text: &str) -> bool {
    ["enable mcp", "turn on mcp", "开启mcp", "启用mcp", "打开mcp"]
        .iter()
        .any(|pattern| text.contains(pattern))
}

fn is_disable_mcp_intent(text: &str) -> bool {
    [
        "disable mcp",
        "turn off mcp",
        "close mcp",
        "关闭mcp",
        "禁用mcp",
    ]
    .iter()
    .any(|pattern| text.contains(pattern))
}

fn is_enable_skills_intent(text: &str) -> bool {
    [
        "enable skills",
        "turn on skills",
        "开启skills",
        "启用skills",
        "打开skills",
    ]
    .iter()
    .any(|pattern| text.contains(pattern))
}

fn is_disable_skills_intent(text: &str) -> bool {
    [
        "disable skills",
        "turn off skills",
        "close skills",
        "关闭skills",
        "禁用skills",
    ]
    .iter()
    .any(|pattern| text.contains(pattern))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent_service::types::{AgentHistoryRole, AppControlRuntimeConfig};

    fn make_config() -> AgentRuntimeConfig {
        AgentRuntimeConfig {
            provider: crate::agent_service::types::AgentProvider::Local,
            model: "qwen2.5-coder:7b".to_string(),
            model_supports_image_input: true,
            api_key_env: "LOCAL_API_KEY".to_string(),
            api_key: None,
            base_url: None,
            max_tokens: None,
            system_prompt: "base prompt".to_string(),
            workspace: Default::default(),
            mcp: McpRuntimeConfig::default(),
            skills: SkillsRuntimeConfig::default(),
            memory: Default::default(),
            control: AppControlRuntimeConfig::default(),
        }
    }

    #[test]
    fn evaluate_rules_detects_mcp_enable() {
        let input = ControlInput {
            user_input: "please enable mcp".to_string(),
            recent_messages: Vec::new(),
            effective_config: make_config(),
        };

        let decision = evaluate_rules(&input);
        assert!(decision.patch.is_some());
        let patch = decision.patch.unwrap_or_default();
        assert!(patch.mcp.as_ref().is_some_and(|cfg| cfg.enabled));
    }

    #[test]
    fn evaluate_rules_detects_skills_disable() {
        let mut config = make_config();
        config.skills.enabled = true;

        let input = ControlInput {
            user_input: "turn off skills".to_string(),
            recent_messages: vec![AgentHistoryMessage {
                role: AgentHistoryRole::User,
                content: "history".to_string(),
            }],
            effective_config: config,
        };

        let decision = evaluate_rules(&input);
        assert!(decision.patch.is_some());
        let patch = decision.patch.unwrap_or_default();
        assert!(patch.skills.as_ref().is_some_and(|cfg| !cfg.enabled));
    }

    #[test]
    fn evaluate_rules_detects_system_prompt_override() {
        let input = ControlInput {
            user_input: "system prompt: be concise and strict".to_string(),
            recent_messages: Vec::new(),
            effective_config: make_config(),
        };

        let decision = evaluate_rules(&input);
        assert!(decision.patch.is_some());
        let patch = decision.patch.unwrap_or_default();
        assert_eq!(
            patch.system_prompt.as_deref(),
            Some("be concise and strict")
        );
    }

    #[test]
    fn fallback_keyword_detection_works() {
        assert!(should_try_model_fallback("update config for tools"));
        assert!(should_try_model_fallback("调整提示词"));
        assert!(!should_try_model_fallback("just say hello"));
    }

    #[test]
    fn parse_model_fallback_response_parses_embedded_json() {
        let raw = "analysis... {\"confidence\":0.5,\"summary\":\"low\",\"patch\":null}";
        let parsed = parse_model_fallback_response(raw);
        assert!(parsed.is_some());
        assert_eq!(parsed.unwrap_or_default().summary.as_deref(), Some("low"));
    }

    #[test]
    fn suggest_conversation_title_by_rules_returns_title() {
        let input = ControlInput {
            user_input: "请帮我分析 Rust MCP 客户端连接超时问题".to_string(),
            recent_messages: Vec::new(),
            effective_config: make_config(),
        };

        let title = suggest_conversation_title_by_rules(&input);
        assert!(title.is_some());
        assert!(!title.unwrap_or_default().is_empty());
    }

    #[test]
    fn normalize_title_limits_length() {
        let title = normalize_title(Some("x".repeat(100)));
        assert!(title.is_some());
        assert!(title.unwrap_or_default().chars().count() <= 36);
    }
}
