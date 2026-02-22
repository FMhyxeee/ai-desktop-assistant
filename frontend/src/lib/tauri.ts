import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import {
  type AgentHistoryMessage,
  type AgentEvent,
  type AppConfig,
  type Conversation,
  type GovernanceReport,
  type GovernanceUpdateReport,
  type InputCard,
  type MemoryDeletePersonalRequest,
  type MemorySearchRequest,
  type MemorySearchResponse,
  type MemoryUpsertPersonalNoteRequest,
  type MemoryUpsertPersonalSecretRequest,
  type McpConfigTestResult,
  type PersonalMemoryEntry,
  type PersonalMemoryListResponse,
  type SkillScanResult,
  type StorageBootstrapRequest,
  type StorageBootstrapResponse,
} from '../types';

interface RuntimeMcpServerPayload {
  name: string;
  enabled: boolean;
  command: string | null;
  args: string[];
  env: Record<string, string>;
}

interface RuntimeMcpImageRecognitionPayload {
  enabled: boolean;
  serverName: string;
  toolName: string;
  argsTemplate: Record<string, unknown>;
}

interface RuntimeMcpPayload {
  enabled: boolean;
  defaultTimeoutSecs: number | null;
  maxRetries: number | null;
  servers: RuntimeMcpServerPayload[];
  imageRecognition: RuntimeMcpImageRecognitionPayload;
}

interface RuntimeSkillsPayload {
  enabled: boolean;
  personalDir: string | null;
  projectDirs: string[];
  autoApply: boolean;
}

interface RuntimeWorkspacePayload {
  rootDir: string;
}

interface RuntimeConfigPayload {
  provider: string;
  model: string;
  modelSupportsImageInput: boolean;
  apiKeyEnv: string;
  apiKey: string | null;
  baseUrl: string | null;
  maxTokens: number | null;
  systemPrompt: string;
  workspace: RuntimeWorkspacePayload;
  mcp: RuntimeMcpPayload;
  skills: RuntimeSkillsPayload;
}

interface StreamInputPayload {
  content: string;
  images: Array<{
    name: string;
    mimeType: string;
    dataUrl: string;
    sizeBytes: number;
  }>;
  conversationId?: string;
  recentMessages: AgentHistoryMessage[];
}

export interface RuntimeConnectionTestResult {
  success: boolean;
  message: string;
  latencyMs: number;
}

const normalizeOptionalText = (value?: string): string | null => {
  const normalized = value?.trim() ?? '';
  return normalized.length > 0 ? normalized : null;
};

const normalizePositiveInt = (value: number | undefined): number | null => {
  if (typeof value === 'number' && Number.isFinite(value) && value > 0) {
    return Math.floor(value);
  }
  return null;
};

const normalizeStringMap = (source: Record<string, string> | undefined): Record<string, string> => {
  if (!source) {
    return {};
  }
  const output: Record<string, string> = {};
  Object.entries(source).forEach(([key, rawValue]) => {
    const normalizedKey = key.trim();
    const normalizedValue = rawValue.trim();
    if (normalizedKey && normalizedValue) {
      output[normalizedKey] = normalizedValue;
    }
  });
  return output;
};

const normalizeJsonObject = (value: unknown): Record<string, unknown> => {
  if (!value || typeof value !== 'object' || Array.isArray(value)) {
    return {};
  }
  return value as Record<string, unknown>;
};

const toRuntimeMcpPayload = (config: AppConfig): RuntimeMcpPayload => ({
  enabled: Boolean(config.mcp.enabled),
  defaultTimeoutSecs: normalizePositiveInt(config.mcp.defaultTimeoutSecs),
  maxRetries: normalizePositiveInt(config.mcp.maxRetries),
  servers: config.mcp.servers.map((server) => ({
    name: server.name.trim(),
    enabled: Boolean(server.enabled),
    command: normalizeOptionalText(server.command),
    args: (server.args ?? [])
      .map((arg) => arg.trim())
      .filter((arg) => arg.length > 0),
    env: normalizeStringMap(server.env),
  })),
  imageRecognition: {
    enabled: Boolean(config.mcp.imageRecognition.enabled),
    serverName: config.mcp.imageRecognition.serverName.trim(),
    toolName: config.mcp.imageRecognition.toolName.trim(),
    argsTemplate: normalizeJsonObject(config.mcp.imageRecognition.argsTemplate),
  },
});

const toRuntimeSkillsPayload = (config: AppConfig): RuntimeSkillsPayload => ({
  enabled: Boolean(config.skills.enabled),
  personalDir: normalizeOptionalText(config.skills.personalDir),
  projectDirs: (config.skills.projectDirs ?? [])
    .map((dir) => dir.trim())
    .filter((dir) => dir.length > 0),
  autoApply: Boolean(config.skills.autoApply),
});

const toRuntimeWorkspacePayload = (config: AppConfig): RuntimeWorkspacePayload => ({
  rootDir: normalizeOptionalText(config.workspace.rootDir) ?? '',
});

const toRuntimeConfigPayload = (config: AppConfig): RuntimeConfigPayload => {
  const apiKey = normalizeOptionalText(config.apiKey);
  const baseUrl = normalizeOptionalText(config.baseUrl);

  return {
    provider: config.provider,
    model: config.model.trim(),
    modelSupportsImageInput: Boolean(config.modelSupportsImageInput),
    apiKeyEnv: config.apiKeyEnv.trim(),
    apiKey,
    baseUrl,
    maxTokens: normalizePositiveInt(config.maxTokens),
    systemPrompt: config.systemPrompt.trim(),
    workspace: toRuntimeWorkspacePayload(config),
    mcp: toRuntimeMcpPayload(config),
    skills: toRuntimeSkillsPayload(config),
  };
};

export type FrontendLogLevel = 'debug' | 'info' | 'warn' | 'error';

export class TauriAPI {
  static async askAgent(input: string): Promise<string> {
    try {
      return await invoke<string>('ask_agent', { input });
    } catch (error) {
      console.error('ask_agent error:', error);
      throw new Error(typeof error === 'string' ? error : 'Failed to ask agent');
    }
  }

  static async startAgentStream(
    input: string | InputCard,
    taskId?: string,
    options?: {
      conversationId?: string;
      recentMessages?: AgentHistoryMessage[];
    }
  ): Promise<string> {
    const streamInput: string | StreamInputPayload =
      typeof input === 'string'
        ? input
        : {
            content: input.content,
            images: (input.images ?? []).map((image) => ({
              name: image.name,
              mimeType: image.mimeType,
              dataUrl: image.dataUrl,
              sizeBytes: image.sizeBytes,
            })),
            conversationId: options?.conversationId,
            recentMessages: options?.recentMessages ?? [],
          };
    try {
      return await invoke<string>('start_agent_stream', {
        input: streamInput,
        taskId,
      });
    } catch (error) {
      console.error('start_agent_stream error:', error);
      throw new Error(typeof error === 'string' ? error : 'Failed to start stream');
    }
  }

  static async resolveConfigChangeRequest(
    taskId: string,
    requestId: string,
    approved: boolean,
    persist: boolean
  ): Promise<void> {
    try {
      await invoke('resolve_config_change_request', {
        taskId,
        requestId,
        approved,
        persist,
      });
    } catch (error) {
      console.error('resolve_config_change_request error:', error);
      throw new Error(typeof error === 'string' ? error : 'Failed to resolve config change request');
    }
  }

  static async cancelAgentTask(taskId: string): Promise<void> {
    try {
      await invoke('cancel_agent_task', { taskId });
    } catch (error) {
      console.error('cancel_agent_task error:', error);
      throw new Error(typeof error === 'string' ? error : 'Failed to cancel task');
    }
  }

  static async updateRuntimeConfig(config: AppConfig): Promise<GovernanceUpdateReport> {
    try {
      return await invoke<GovernanceUpdateReport>('update_runtime_config', {
        config: toRuntimeConfigPayload(config),
      });
    } catch (error) {
      console.error('update_runtime_config error:', error);
      throw new Error(typeof error === 'string' ? error : 'Failed to update runtime config');
    }
  }

  static async testRuntimeConfig(config: AppConfig): Promise<RuntimeConnectionTestResult> {
    try {
      return await invoke<RuntimeConnectionTestResult>('test_runtime_config', {
        config: toRuntimeConfigPayload(config),
      });
    } catch (error) {
      console.error('test_runtime_config error:', error);
      throw new Error(typeof error === 'string' ? error : 'Failed to test runtime config');
    }
  }

  static async testMcpConfig(config: AppConfig): Promise<McpConfigTestResult> {
    try {
      return await invoke<McpConfigTestResult>('test_mcp_config', {
        config: toRuntimeMcpPayload(config),
      });
    } catch (error) {
      console.error('test_mcp_config error:', error);
      throw new Error(typeof error === 'string' ? error : 'Failed to test MCP config');
    }
  }

  static async scanSkillsConfig(config: AppConfig): Promise<SkillScanResult> {
    try {
      return await invoke<SkillScanResult>('scan_skills_config', {
        config: toRuntimeSkillsPayload(config),
        workspace: toRuntimeWorkspacePayload(config),
      });
    } catch (error) {
      console.error('scan_skills_config error:', error);
      throw new Error(typeof error === 'string' ? error : 'Failed to scan skills config');
    }
  }

  static async runGovernanceScan(config?: AppConfig): Promise<GovernanceReport> {
    try {
      const payload = config ? toRuntimeConfigPayload(config) : null;
      return await invoke<GovernanceReport>('run_governance_scan', {
        config: payload,
      });
    } catch (error) {
      console.error('run_governance_scan error:', error);
      throw new Error(typeof error === 'string' ? error : 'Failed to run governance scan');
    }
  }

  static async listenAgentEvents(callback: (event: AgentEvent) => void): Promise<() => void> {
    try {
      const unlisten = await listen<AgentEvent>('agent://event', (event) => {
        callback(event.payload);
      });
      return unlisten;
    } catch (error) {
      console.error('listenAgentEvents error:', error);
      throw new Error('Failed to listen to agent events');
    }
  }

  static async frontendLog(
    level: FrontendLogLevel,
    message: string,
    context?: unknown
  ): Promise<void> {
    try {
      await invoke('frontend_log', {
        level,
        message,
        context: context ?? null,
      });
    } catch {
      // Keep console path as the fallback when Tauri command is unavailable.
    }
  }

  static async storageBootstrap(
    request?: StorageBootstrapRequest
  ): Promise<StorageBootstrapResponse> {
    try {
      return await invoke<StorageBootstrapResponse>('storage_bootstrap', {
        request: request ?? null,
      });
    } catch (error) {
      console.error('storage_bootstrap error:', error);
      throw new Error(
        typeof error === 'string' ? error : 'Failed to bootstrap workspace storage'
      );
    }
  }

  static async storageUpsertConversation(conversation: Conversation): Promise<void> {
    try {
      await invoke('storage_upsert_conversation', { conversation });
    } catch (error) {
      console.error('storage_upsert_conversation error:', error);
      throw new Error(
        typeof error === 'string' ? error : 'Failed to upsert conversation'
      );
    }
  }

  static async storageDeleteConversation(conversationId: string): Promise<void> {
    try {
      await invoke('storage_delete_conversation', { conversationId });
    } catch (error) {
      console.error('storage_delete_conversation error:', error);
      throw new Error(
        typeof error === 'string' ? error : 'Failed to delete conversation'
      );
    }
  }

  static async storageSetCurrentConversation(
    conversationId: string | null
  ): Promise<void> {
    try {
      await invoke('storage_set_current_conversation', { conversationId });
    } catch (error) {
      console.error('storage_set_current_conversation error:', error);
      throw new Error(
        typeof error === 'string' ? error : 'Failed to persist current conversation'
      );
    }
  }

  static async storageExportConversation(
    conversationId: string
  ): Promise<Conversation> {
    try {
      return await invoke<Conversation>('storage_export_conversation', {
        conversationId,
      });
    } catch (error) {
      console.error('storage_export_conversation error:', error);
      throw new Error(
        typeof error === 'string' ? error : 'Failed to export conversation'
      );
    }
  }

  static async memorySearch(
    request: MemorySearchRequest
  ): Promise<MemorySearchResponse> {
    try {
      return await invoke<MemorySearchResponse>('memory_search', { request });
    } catch (error) {
      console.error('memory_search error:', error);
      throw new Error(typeof error === 'string' ? error : 'Failed to search memory');
    }
  }

  static async memoryUpsertPersonalNote(
    request: MemoryUpsertPersonalNoteRequest
  ): Promise<PersonalMemoryEntry> {
    try {
      return await invoke<PersonalMemoryEntry>('memory_upsert_personal_note', {
        request,
      });
    } catch (error) {
      console.error('memory_upsert_personal_note error:', error);
      throw new Error(
        typeof error === 'string' ? error : 'Failed to upsert personal note'
      );
    }
  }

  static async memoryUpsertPersonalSecret(
    request: MemoryUpsertPersonalSecretRequest
  ): Promise<PersonalMemoryEntry> {
    try {
      return await invoke<PersonalMemoryEntry>('memory_upsert_personal_secret', {
        request,
      });
    } catch (error) {
      console.error('memory_upsert_personal_secret error:', error);
      throw new Error(
        typeof error === 'string' ? error : 'Failed to upsert personal secret'
      );
    }
  }

  static async memoryListPersonal(): Promise<PersonalMemoryListResponse> {
    try {
      return await invoke<PersonalMemoryListResponse>('memory_list_personal');
    } catch (error) {
      console.error('memory_list_personal error:', error);
      throw new Error(
        typeof error === 'string' ? error : 'Failed to list personal memory'
      );
    }
  }

  static async memoryDeletePersonal(
    request: MemoryDeletePersonalRequest
  ): Promise<void> {
    try {
      await invoke('memory_delete_personal', { request });
    } catch (error) {
      console.error('memory_delete_personal error:', error);
      throw new Error(
        typeof error === 'string' ? error : 'Failed to delete personal memory'
      );
    }
  }

  static async setStore(key: string, value: unknown): Promise<void> {
    try {
      localStorage.setItem(key, JSON.stringify(value));
    } catch (error) {
      console.error('setStore error:', error);
      throw new Error('Failed to save to store');
    }
  }

  static async getStore<T>(key: string, defaultValue: T): Promise<T> {
    try {
      const item = localStorage.getItem(key);
      return item ? (JSON.parse(item) as T) : defaultValue;
    } catch (error) {
      console.error('getStore error:', error);
      return defaultValue;
    }
  }

  static async deleteStore(key: string): Promise<void> {
    try {
      localStorage.removeItem(key);
    } catch (error) {
      console.error('deleteStore error:', error);
      throw new Error('Failed to delete from store');
    }
  }
}
