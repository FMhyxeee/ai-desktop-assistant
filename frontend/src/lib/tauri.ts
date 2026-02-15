import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import type {
  AgentEvent,
  AppConfig,
  InputCard,
  McpConfigTestResult,
  SkillScanResult,
} from '../types';

interface RuntimeMcpAuthPayload {
  type: string;
  tokenEnv: string | null;
  usernameEnv: string | null;
  passwordEnv: string | null;
  apiKeyEnv: string | null;
  apiKeyHeader: string | null;
  queryParam: string | null;
  tokenUrl: string | null;
  clientIdEnv: string | null;
  clientSecretEnv: string | null;
  scope: string | null;
  audience: string | null;
}

interface RuntimeMcpTlsPayload {
  caCertPath: string | null;
  clientCertPath: string | null;
  clientKeyPath: string | null;
  dangerAcceptInvalidCerts: boolean;
  dangerAcceptInvalidHostnames: boolean;
}

interface RuntimeMcpServerPayload {
  name: string;
  enabled: boolean;
  transport: string;
  endpoint: string | null;
  command: string | null;
  args: string[];
  timeoutSecs: number | null;
  env: Record<string, string>;
  headers: Record<string, string>;
  auth: RuntimeMcpAuthPayload | null;
  tls: RuntimeMcpTlsPayload | null;
}

interface RuntimeMcpPayload {
  enabled: boolean;
  defaultTimeoutSecs: number | null;
  maxRetries: number | null;
  servers: RuntimeMcpServerPayload[];
}

interface RuntimeSkillsPayload {
  enabled: boolean;
  personalDir: string | null;
  projectDirs: string[];
  autoApply: boolean;
}

interface RuntimeConfigPayload {
  provider: string;
  model: string;
  apiKeyEnv: string;
  apiKey: string | null;
  baseUrl: string | null;
  maxTokens: number | null;
  systemPrompt: string;
  mcp: RuntimeMcpPayload;
  skills: RuntimeSkillsPayload;
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

const toRuntimeMcpPayload = (config: AppConfig): RuntimeMcpPayload => ({
  enabled: Boolean(config.mcp.enabled),
  defaultTimeoutSecs: normalizePositiveInt(config.mcp.defaultTimeoutSecs),
  maxRetries: normalizePositiveInt(config.mcp.maxRetries),
  servers: config.mcp.servers.map((server) => ({
    name: server.name.trim(),
    enabled: Boolean(server.enabled),
    transport: server.transport,
    endpoint: normalizeOptionalText(server.endpoint),
    command: normalizeOptionalText(server.command),
    args: (server.args ?? [])
      .map((arg) => arg.trim())
      .filter((arg) => arg.length > 0),
    timeoutSecs: normalizePositiveInt(server.timeoutSecs),
    env: normalizeStringMap(server.env),
    headers: normalizeStringMap(server.headers),
    auth: server.auth
      ? {
          type: server.auth.type,
          tokenEnv: normalizeOptionalText(server.auth.tokenEnv),
          usernameEnv: normalizeOptionalText(server.auth.usernameEnv),
          passwordEnv: normalizeOptionalText(server.auth.passwordEnv),
          apiKeyEnv: normalizeOptionalText(server.auth.apiKeyEnv),
          apiKeyHeader: normalizeOptionalText(server.auth.apiKeyHeader),
          queryParam: normalizeOptionalText(server.auth.queryParam),
          tokenUrl: normalizeOptionalText(server.auth.tokenUrl),
          clientIdEnv: normalizeOptionalText(server.auth.clientIdEnv),
          clientSecretEnv: normalizeOptionalText(server.auth.clientSecretEnv),
          scope: normalizeOptionalText(server.auth.scope),
          audience: normalizeOptionalText(server.auth.audience),
        }
      : null,
    tls: server.tls
      ? {
          caCertPath: normalizeOptionalText(server.tls.caCertPath),
          clientCertPath: normalizeOptionalText(server.tls.clientCertPath),
          clientKeyPath: normalizeOptionalText(server.tls.clientKeyPath),
          dangerAcceptInvalidCerts: Boolean(server.tls.dangerAcceptInvalidCerts),
          dangerAcceptInvalidHostnames: Boolean(server.tls.dangerAcceptInvalidHostnames),
        }
      : null,
  })),
});

const toRuntimeSkillsPayload = (config: AppConfig): RuntimeSkillsPayload => ({
  enabled: Boolean(config.skills.enabled),
  personalDir: normalizeOptionalText(config.skills.personalDir),
  projectDirs: (config.skills.projectDirs ?? [])
    .map((dir) => dir.trim())
    .filter((dir) => dir.length > 0),
  autoApply: Boolean(config.skills.autoApply),
});

const toRuntimeConfigPayload = (config: AppConfig): RuntimeConfigPayload => {
  const apiKey = normalizeOptionalText(config.apiKey);
  const baseUrl = normalizeOptionalText(config.baseUrl);

  return {
    provider: config.provider,
    model: config.model.trim(),
    apiKeyEnv: config.apiKeyEnv.trim(),
    apiKey,
    baseUrl,
    maxTokens: normalizePositiveInt(config.maxTokens),
    systemPrompt: config.systemPrompt.trim(),
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

  static async startAgentStream(input: string | InputCard, taskId?: string): Promise<string> {
    try {
      return await invoke<string>('start_agent_stream', {
        input,
        taskId,
      });
    } catch (error) {
      console.error('start_agent_stream error:', error);
      throw new Error(typeof error === 'string' ? error : 'Failed to start stream');
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

  static async updateRuntimeConfig(config: AppConfig): Promise<void> {
    try {
      await invoke('update_runtime_config', {
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
      });
    } catch (error) {
      console.error('scan_skills_config error:', error);
      throw new Error(typeof error === 'string' ? error : 'Failed to scan skills config');
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
