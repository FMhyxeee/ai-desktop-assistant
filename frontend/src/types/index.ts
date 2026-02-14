export interface Message {
  id: string;
  role: 'user' | 'assistant' | 'system';
  content: string;
  timestamp: number;
  isStreaming?: boolean;
}

export interface Conversation {
  id: string;
  title: string;
  messages: Message[];
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
}

export interface AgentEvent {
  type: AgentEventType;
  task_id: string;
  chunk?: string;
  output?: string;
  code?: string;
  message?: string;
}

export interface TauriResponse<T = unknown> {
  success?: boolean;
  data?: T;
  error?: string;
}
