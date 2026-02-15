import React from 'react';
import { Blocks, Bot, Eye, EyeOff, PlugZap, Plus, Save, Sparkles, Trash2, X } from 'lucide-react';
import {
  applyProviderDefaults,
  createDefaultMcpServer,
  getProviderPreset,
  normalizeMcpRuntimeConfig,
  PROVIDER_PRESETS,
} from '../config/providers';
import { TauriAPI } from '../lib/tauri';
import { useAppStore } from '../store/appStore';
import {
  AgentProvider,
  McpAuthType,
  McpTransportKind,
  type AppConfig,
  type McpConfigTestResult,
  type McpServerConfig,
  type SkillScanResult,
} from '../types';

interface SettingsModalProps {
  isOpen: boolean;
  onClose: () => void;
}

type SettingsTab = 'model' | 'mcp' | 'skills';

type Feedback = {
  type: 'success' | 'error';
  message: string;
};

const TRANSPORT_OPTIONS: Array<{ value: McpTransportKind; label: string }> = [
  { value: McpTransportKind.Stdio, label: 'stdio' },
  { value: McpTransportKind.Tcp, label: 'tcp' },
  { value: McpTransportKind.Http, label: 'http' },
  { value: McpTransportKind.Https, label: 'https' },
  { value: McpTransportKind.Websocket, label: 'websocket' },
  { value: McpTransportKind.Wss, label: 'wss' },
  { value: McpTransportKind.Sse, label: 'sse' },
];

const AUTH_OPTIONS: Array<{ value: McpAuthType; label: string }> = [
  { value: McpAuthType.None, label: 'none' },
  { value: McpAuthType.Bearer, label: 'bearer' },
  { value: McpAuthType.Basic, label: 'basic' },
  { value: McpAuthType.ApiKey, label: 'api_key' },
  { value: McpAuthType.OAuth2, label: 'oauth2' },
];

const getErrorMessage = (error: unknown): string => {
  if (error instanceof Error && error.message) return error.message;
  if (typeof error === 'string' && error.trim()) return error;
  return '操作失败，请检查配置后重试。';
};

const parseLineList = (raw: string): string[] =>
  raw
    .split(/\r?\n/)
    .map((line) => line.trim())
    .filter((line) => line.length > 0);

const listToText = (list: string[]): string => list.join('\n');

const mapToText = (map: Record<string, string>): string =>
  Object.entries(map)
    .map(([key, value]) => `${key}=${value}`)
    .join('\n');

const textToMap = (raw: string): Record<string, string> => {
  const result: Record<string, string> = {};
  raw.split(/\r?\n/).forEach((line) => {
    const trimmed = line.trim();
    if (!trimmed) return;
    const sep = trimmed.indexOf('=');
    if (sep <= 0) return;
    const key = trimmed.slice(0, sep).trim();
    const value = trimmed.slice(sep + 1).trim();
    if (key && value) result[key] = value;
  });
  return result;
};

const normalizePositiveInt = (value: number | undefined): number | undefined => {
  if (typeof value !== 'number' || !Number.isFinite(value) || value <= 0) return undefined;
  return Math.floor(value);
};

const normalizeText = (value: string | undefined): string | undefined => {
  if (!value) return undefined;
  const trimmed = value.trim();
  return trimmed.length > 0 ? trimmed : undefined;
};

const normalizeStringArrayFromUnknown = (value: unknown): string[] => {
  if (!Array.isArray(value)) return [];
  return value
    .map((item) => (typeof item === 'string' ? item.trim() : ''))
    .filter((item) => item.length > 0);
};

const normalizeStringMapFromUnknown = (value: unknown): Record<string, string> => {
  if (!value || typeof value !== 'object' || Array.isArray(value)) return {};
  const source = value as Record<string, unknown>;
  const result: Record<string, string> = {};
  Object.entries(source).forEach(([key, rawValue]) => {
    const normalizedKey = key.trim();
    const normalizedValue = typeof rawValue === 'string' ? rawValue.trim() : '';
    if (normalizedKey && normalizedValue) {
      result[normalizedKey] = normalizedValue;
    }
  });
  return result;
};

const normalizeBooleanFromUnknown = (value: unknown, fallback: boolean): boolean => {
  if (typeof value === 'boolean') return value;
  if (typeof value === 'string') {
    const normalized = value.trim().toLowerCase();
    if (normalized === 'true') return true;
    if (normalized === 'false') return false;
  }
  return fallback;
};

const normalizeTransportFromUnknown = (value: unknown): McpTransportKind => {
  if (typeof value !== 'string') return McpTransportKind.Stdio;
  const normalized = value.trim().toLowerCase();
  switch (normalized) {
    case 'stdio':
      return McpTransportKind.Stdio;
    case 'tcp':
      return McpTransportKind.Tcp;
    case 'http':
      return McpTransportKind.Http;
    case 'https':
      return McpTransportKind.Https;
    case 'websocket':
    case 'ws':
      return McpTransportKind.Websocket;
    case 'wss':
      return McpTransportKind.Wss;
    case 'sse':
      return McpTransportKind.Sse;
    default:
      return McpTransportKind.Stdio;
  }
};

const isRecord = (value: unknown): value is Record<string, unknown> =>
  Boolean(value) && typeof value === 'object' && !Array.isArray(value);

const isLegacyServerShape = (value: unknown): value is Record<string, unknown> => {
  if (!isRecord(value)) return false;
  return (
    'type' in value ||
    'transport' in value ||
    'command' in value ||
    'endpoint' in value ||
    'args' in value ||
    'env' in value ||
    'headers' in value
  );
};

const toServerFromLegacyShape = (name: string, value: Record<string, unknown>): McpServerConfig => {
  const timeoutRaw = value.timeoutSecs ?? value.timeout;
  const timeoutSecs =
    typeof timeoutRaw === 'number'
      ? normalizePositiveInt(timeoutRaw)
      : typeof timeoutRaw === 'string'
      ? normalizePositiveInt(Number.parseInt(timeoutRaw, 10))
      : undefined;

  return {
    name: name.trim(),
    enabled: normalizeBooleanFromUnknown(value.enabled, true),
    transport: normalizeTransportFromUnknown(value.transport ?? value.type),
    endpoint: normalizeText(typeof value.endpoint === 'string' ? value.endpoint : '') ?? '',
    command: normalizeText(typeof value.command === 'string' ? value.command : '') ?? '',
    args: normalizeStringArrayFromUnknown(value.args),
    timeoutSecs,
    env: normalizeStringMapFromUnknown(value.env),
    headers: normalizeStringMapFromUnknown(value.headers),
    auth: {
      type: McpAuthType.None,
    },
  };
};

const normalizeConfigForSave = (config: AppConfig): AppConfig => ({
  ...config,
  model: config.model.trim(),
  apiKeyEnv: config.apiKeyEnv.trim(),
  apiKey: (config.apiKey ?? '').trim(),
  baseUrl: (config.baseUrl ?? '').trim(),
  maxTokens: normalizePositiveInt(config.maxTokens),
  systemPrompt: config.systemPrompt.trim(),
  mcp: {
    ...config.mcp,
    defaultTimeoutSecs: normalizePositiveInt(config.mcp.defaultTimeoutSecs),
    maxRetries: normalizePositiveInt(config.mcp.maxRetries),
    servers: config.mcp.servers.map((server) => ({
      ...server,
      name: server.name.trim(),
      endpoint: normalizeText(server.endpoint) ?? '',
      command: normalizeText(server.command) ?? '',
      args: server.args.map((item) => item.trim()).filter((item) => item.length > 0),
      timeoutSecs: normalizePositiveInt(server.timeoutSecs),
      env: textToMap(mapToText(server.env)),
      headers: textToMap(mapToText(server.headers)),
      auth: server.auth
        ? {
            ...server.auth,
            tokenEnv: normalizeText(server.auth.tokenEnv),
            usernameEnv: normalizeText(server.auth.usernameEnv),
            passwordEnv: normalizeText(server.auth.passwordEnv),
            apiKeyEnv: normalizeText(server.auth.apiKeyEnv),
            apiKeyHeader: normalizeText(server.auth.apiKeyHeader),
            queryParam: normalizeText(server.auth.queryParam),
            tokenUrl: normalizeText(server.auth.tokenUrl),
            clientIdEnv: normalizeText(server.auth.clientIdEnv),
            clientSecretEnv: normalizeText(server.auth.clientSecretEnv),
            scope: normalizeText(server.auth.scope),
            audience: normalizeText(server.auth.audience),
          }
        : { type: McpAuthType.None },
    })),
  },
  skills: {
    ...config.skills,
    personalDir: (normalizeText(config.skills.personalDir) ?? ''),
    projectDirs: config.skills.projectDirs.map((item) => item.trim()).filter((item) => item.length > 0),
    autoApply: Boolean(config.skills.autoApply),
  },
});

const SettingsModal: React.FC<SettingsModalProps> = ({ isOpen, onClose }) => {
  const { config, updateConfig, streamingTaskId } = useAppStore();

  const [tab, setTab] = React.useState<SettingsTab>('model');
  const [localConfig, setLocalConfig] = React.useState(config);
  const [showApiKey, setShowApiKey] = React.useState(false);

  const [isSaving, setIsSaving] = React.useState(false);
  const [isTestingModel, setIsTestingModel] = React.useState(false);
  const [isTestingMcp, setIsTestingMcp] = React.useState(false);
  const [isScanningSkills, setIsScanningSkills] = React.useState(false);

  const [saveError, setSaveError] = React.useState<string | null>(null);
  const [modelFeedback, setModelFeedback] = React.useState<Feedback | null>(null);
  const [mcpResult, setMcpResult] = React.useState<McpConfigTestResult | null>(null);
  const [mcpError, setMcpError] = React.useState<string | null>(null);
  const [mcpJsonDraft, setMcpJsonDraft] = React.useState('');
  const [mcpJsonError, setMcpJsonError] = React.useState<string | null>(null);
  const [skillsResult, setSkillsResult] = React.useState<SkillScanResult | null>(null);
  const [skillsError, setSkillsError] = React.useState<string | null>(null);

  React.useEffect(() => {
    setLocalConfig(config);
    setTab('model');
    setShowApiKey(false);
    setSaveError(null);
    setModelFeedback(null);
    setMcpResult(null);
    setMcpError(null);
    setMcpJsonDraft(JSON.stringify(config.mcp, null, 2));
    setMcpJsonError(null);
    setSkillsResult(null);
    setSkillsError(null);
  }, [config, isOpen]);

  const preset = getProviderPreset(localConfig.provider);
  const hasStreaming = Boolean(streamingTaskId);
  const disableSave = isSaving || isTestingModel || isTestingMcp || isScanningSkills || hasStreaming;

  const updateServer = (index: number, updater: (server: McpServerConfig) => McpServerConfig) => {
    setLocalConfig((prev) => {
      const servers = [...prev.mcp.servers];
      const target = servers[index];
      if (!target) return prev;
      servers[index] = updater(target);
      return { ...prev, mcp: { ...prev.mcp, servers } };
    });
  };

  const runModelTest = async () => {
    if (hasStreaming) {
      setModelFeedback({ type: 'error', message: '当前有进行中的会话，请先停止生成再测试连接。' });
      return;
    }
    const normalized = normalizeConfigForSave(localConfig);
    if (!normalized.model) {
      setModelFeedback({ type: 'error', message: '模型名称不能为空。' });
      return;
    }
    setIsTestingModel(true);
    setModelFeedback(null);
    try {
      const result = await TauriAPI.testRuntimeConfig(normalized);
      setModelFeedback({
        type: result.success ? 'success' : 'error',
        message: `${result.message}（${result.latencyMs} ms）`,
      });
    } catch (error) {
      setModelFeedback({ type: 'error', message: getErrorMessage(error) });
    } finally {
      setIsTestingModel(false);
    }
  };

  const runMcpTest = async () => {
    if (hasStreaming) {
      setMcpError('当前有进行中的会话，请先停止生成再测试 MCP。');
      return;
    }
    setIsTestingMcp(true);
    setMcpError(null);
    setMcpResult(null);
    try {
      setMcpResult(await TauriAPI.testMcpConfig(normalizeConfigForSave(localConfig)));
    } catch (error) {
      setMcpError(getErrorMessage(error));
    } finally {
      setIsTestingMcp(false);
    }
  };

  const exportMcpJsonFromForm = () => {
    setMcpJsonDraft(JSON.stringify(normalizeConfigForSave(localConfig).mcp, null, 2));
    setMcpJsonError(null);
  };

  const applyMcpJsonToForm = () => {
    try {
      const parsed = JSON.parse(mcpJsonDraft) as unknown;
      const source =
        parsed && typeof parsed === 'object' && !Array.isArray(parsed) && 'mcp' in parsed
          ? (parsed as Record<string, unknown>).mcp
          : parsed;

      if (!source || typeof source !== 'object' || Array.isArray(source)) {
        throw new Error('JSON 根对象必须是 MCP 配置对象，或包含 mcp 字段。');
      }

      const sourceObject = source as Record<string, unknown>;
      const isFullRuntime =
        'enabled' in sourceObject ||
        'defaultTimeoutSecs' in sourceObject ||
        'maxRetries' in sourceObject ||
        'servers' in sourceObject;

      if (isFullRuntime) {
        const normalizedMcp = normalizeMcpRuntimeConfig(source);
        setLocalConfig((prev) => ({
          ...prev,
          mcp: normalizedMcp,
        }));
      } else {
        let importedServers: McpServerConfig[] = [];

        if (isLegacyServerShape(sourceObject)) {
          const name =
            normalizeText(typeof sourceObject.name === 'string' ? sourceObject.name : '') ??
            `imported-${Date.now()}`;
          importedServers = [toServerFromLegacyShape(name, sourceObject)];
        } else {
          importedServers = Object.entries(sourceObject)
            .filter(([, raw]) => isLegacyServerShape(raw))
            .map(([name, raw]) =>
              toServerFromLegacyShape(name, raw as Record<string, unknown>)
            );
        }

        if (importedServers.length === 0) {
          throw new Error(
            '未识别到 MCP server 定义。可粘贴 {\"server-name\": {...}} 或完整 mcp 对象。'
          );
        }

        setLocalConfig((prev) => {
          const current = prev.mcp.servers;
          const next = [...current];
          importedServers.forEach((server) => {
            const normalizedName = server.name.trim().toLowerCase();
            if (!normalizedName) {
              return;
            }
            const existingIndex = next.findIndex(
              (item) => item.name.trim().toLowerCase() === normalizedName
            );
            if (existingIndex >= 0) {
              next[existingIndex] = server;
            } else {
              next.push(server);
            }
          });

          return {
            ...prev,
            mcp: {
              ...prev.mcp,
              servers: next,
            },
          };
        });
      }

      setMcpJsonError(null);
      setMcpError(null);
      setMcpResult(null);
    } catch (error) {
      setMcpJsonError(getErrorMessage(error));
    }
  };

  const runSkillsScan = async () => {
    if (hasStreaming) {
      setSkillsError('当前有进行中的会话，请先停止生成再扫描 Skills。');
      return;
    }
    setIsScanningSkills(true);
    setSkillsError(null);
    setSkillsResult(null);
    try {
      setSkillsResult(await TauriAPI.scanSkillsConfig(normalizeConfigForSave(localConfig)));
    } catch (error) {
      setSkillsError(getErrorMessage(error));
    } finally {
      setIsScanningSkills(false);
    }
  };

  const save = async () => {
    if (hasStreaming) {
      setSaveError('当前有进行中的会话，请先停止生成再保存设置。');
      return;
    }
    const normalized = normalizeConfigForSave(localConfig);
    if (!normalized.model) {
      setSaveError('模型名称不能为空。');
      setTab('model');
      return;
    }
    setIsSaving(true);
    setSaveError(null);
    try {
      await TauriAPI.updateRuntimeConfig(normalized);
      updateConfig(normalized);
      onClose();
    } catch (error) {
      setSaveError(getErrorMessage(error));
    } finally {
      setIsSaving(false);
    }
  };

  if (!isOpen) return null;

  return (
    <div className="modal-overlay" onClick={onClose}>
      <div className="settings-modal" onClick={(event) => event.stopPropagation()}>
        <div className="modal-header">
          <h2 className="modal-title">助手设置</h2>
          <button type="button" onClick={onClose} className="modal-close" aria-label="关闭设置">
            <X size={18} />
          </button>
        </div>

        <div className="settings-tabs">
          <button type="button" className={`settings-tab ${tab === 'model' ? 'active' : ''}`} onClick={() => setTab('model')}>
            <Bot size={16} />模型
          </button>
          <button type="button" className={`settings-tab ${tab === 'mcp' ? 'active' : ''}`} onClick={() => setTab('mcp')}>
            <Blocks size={16} />MCP
          </button>
          <button type="button" className={`settings-tab ${tab === 'skills' ? 'active' : ''}`} onClick={() => setTab('skills')}>
            <Sparkles size={16} />Skills
          </button>
        </div>

        <div className="modal-body">
          {tab === 'model' && (
            <>
              <section className="field-group">
                <label className="field-label" htmlFor="provider-select">模型提供商</label>
                <select
                  id="provider-select"
                  value={localConfig.provider}
                  onChange={(event) => setLocalConfig((prev) => applyProviderDefaults(prev, event.target.value as AgentProvider))}
                  className="field-select"
                >
                  {PROVIDER_PRESETS.map((item) => (
                    <option key={item.value} value={item.value}>{item.label}</option>
                  ))}
                </select>
                <p className="field-help">{preset.description}</p>
              </section>

              <section className="field-group">
                <label className="field-label" htmlFor="model-input">模型名称</label>
                <input id="model-input" className="field-input" value={localConfig.model} onChange={(event) => setLocalConfig((prev) => ({ ...prev, model: event.target.value }))} />
                <p className="field-help">{preset.modelHint}</p>
              </section>

              <section className="field-group">
                <label className="field-label" htmlFor="api-key-env">API Key 环境变量</label>
                <input id="api-key-env" className="field-input" value={localConfig.apiKeyEnv} onChange={(event) => setLocalConfig((prev) => ({ ...prev, apiKeyEnv: event.target.value }))} />
              </section>

              {preset.supportsApiKey && (
                <section className="field-group">
                  <label className="field-label" htmlFor="provider-api-key">{preset.apiKeyLabel ?? 'API Key'}</label>
                  <div className="password-wrap">
                    <input
                      id="provider-api-key"
                      type={showApiKey ? 'text' : 'password'}
                      value={localConfig.apiKey || ''}
                      onChange={(event) => setLocalConfig((prev) => ({ ...prev, apiKey: event.target.value }))}
                      className="field-input"
                    />
                    <button type="button" className="toggle-visibility" onClick={() => setShowApiKey((value) => !value)} aria-label="切换密钥显示">
                      {showApiKey ? <EyeOff size={16} /> : <Eye size={16} />}
                    </button>
                  </div>
                </section>
              )}

              {preset.supportsBaseUrl && (
                <section className="field-group">
                  <label className="field-label" htmlFor="base-url">{preset.baseUrlLabel ?? 'Base URL'}</label>
                  <input id="base-url" className="field-input" value={localConfig.baseUrl || ''} onChange={(event) => setLocalConfig((prev) => ({ ...prev, baseUrl: event.target.value }))} />
                </section>
              )}

              {preset.supportsMaxTokens && (
                <section className="field-group">
                  <label className="field-label" htmlFor="max-tokens">最大输出 Token</label>
                  <input
                    id="max-tokens"
                    className="field-input"
                    type="number"
                    min={1}
                    value={localConfig.maxTokens ?? ''}
                    onChange={(event) => {
                      const raw = event.target.value.trim();
                      setLocalConfig((prev) => ({ ...prev, maxTokens: raw ? Number.parseInt(raw, 10) : undefined }));
                    }}
                  />
                </section>
              )}

              <section className="field-group">
                <label className="field-label" htmlFor="system-prompt">系统提示词（System Prompt）</label>
                <textarea
                  id="system-prompt"
                  className="field-textarea"
                  value={localConfig.systemPrompt}
                  onChange={(event) => setLocalConfig((prev) => ({ ...prev, systemPrompt: event.target.value }))}
                />
              </section>

              <section className="field-group">
                <div className="settings-inline-head">
                  <p className="settings-inline-title">连通性测试</p>
                  <button type="button" className="btn-ghost" onClick={runModelTest} disabled={isTestingModel || hasStreaming}>
                    <PlugZap size={16} />{isTestingModel ? '测试中...' : '测试模型连接'}
                  </button>
                </div>
                {modelFeedback && <p className={`field-status ${modelFeedback.type}`}>{modelFeedback.message}</p>}
              </section>
            </>
          )}

          {tab === 'mcp' && (
            <>
              <section className="field-group">
                <label className="switch-row">
                  <span className="field-label">启用 MCP</span>
                  <input type="checkbox" checked={localConfig.mcp.enabled} onChange={(event) => setLocalConfig((prev) => ({ ...prev, mcp: { ...prev.mcp, enabled: event.target.checked } }))} />
                </label>
                <div className="settings-grid-2">
                  <div>
                    <label className="field-label" htmlFor="mcp-timeout">默认超时（秒）</label>
                    <input
                      id="mcp-timeout"
                      className="field-input"
                      type="number"
                      min={1}
                      value={localConfig.mcp.defaultTimeoutSecs ?? ''}
                      onChange={(event) => {
                        const raw = event.target.value.trim();
                        setLocalConfig((prev) => ({ ...prev, mcp: { ...prev.mcp, defaultTimeoutSecs: raw ? Number.parseInt(raw, 10) : undefined } }));
                      }}
                    />
                  </div>
                  <div>
                    <label className="field-label" htmlFor="mcp-retries">最大重试次数</label>
                    <input
                      id="mcp-retries"
                      className="field-input"
                      type="number"
                      min={1}
                      value={localConfig.mcp.maxRetries ?? ''}
                      onChange={(event) => {
                        const raw = event.target.value.trim();
                        setLocalConfig((prev) => ({ ...prev, mcp: { ...prev.mcp, maxRetries: raw ? Number.parseInt(raw, 10) : undefined } }));
                      }}
                    />
                  </div>
                </div>
                <p className="field-help">敏感信息只保存环境变量名，运行时从系统环境读取。</p>
              </section>

              <section className="field-group">
                <div className="settings-inline-head">
                  <p className="settings-inline-title">MCP JSON 配置（高级）</p>
                  <div className="settings-button-row">
                    <button type="button" className="btn-ghost" onClick={exportMcpJsonFromForm}>
                      从表单生成
                    </button>
                    <button type="button" className="btn-ghost" onClick={applyMcpJsonToForm}>
                      应用 JSON
                    </button>
                  </div>
                </div>
                <textarea
                  className="field-textarea json-config-textarea"
                  value={mcpJsonDraft}
                  onChange={(event) => setMcpJsonDraft(event.target.value)}
                  placeholder={'支持：{"zai-mcp-server":{"type":"stdio","command":"npx","args":["-y","@z_ai/mcp-server"],"env":{"Z_AI_API_KEY":"***"}}}'}
                />
                <p className="field-help">支持单个/多个 server map，导入时会追加到现有列表（同名覆盖）。</p>
                {mcpJsonError && <p className="field-error">{mcpJsonError}</p>}
              </section>

              <section className="field-group">
                <div className="settings-inline-head">
                  <p className="settings-inline-title">MCP Servers</p>
                  <button type="button" className="btn-ghost" onClick={() => setLocalConfig((prev) => ({ ...prev, mcp: { ...prev.mcp, servers: [...prev.mcp.servers, createDefaultMcpServer()] } }))}>
                    <Plus size={16} />新增 Server
                  </button>
                </div>

                <div className="settings-stack">
                  {localConfig.mcp.servers.map((server, index) => {
                    const auth = server.auth ?? { type: McpAuthType.None };
                    return (
                      <article key={`server-${index}`} className="mcp-server-card">
                        <div className="mcp-server-head">
                          <strong className="mcp-server-title">{server.name.trim() || `Server ${index + 1}`}</strong>
                          <div className="mcp-server-actions">
                            <label className="switch-row compact">
                              <span>启用</span>
                              <input type="checkbox" checked={server.enabled} onChange={(event) => updateServer(index, (item) => ({ ...item, enabled: event.target.checked }))} />
                            </label>
                            <button type="button" className="btn-ghost danger" onClick={() => setLocalConfig((prev) => ({ ...prev, mcp: { ...prev.mcp, servers: prev.mcp.servers.filter((_, i) => i !== index) } }))}>
                              <Trash2 size={14} />删除
                            </button>
                          </div>
                        </div>

                        <div className="settings-grid-2">
                          <div>
                            <label className="field-label" htmlFor={`name-${index}`}>名称</label>
                            <input id={`name-${index}`} className="field-input" value={server.name} onChange={(event) => updateServer(index, (item) => ({ ...item, name: event.target.value }))} />
                          </div>
                          <div>
                            <label className="field-label" htmlFor={`transport-${index}`}>Transport</label>
                            <select id={`transport-${index}`} className="field-select" value={server.transport} onChange={(event) => updateServer(index, (item) => ({ ...item, transport: event.target.value as McpTransportKind }))}>
                              {TRANSPORT_OPTIONS.map((option) => <option key={option.value} value={option.value}>{option.label}</option>)}
                            </select>
                          </div>
                        </div>

                        <div className="settings-grid-2">
                          <div>
                            <label className="field-label" htmlFor={`endpoint-${index}`}>Endpoint</label>
                            <input id={`endpoint-${index}`} className="field-input" value={server.endpoint ?? ''} onChange={(event) => updateServer(index, (item) => ({ ...item, endpoint: event.target.value }))} />
                          </div>
                          <div>
                            <label className="field-label" htmlFor={`timeout-${index}`}>超时（秒）</label>
                            <input
                              id={`timeout-${index}`}
                              className="field-input"
                              type="number"
                              min={1}
                              value={server.timeoutSecs ?? ''}
                              onChange={(event) => {
                                const raw = event.target.value.trim();
                                updateServer(index, (item) => ({ ...item, timeoutSecs: raw ? Number.parseInt(raw, 10) : undefined }));
                              }}
                            />
                          </div>
                        </div>

                        {server.transport === McpTransportKind.Stdio && (
                          <>
                            <label className="field-label" htmlFor={`command-${index}`}>Command</label>
                            <input id={`command-${index}`} className="field-input" value={server.command ?? ''} onChange={(event) => updateServer(index, (item) => ({ ...item, command: event.target.value }))} />

                            <label className="field-label" htmlFor={`args-${index}`}>Args（每行一个）</label>
                            <textarea id={`args-${index}`} className="field-textarea compact" value={listToText(server.args)} onChange={(event) => updateServer(index, (item) => ({ ...item, args: parseLineList(event.target.value) }))} />
                          </>
                        )}

                        <div className="settings-grid-2">
                          <div>
                            <label className="field-label" htmlFor={`env-${index}`}>环境变量（每行 KEY=VALUE）</label>
                            <textarea id={`env-${index}`} className="field-textarea compact" value={mapToText(server.env)} onChange={(event) => updateServer(index, (item) => ({ ...item, env: textToMap(event.target.value) }))} />
                          </div>
                          <div>
                            <label className="field-label" htmlFor={`headers-${index}`}>Headers（每行 KEY=VALUE）</label>
                            <textarea id={`headers-${index}`} className="field-textarea compact" value={mapToText(server.headers)} onChange={(event) => updateServer(index, (item) => ({ ...item, headers: textToMap(event.target.value) }))} />
                          </div>
                        </div>

                        <label className="field-label" htmlFor={`auth-type-${index}`}>认证方式</label>
                        <select id={`auth-type-${index}`} className="field-select" value={auth.type} onChange={(event) => updateServer(index, (item) => ({ ...item, auth: { ...(item.auth ?? { type: McpAuthType.None }), type: event.target.value as McpAuthType } }))}>
                          {AUTH_OPTIONS.map((option) => <option key={option.value} value={option.value}>{option.label}</option>)}
                        </select>

                        <div className="settings-grid-2">
                          <div>
                            <label className="field-label" htmlFor={`token-env-${index}`}>tokenEnv</label>
                            <input id={`token-env-${index}`} className="field-input" value={auth.tokenEnv ?? ''} onChange={(event) => updateServer(index, (item) => ({ ...item, auth: { ...(item.auth ?? { type: auth.type }), ...auth, tokenEnv: event.target.value } }))} />
                          </div>
                          <div>
                            <label className="field-label" htmlFor={`api-key-env-${index}`}>apiKeyEnv</label>
                            <input id={`api-key-env-${index}`} className="field-input" value={auth.apiKeyEnv ?? ''} onChange={(event) => updateServer(index, (item) => ({ ...item, auth: { ...(item.auth ?? { type: auth.type }), ...auth, apiKeyEnv: event.target.value } }))} />
                          </div>
                          <div>
                            <label className="field-label" htmlFor={`username-env-${index}`}>usernameEnv</label>
                            <input id={`username-env-${index}`} className="field-input" value={auth.usernameEnv ?? ''} onChange={(event) => updateServer(index, (item) => ({ ...item, auth: { ...(item.auth ?? { type: auth.type }), ...auth, usernameEnv: event.target.value } }))} />
                          </div>
                          <div>
                            <label className="field-label" htmlFor={`password-env-${index}`}>passwordEnv</label>
                            <input id={`password-env-${index}`} className="field-input" value={auth.passwordEnv ?? ''} onChange={(event) => updateServer(index, (item) => ({ ...item, auth: { ...(item.auth ?? { type: auth.type }), ...auth, passwordEnv: event.target.value } }))} />
                          </div>
                          <div>
                            <label className="field-label" htmlFor={`client-id-env-${index}`}>clientIdEnv</label>
                            <input id={`client-id-env-${index}`} className="field-input" value={auth.clientIdEnv ?? ''} onChange={(event) => updateServer(index, (item) => ({ ...item, auth: { ...(item.auth ?? { type: auth.type }), ...auth, clientIdEnv: event.target.value } }))} />
                          </div>
                          <div>
                            <label className="field-label" htmlFor={`client-secret-env-${index}`}>clientSecretEnv</label>
                            <input id={`client-secret-env-${index}`} className="field-input" value={auth.clientSecretEnv ?? ''} onChange={(event) => updateServer(index, (item) => ({ ...item, auth: { ...(item.auth ?? { type: auth.type }), ...auth, clientSecretEnv: event.target.value } }))} />
                          </div>
                          <div>
                            <label className="field-label" htmlFor={`token-url-${index}`}>tokenUrl</label>
                            <input id={`token-url-${index}`} className="field-input" value={auth.tokenUrl ?? ''} onChange={(event) => updateServer(index, (item) => ({ ...item, auth: { ...(item.auth ?? { type: auth.type }), ...auth, tokenUrl: event.target.value } }))} />
                          </div>
                          <div>
                            <label className="field-label" htmlFor={`api-key-header-${index}`}>apiKeyHeader</label>
                            <input id={`api-key-header-${index}`} className="field-input" value={auth.apiKeyHeader ?? ''} onChange={(event) => updateServer(index, (item) => ({ ...item, auth: { ...(item.auth ?? { type: auth.type }), ...auth, apiKeyHeader: event.target.value } }))} />
                          </div>
                        </div>
                      </article>
                    );
                  })}
                </div>

                <div className="settings-inline-head">
                  <p className="settings-inline-title">MCP 配置测试</p>
                  <button type="button" className="btn-ghost" onClick={runMcpTest} disabled={isTestingMcp || hasStreaming}>
                    <PlugZap size={16} />{isTestingMcp ? '测试中...' : '测试 MCP'}
                  </button>
                </div>

                {mcpError && <p className="field-error">{mcpError}</p>}
                {mcpResult && (
                  <div className="result-list">
                    <p className={`field-status ${mcpResult.success ? 'success' : 'error'}`}>{mcpResult.message}</p>
                    {mcpResult.serverResults.map((item) => (
                      <div key={`${item.name}-${item.enabled}-${item.success}`} className={`result-item ${item.success ? 'success' : 'error'}`}>
                        <p className="result-title">{item.name || '未命名 Server'} · {item.latencyMs} ms</p>
                        <p className="result-sub">{item.enabled ? '已启用' : '未启用'} · {item.toolCount} tools</p>
                        {item.tools.length > 0 && <p className="result-sub">{item.tools.join(', ')}</p>}
                        {item.error && <p className="field-error">{item.error}</p>}
                      </div>
                    ))}
                  </div>
                )}
              </section>
            </>
          )}
          {tab === 'skills' && (
            <>
              <section className="field-group">
                <label className="switch-row">
                  <span className="field-label">启用 Skills</span>
                  <input type="checkbox" checked={localConfig.skills.enabled} onChange={(event) => setLocalConfig((prev) => ({ ...prev, skills: { ...prev.skills, enabled: event.target.checked } }))} />
                </label>

                <label className="field-label" htmlFor="personal-dir">personalDir</label>
                <input id="personal-dir" className="field-input" value={localConfig.skills.personalDir ?? ''} onChange={(event) => setLocalConfig((prev) => ({ ...prev, skills: { ...prev.skills, personalDir: event.target.value } }))} />

                <label className="field-label" htmlFor="project-dirs">projectDirs（每行一个目录）</label>
                <textarea
                  id="project-dirs"
                  className="field-textarea compact"
                  value={listToText(localConfig.skills.projectDirs)}
                  onChange={(event) => setLocalConfig((prev) => ({ ...prev, skills: { ...prev.skills, projectDirs: parseLineList(event.target.value) } }))}
                />

                <label className="switch-row compact">
                  <span>autoApply</span>
                  <input type="checkbox" checked={localConfig.skills.autoApply} onChange={(event) => setLocalConfig((prev) => ({ ...prev, skills: { ...prev.skills, autoApply: event.target.checked } }))} />
                </label>

                <div className="settings-inline-head">
                  <p className="settings-inline-title">Skills 扫描预览</p>
                  <button type="button" className="btn-ghost" onClick={runSkillsScan} disabled={isScanningSkills || hasStreaming}>
                    <PlugZap size={16} />{isScanningSkills ? '扫描中...' : '扫描 Skills'}
                  </button>
                </div>

                {skillsError && <p className="field-error">{skillsError}</p>}
                {skillsResult && (
                  <div className="result-list">
                    <p className={`field-status ${skillsResult.success ? 'success' : 'error'}`}>{skillsResult.message}</p>
                    {skillsResult.warnings.map((warning) => <p key={warning} className="field-error">{warning}</p>)}
                    {skillsResult.skills.map((item) => (
                      <div key={`${item.source}-${item.path}`} className="result-item success">
                        <p className="result-title">{item.name} · {item.source}</p>
                        <p className="result-sub">{item.path}</p>
                        <p className="result-sub">{item.description}</p>
                      </div>
                    ))}
                  </div>
                )}
              </section>
            </>
          )}

          {saveError && <p className="field-error">{saveError}</p>}
        </div>

        <div className="modal-footer">
          <button type="button" onClick={onClose} className="btn-ghost" disabled={disableSave}>取消</button>
          <button type="button" onClick={save} className="btn-primary" disabled={disableSave}>
            <Save size={16} />{isSaving ? '保存中...' : '保存设置'}
          </button>
        </div>
      </div>
    </div>
  );
};

export default SettingsModal;

