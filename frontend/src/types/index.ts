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

export interface AppConfig {
  provider: AgentProvider;
  model: string;
  apiKeyEnv: string;
  apiKey?: string;
  baseUrl?: string;
  maxTokens?: number;
  systemPrompt: string;
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

export interface TauriResponse<T = unknown> {
  success?: boolean;
  data?: T;
  error?: string;
}
