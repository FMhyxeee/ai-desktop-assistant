export interface Message {
  id: string;
  role: 'user' | 'assistant' | 'system';
  content: string;
  timestamp: number;
  isStreaming?: boolean;
}

export const enum InputCardKind {
  Text = 'text',
  Command = 'command',
}

export interface InputCard {
  kind: InputCardKind;
  content: string;
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

export type ProtocolOpPayload =
  | {
      type: 'user_turn';
      model: string;
      cwd: string;
      approval_policy: string;
      sandbox_policy: string;
      text: string;
    }
  | {
      type: 'run_user_shell_command';
      command: string;
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

export const enum McpTransportKind {
  Stdio = 'stdio',
  Tcp = 'tcp',
  Http = 'http',
  Https = 'https',
  Websocket = 'websocket',
  Wss = 'wss',
  Sse = 'sse',
}

export const enum McpAuthType {
  None = 'none',
  Bearer = 'bearer',
  Basic = 'basic',
  ApiKey = 'api_key',
  OAuth2 = 'oauth2',
}

export interface McpAuthConfig {
  type: McpAuthType;
  tokenEnv?: string;
  usernameEnv?: string;
  passwordEnv?: string;
  apiKeyEnv?: string;
  apiKeyHeader?: string;
  queryParam?: string;
  tokenUrl?: string;
  clientIdEnv?: string;
  clientSecretEnv?: string;
  scope?: string;
  audience?: string;
}

export interface McpTlsConfig {
  caCertPath?: string;
  clientCertPath?: string;
  clientKeyPath?: string;
  dangerAcceptInvalidCerts?: boolean;
  dangerAcceptInvalidHostnames?: boolean;
}

export interface McpServerConfig {
  name: string;
  enabled: boolean;
  transport: McpTransportKind;
  endpoint?: string;
  command?: string;
  args: string[];
  timeoutSecs?: number;
  env: Record<string, string>;
  headers: Record<string, string>;
  auth?: McpAuthConfig;
  tls?: McpTlsConfig;
}

export interface McpRuntimeConfig {
  enabled: boolean;
  defaultTimeoutSecs?: number;
  maxRetries?: number;
  servers: McpServerConfig[];
}

export interface SkillsRuntimeConfig {
  enabled: boolean;
  personalDir?: string;
  projectDirs: string[];
  autoApply: boolean;
}

export interface AppConfig {
  provider: AgentProvider;
  model: string;
  apiKeyEnv: string;
  apiKey?: string;
  baseUrl?: string;
  maxTokens?: number;
  systemPrompt: string;
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
