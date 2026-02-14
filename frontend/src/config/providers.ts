import { AgentProvider, type AppConfig } from '../types';

export interface ProviderPreset {
  value: AgentProvider;
  label: string;
  description: string;
  defaultModel: string;
  modelHint: string;
  defaultApiKeyEnv: string;
  defaultSystemPrompt: string;
  promptHint: string;
  supportsApiKey: boolean;
  supportsBaseUrl: boolean;
  supportsMaxTokens?: boolean;
  apiKeyLabel?: string;
  apiKeyPlaceholder?: string;
  baseUrlLabel?: string;
  baseUrlPlaceholder?: string;
}

export const PROVIDER_PRESETS: ProviderPreset[] = [
  {
    value: AgentProvider.OpenAi,
    label: 'OpenAI',
    description: 'OpenAI 官方 API',
    defaultModel: 'gpt-4o-mini',
    modelHint: '推荐：gpt-4o-mini（性价比）或 gpt-4.1（效果更强）。',
    defaultApiKeyEnv: 'OPENAI_API_KEY',
    defaultSystemPrompt: '你是一个严谨、高效的 AI 助手。请优先给出结论与可执行步骤。',
    promptHint: '适合通用问答、代码解释与文档整理。',
    supportsApiKey: true,
    supportsBaseUrl: false,
    apiKeyLabel: 'OpenAI API Key',
    apiKeyPlaceholder: 'sk-...',
  },
  {
    value: AgentProvider.Glm,
    label: 'GLM',
    description: '智谱标准 API',
    defaultModel: 'glm-5',
    modelHint: '推荐：glm-5（通用）或 glm-4.7-flashx（响应更快）。',
    defaultApiKeyEnv: 'GLM_API_KEY',
    defaultSystemPrompt: '你是一个专业中文 AI 助手。请先给结论，再给步骤，必要时给示例。',
    promptHint: '适合中文问答、代码修复与工具调用任务。',
    supportsApiKey: true,
    supportsBaseUrl: true,
    apiKeyLabel: 'GLM API Key',
    apiKeyPlaceholder: 'Enter GLM API key',
    baseUrlLabel: 'GLM Base URL',
    baseUrlPlaceholder: 'https://open.bigmodel.cn/api/paas/v4/chat/completions',
  },
  {
    value: AgentProvider.GlmCoding,
    label: 'GLM Coding Plan',
    description: '智谱 Coding Plan API',
    defaultModel: 'glm-5-coding',
    modelHint: '推荐：glm-5-coding（优先）或 glm-4.7-coding。',
    defaultApiKeyEnv: 'GLM_API_KEY',
    defaultSystemPrompt: '你是资深编程助手。请输出可直接执行的改动方案，并附验证步骤。',
    promptHint: '适合重构、多文件修改与复杂工程任务。',
    supportsApiKey: true,
    supportsBaseUrl: true,
    apiKeyLabel: 'GLM API Key',
    apiKeyPlaceholder: 'Enter GLM API key',
    baseUrlLabel: 'Coding Plan Base URL',
    baseUrlPlaceholder: 'https://open.bigmodel.cn/api/coding/paas/v4/chat/completions',
  },
  {
    value: AgentProvider.Anthropic,
    label: 'Anthropic',
    description: 'Claude 模型 API',
    defaultModel: 'claude-3-5-sonnet-latest',
    modelHint: '推荐：claude-3-5-sonnet-latest。',
    defaultApiKeyEnv: 'ANTHROPIC_API_KEY',
    defaultSystemPrompt: '你是重视结构化表达的 AI 助手。请分点清晰说明，并标注关键风险。',
    promptHint: '适合长上下文分析、评审与复杂写作。',
    supportsApiKey: true,
    supportsBaseUrl: true,
    supportsMaxTokens: true,
    apiKeyLabel: 'Anthropic API Key',
    apiKeyPlaceholder: 'sk-ant-...',
    baseUrlLabel: 'Anthropic Base URL',
    baseUrlPlaceholder: 'https://api.anthropic.com/v1/messages',
  },
  {
    value: AgentProvider.Local,
    label: '本地模型 (Ollama/OpenAI 兼容)',
    description: '本地模型或自建网关',
    defaultModel: 'qwen2.5-coder:7b',
    modelHint: '推荐：qwen2.5-coder:7b 或 deepseek-coder 系列。',
    defaultApiKeyEnv: 'LOCAL_API_KEY',
    defaultSystemPrompt: '你是本地离线编码助手。回答应简洁、可落地，并优先结合当前项目上下文。',
    promptHint: '适合私有环境、本地调试和低延迟场景。',
    supportsApiKey: false,
    supportsBaseUrl: true,
    baseUrlLabel: 'Local Base URL',
    baseUrlPlaceholder: 'http://localhost:11434/api/chat',
  },
];

const PRESET_MAP = new Map(PROVIDER_PRESETS.map((preset) => [preset.value, preset]));

export const getProviderPreset = (provider: AgentProvider): ProviderPreset =>
  PRESET_MAP.get(provider) ?? PRESET_MAP.get(AgentProvider.OpenAi)!;

export const createDefaultConfig = (provider: AgentProvider = AgentProvider.OpenAi): AppConfig => {
  const preset = getProviderPreset(provider);
  return {
    provider: preset.value,
    model: preset.defaultModel,
    apiKeyEnv: preset.defaultApiKeyEnv,
    apiKey: '',
    baseUrl: preset.supportsBaseUrl ? '' : undefined,
    maxTokens: preset.supportsMaxTokens ? 1024 : undefined,
    systemPrompt: preset.defaultSystemPrompt,
  };
};

export const applyProviderDefaults = (config: AppConfig, provider: AgentProvider): AppConfig => {
  const preset = getProviderPreset(provider);
  return {
    ...config,
    provider,
    model: preset.defaultModel,
    apiKeyEnv: preset.defaultApiKeyEnv,
    apiKey: '',
    baseUrl: preset.supportsBaseUrl ? '' : undefined,
    maxTokens: preset.supportsMaxTokens ? (config.maxTokens ?? 1024) : undefined,
    systemPrompt: preset.defaultSystemPrompt,
  };
};

const normalizeProvider = (value: unknown): AgentProvider => {
  if (typeof value !== 'string') {
    return AgentProvider.OpenAi;
  }
  const normalized = value.trim().toLowerCase();
  switch (normalized) {
    case 'open_ai':
    case 'openai':
      return AgentProvider.OpenAi;
    case 'glm':
      return AgentProvider.Glm;
    case 'glm_coding':
    case 'glm-coding':
    case 'glmcoding':
      return AgentProvider.GlmCoding;
    case 'anthropic':
      return AgentProvider.Anthropic;
    case 'local':
    case 'local-llm':
    case 'local_llm':
      return AgentProvider.Local;
    default:
      if (value === 'OpenAi') {
        return AgentProvider.OpenAi;
      }
      if (value === 'Glm') {
        return AgentProvider.Glm;
      }
      return AgentProvider.OpenAi;
  }
};

const normalizeOptionalText = (value: unknown): string | undefined => {
  if (typeof value !== 'string') {
    return undefined;
  }
  const trimmed = value.trim();
  return trimmed.length > 0 ? trimmed : undefined;
};

const normalizeMaxTokens = (value: unknown): number | undefined => {
  if (typeof value === 'number' && Number.isFinite(value) && value > 0) {
    return Math.floor(value);
  }
  if (typeof value === 'string') {
    const parsed = Number.parseInt(value, 10);
    if (Number.isFinite(parsed) && parsed > 0) {
      return parsed;
    }
  }
  return undefined;
};

export const normalizeStoredConfig = (raw: unknown): Partial<AppConfig> => {
  if (!raw || typeof raw !== 'object') {
    return {};
  }

  const source = raw as Record<string, unknown>;
  const provider = normalizeProvider(source.provider);
  const preset = getProviderPreset(provider);

  const model = normalizeOptionalText(source.model) ?? preset.defaultModel;
  const apiKeyEnv = normalizeOptionalText(source.apiKeyEnv) ?? preset.defaultApiKeyEnv;

  const legacyApiKey =
    provider === AgentProvider.OpenAi
      ? normalizeOptionalText(source.openAiApiKey)
      : normalizeOptionalText(source.glmApiKey);

  const apiKey = normalizeOptionalText(source.apiKey) ?? legacyApiKey ?? '';
  const baseUrl = normalizeOptionalText(source.baseUrl) ?? normalizeOptionalText(source.glmUrl) ?? '';
  const systemPrompt =
    typeof source.systemPrompt === 'string'
      ? source.systemPrompt
      : createDefaultConfig(provider).systemPrompt;

  const normalized: Partial<AppConfig> = {
    provider,
    model,
    apiKeyEnv,
    apiKey,
    systemPrompt,
  };

  if (preset.supportsBaseUrl) {
    normalized.baseUrl = baseUrl;
  }

  if (preset.supportsMaxTokens) {
    normalized.maxTokens = normalizeMaxTokens(source.maxTokens) ?? 1024;
  }

  return normalized;
};
