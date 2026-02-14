// 消息类型
export interface Message {
  id: string;
  role: 'user' | 'assistant' | 'system';
  content: string;
  timestamp: number;
  isStreaming?: boolean;
}

// 对话会话类型
export interface Conversation {
  id: string;
  title: string;
  messages: Message[];
  createdAt: number;
  updatedAt: number;
}

// AI 提供商
export const enum AgentProvider {
  OpenAi = 'OpenAi',
  Glm = 'Glm',
}

// 配置类型
export interface AppConfig {
  provider: AgentProvider;
  model: string;
  apiKeyEnv: string;
  systemPrompt: string;
  openAiApiKey?: string;
  glmApiKey?: string;
  glmUrl?: string;
}

// Agent 事件类型
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

// Tauri 命令响应
export interface TauriResponse<T = any> {
  success?: boolean;
  data?: T;
  error?: string;
}
