export interface Message {
  id: string;
  role: 'user' | 'assistant' | 'system';
  content: string;
  images?: InputImageAttachment[];
  timestamp: number;
  isStreaming?: boolean;
}

export interface InputImageAttachment {
  id: string;
  name: string;
  mimeType: string;
  dataUrl: string;
  sizeBytes: number;
}

export interface InputCard {
  content: string;
  images?: InputImageAttachment[];
}

export type ProtocolCardDirection = 'op' | 'event';
export type ProtocolCardLevel = 'info' | 'success' | 'warning' | 'error';

export interface ProtocolTokenUsage {
  prompt_tokens: number;
  completion_tokens: number;
  total_tokens: number;
}

export interface ProtocolMcpToolInfo {
  name: string;
  description: string;
  server: string;
}

export interface ProtocolMcpResourceInfo {
  uri: string;
  name: string;
  description?: string;
  mime_type?: string;
}

export interface ProtocolPromptArgumentInfo {
  name: string;
  description?: string;
  required: boolean;
}

export interface ProtocolMcpPromptInfo {
  name: string;
  description?: string;
  arguments?: ProtocolPromptArgumentInfo[];
}

export type ProtocolPromptContent =
  | {
      type: 'text';
      text: string;
    }
  | {
      type: 'image';
      data: string;
      mime_type: string;
    };

export interface ProtocolPromptMessage {
  role: string;
  content: ProtocolPromptContent;
}

export interface ProtocolSkillEntry {
  name: string;
  description: string;
  path: string;
  source: string;
  has_auxiliary_files: boolean;
}

export type GovernanceSeverity = 'blocker' | 'warning' | 'info';

export interface GovernanceIssue {
  severity: GovernanceSeverity;
  category: string;
  code: string;
  message: string;
  evidence?: string;
  suggestion?: string;
}

export interface GovernanceReport {
  generatedAtUnixMs: number;
  scope: string;
  summary: string;
  issues: GovernanceIssue[];
  blockerCount: number;
  warningCount: number;
  infoCount: number;
}

export interface GovernanceUpdateReport {
  before?: GovernanceReport | null;
  after: GovernanceReport;
}

export type PatchDecisionSource = 'rule' | 'model_fallback';

export interface ProtocolRuntimeConfigPatch {
  mcp?: McpRuntimeConfig | null;
  skills?: SkillsRuntimeConfig | null;
}

export interface ProtocolMcpRewriteEntry {
  fieldPath: string;
  reason: string;
  before: string;
  after: string;
}

export interface ProtocolMcpRejectPreview {
  fieldPath: string;
  reason: string;
}

export interface ProtocolMcpArgNormalization {
  rewrittenCount: number;
  rewrites: ProtocolMcpRewriteEntry[];
  rejectPreview?: ProtocolMcpRejectPreview | null;
}

export interface ProtocolGuidanceTemplate {
  templateType: string;
  target: string;
  payload: Record<string, unknown>;
}

export interface ProtocolGuidancePathVars {
  workspaceRoot: string;
  ahDir: string;
  screenshotsDir: string;
  imageRecognitionDir: string;
}

export interface ProtocolGuidanceToolConstraint {
  tool: string;
  serverName: string;
  imageTool: boolean;
  imageFields: string[];
  pathFields: string[];
}

export interface ProtocolGuidanceHitRules {
  signals: string[];
  selectedContracts: string[];
  selectedServers: string[];
  selectedPrompts: string[];
}

export interface ProtocolGuidanceGovernanceSummary {
  blockerCount: number;
  warningCount: number;
  infoCount: number;
  topIssueCodes: string[];
}

export interface ProtocolGuidanceMemorySummary {
  enabled: boolean;
  contextFound: boolean;
  nonSecretCount: number;
  secretCount: number;
  sourceTags: string[];
  redactedLines: string[];
}

export interface ProtocolGuidanceInput {
  userInput: string;
  workspaceRoot: string;
  mcpContractsTotal: number;
  mcpServersTotal: number;
  skillsTotal: number;
  hitRules: ProtocolGuidanceHitRules;
  governance: ProtocolGuidanceGovernanceSummary;
  memory: ProtocolGuidanceMemorySummary;
}

export interface ProtocolGuidanceOutput {
  systemPromptFragment: string;
  pathVars: ProtocolGuidancePathVars;
  toolConstraints: ProtocolGuidanceToolConstraint[];
  templates: ProtocolGuidanceTemplate[];
  warnings: string[];
}

export interface ProtocolProcessInfo {
  id: string;
  command: string;
  pid?: number | null;
  status: string;
  startedAtUnixMs: number;
  exitCode?: number | null;
}

export type AgentHistoryRole = 'user' | 'assistant' | 'system';

export interface AgentHistoryMessage {
  role: AgentHistoryRole;
  content: string;
}

export type ApprovalPolicy = 'always-ask' | 'read-only-safe' | 'never-ask';
export type SandboxPolicy = 'readonly' | 'persistent' | 'in-memory';

export type ProtocolOpPayload =
  | {
      type: 'user_turn';
      model: string;
      cwd: string;
      approval_policy: ApprovalPolicy;
      sandbox_policy: SandboxPolicy;
      text: string;
      images?: Array<{
        name: string;
        mime_type: string;
      }>;
    }
  | {
      type: 'run_user_shell_command';
      command: string;
    }
  | {
      type: 'proc_command';
      command: string;
    }
  | {
      type: 'run_sub_agent';
      mode: string;
      input: string;
    }
  | {
      type: 'interrupt';
    };

export type ProtocolEventPayload =
  | {
      type: 'turn_started';
      turn_id: string;
    }
  | {
      type: 'model_streaming';
      chunk: string;
    }
  | {
      type: 'model_complete';
      content: string;
      usage: ProtocolTokenUsage;
    }
  | {
      type: 'tool_call_requested';
      tool: string;
      args: unknown;
      normalization?: ProtocolMcpArgNormalization | null;
    }
  | {
      type: 'tool_call_result';
      tool: string;
      result: unknown;
    }
  | {
      type: 'run_user_shell_command';
      command: string;
    }
  | {
      type: 'process_started';
      process: ProtocolProcessInfo;
    }
  | {
      type: 'process_list';
      processes: ProtocolProcessInfo[];
    }
  | {
      type: 'process_logs';
      process_id: string;
      logs: string[];
    }
  | {
      type: 'process_stopped';
      process: ProtocolProcessInfo;
    }
  | {
      type: 'process_error';
      action: string;
      message: string;
    }
  | {
      type: 'think_status';
      active: boolean;
    }
  | {
      type: 'reasoning_streaming';
      chunk: string;
    }
  | {
      type: 'warning';
      message: string;
    }
  | {
      type: 'error';
      code: string;
      message: string;
    }
  | {
      type: 'turn_aborted';
      reason: string;
    }
  | {
      type: 'turn_complete';
      result: unknown;
    }
  | {
      type: 'mcp_list_tools_response';
      tools: ProtocolMcpToolInfo[];
    }
  | {
      type: 'mcp_list_resources_response';
      resources: ProtocolMcpResourceInfo[];
    }
  | {
      type: 'mcp_resource_content';
      uri: string;
      content: string;
    }
  | {
      type: 'mcp_list_prompts_response';
      prompts: ProtocolMcpPromptInfo[];
    }
  | {
      type: 'mcp_prompt_result';
      messages: ProtocolPromptMessage[];
    }
  | {
      type: 'list_skills_response';
      skills: ProtocolSkillEntry[];
    }
  | {
      type: 'skill_content';
      name: string;
      content: string;
      auxiliary_files: string[];
    }
  | {
      type: 'skill_applied';
      name: string;
    }
  | {
      type: 'skill_file_content';
      skill_name: string;
      file_path: string;
      content: string;
    }
  | {
      type: 'sub_agent_started';
      mode: string;
      input: string;
    }
  | {
      type: 'sub_agent_progress';
      mode: string;
      message: string;
    }
  | {
      type: 'sub_agent_completed';
      mode: string;
      output: string;
    }
  | {
      type: 'sub_agent_failed';
      mode: string;
      error: string;
    }
  | {
      type: 'governance_report';
      report: GovernanceReport;
    }
  | {
      type: 'guidance_context';
      input: ProtocolGuidanceInput;
      output: ProtocolGuidanceOutput;
    }
  | {
      type: 'conversation_title_suggestion';
      conversation_id: string;
      title: string;
    }
  | {
      type: 'control_decision';
      source: PatchDecisionSource;
      confidence: number;
      summary: string;
      developer_instructions?: string | null;
      patch?: ProtocolRuntimeConfigPatch | null;
    }
  | {
      type: 'config_change_request';
      request_id: string;
      summary: string;
      source: PatchDecisionSource;
      confidence: number;
      patch: ProtocolRuntimeConfigPatch;
      expires_at_unix_ms: number;
    }
  | {
      type: 'config_change_result';
      request_id: string;
      approved: boolean;
      persisted: boolean;
      applied: boolean;
      reason: string;
      patch?: ProtocolRuntimeConfigPatch | null;
    }
  | {
      type: 'tool_approval_required';
      request_id: string;
      tool: string;
      args: unknown;
      risk_level: 'low' | 'medium' | 'high';
      reason: string;
      expires_at_unix_ms: number;
    }
  | {
      type: 'tool_approval_result';
      request_id: string;
      approved: boolean;
      remember_choice: boolean;
      reason: string;
    };

export interface ProtocolCard {
  id: string;
  taskId: string;
  seq: number;
  direction: ProtocolCardDirection;
  type: string;
  payload: ProtocolOpPayload | ProtocolEventPayload;
  level: ProtocolCardLevel;
  summary: string;
  timestamp: number;
  streaming?: boolean;
  retryable?: boolean;
  retryInput?: InputCard;
}

export interface Conversation {
  id: string;
  title: string;
  pinned?: boolean;
  messages: Message[];
  protocolCards: ProtocolCard[];
  createdAt: number;
  updatedAt: number;
}

export const enum AgentProvider {
  OpenAi = 'open_ai',
  Glm = 'glm',
  GlmCoding = 'glm_coding',
  Anthropic = 'anthropic',
  Local = 'local',
}

export interface McpServerConfig {
  name: string;
  enabled: boolean;
  command?: string;
  args: string[];
  env: Record<string, string>;
}

export interface McpRuntimeConfig {
  enabled: boolean;
  defaultTimeoutSecs?: number;
  maxRetries?: number;
  servers: McpServerConfig[];
  imageRecognition: McpImageRecognitionConfig;
}

export interface McpImageRecognitionConfig {
  enabled: boolean;
  serverName: string;
  toolName: string;
  argsTemplate: Record<string, unknown>;
}

export interface SkillsRuntimeConfig {
  enabled: boolean;
  personalDir?: string;
  projectDirs: string[];
  autoApply: boolean;
}

export interface WorkspaceConfig {
  rootDir: string;
}

export interface AppConfig {
  provider: AgentProvider;
  model: string;
  modelSupportsImageInput: boolean;
  apiKeyEnv: string;
  apiKey?: string;
  baseUrl?: string;
  maxTokens?: number;
  workspace: WorkspaceConfig;
  mcp: McpRuntimeConfig;
  skills: SkillsRuntimeConfig;
}

export const enum AgentEventType {
  Started = 'started',
  Delta = 'delta',
  Completed = 'completed',
  Error = 'error',
  OpSubmitted = 'op_submitted',
  ProtocolEvent = 'protocol_event',
}

export interface AgentEvent {
  type: AgentEventType;
  task_id: string;
  chunk?: string;
  output?: string;
  code?: string;
  message?: string;
  seq?: number;
  payload?: ProtocolOpPayload | ProtocolEventPayload;
}

export interface McpServerTestResult {
  name: string;
  enabled: boolean;
  success: boolean;
  latencyMs: number;
  toolCount: number;
  tools: string[];
  error?: string;
}

export interface McpConfigTestResult {
  success: boolean;
  message: string;
  serverResults: McpServerTestResult[];
}

export interface SkillScanEntry {
  name: string;
  description: string;
  path: string;
  source: string;
  hasAuxiliaryFiles: boolean;
}

export interface SkillScanResult {
  success: boolean;
  message: string;
  warnings: string[];
  skills: SkillScanEntry[];
}

export interface TauriResponse<T = unknown> {
  success?: boolean;
  data?: T;
  error?: string;
}

export interface LegacySessionSnapshot {
  conversations: Conversation[];
  currentConversationId?: string | null;
}

export interface StorageBootstrapRequest {
  legacy?: LegacySessionSnapshot | null;
}

export interface StorageBootstrapResponse {
  conversations: Conversation[];
  currentConversationId?: string | null;
  migratedLegacy: boolean;
}

export type MemorySearchScope = 'workspace' | 'global' | 'both';

export interface MemorySearchRequest {
  query: string;
  scope?: MemorySearchScope;
  limit?: number;
  includeSecrets?: boolean;
}

export interface MemorySearchEntry {
  source: string;
  kind: string;
  memoryId?: number | null;
  conversationId?: string | null;
  messageId?: string | null;
  label: string;
  snippet: string;
  score: number;
  secret: boolean;
}

export interface MemorySearchResponse {
  results: MemorySearchEntry[];
  sqliteVecAvailable: boolean;
}

export interface MemoryUpsertPersonalNoteRequest {
  label: string;
  descriptorText: string;
  scope?: string;
}

export interface MemoryUpsertPersonalSecretRequest {
  label: string;
  descriptorText: string;
  secretText: string;
  scope?: string;
}

export interface PersonalMemoryEntry {
  id: number;
  kind: string;
  label: string;
  descriptorText: string;
  scope: string;
  createdAt: number;
  updatedAt: number;
  hasSecret: boolean;
}

export interface PersonalMemoryListResponse {
  items: PersonalMemoryEntry[];
}

export interface MemoryDeletePersonalRequest {
  id: number;
}

// Permission and Approval System Types

export interface ToolApprovalRequest {
  requestId: string;
  taskId: string;
  tool: string;
  args: unknown;
  riskLevel: 'low' | 'medium' | 'high';
  reason: string;
  expiresAtUnixMs: number;
  timestamp: number;
}

export interface PermissionPolicy {
  approvalPolicy: ApprovalPolicy;
  toolAllowList: string[];
  toolDenyList: string[];
  rememberApprovals: boolean;
}

export interface ApprovalResolution {
  requestId: string;
  approved: boolean;
  rememberChoice: boolean;
}
