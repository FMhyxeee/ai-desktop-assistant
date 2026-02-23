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
  type AppConfig,
  type GovernanceReport,
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

const normalizeJsonObject = (value: unknown): Record<string, unknown> => {
  if (!value || typeof value !== 'object' || Array.isArray(value)) {
    return {};
  }
  return value as Record<string, unknown>;
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

const isRecord = (value: unknown): value is Record<string, unknown> =>
  Boolean(value) && typeof value === 'object' && !Array.isArray(value);

const isLegacyServerShape = (value: unknown): value is Record<string, unknown> => {
  if (!isRecord(value)) return false;
  return (
    'type' in value ||
    'transport' in value ||
    'command' in value ||
    'args' in value ||
    'env' in value
  );
};

const toServerFromLegacyShape = (name: string, value: Record<string, unknown>): McpServerConfig => {
  return {
    name: name.trim(),
    enabled: normalizeBooleanFromUnknown(value.enabled, true),
    command: normalizeText(typeof value.command === 'string' ? value.command : '') ?? '',
    args: normalizeStringArrayFromUnknown(value.args),
    env: normalizeStringMapFromUnknown(value.env),
  };
};

const normalizeConfigForSave = (config: AppConfig): AppConfig => ({
  ...config,
  model: config.model.trim(),
  modelSupportsImageInput: Boolean(config.modelSupportsImageInput),
  apiKeyEnv: config.apiKeyEnv.trim(),
  apiKey: (config.apiKey ?? '').trim(),
  baseUrl: (config.baseUrl ?? '').trim(),
  maxTokens: normalizePositiveInt(config.maxTokens),
  workspace: {
    rootDir: (normalizeText(config.workspace.rootDir) ?? ''),
  },
  mcp: {
    ...config.mcp,
    defaultTimeoutSecs: normalizePositiveInt(config.mcp.defaultTimeoutSecs),
    maxRetries: normalizePositiveInt(config.mcp.maxRetries),
    servers: config.mcp.servers.map((server) => ({
      name: server.name.trim(),
      enabled: Boolean(server.enabled),
      command: normalizeText(server.command) ?? '',
      args: server.args.map((item) => item.trim()).filter((item) => item.length > 0),
      env: textToMap(mapToText(server.env)),
    })),
    imageRecognition: {
      enabled: Boolean(config.mcp.imageRecognition.enabled),
      serverName: config.mcp.imageRecognition.serverName.trim(),
      toolName: config.mcp.imageRecognition.toolName.trim(),
      argsTemplate: normalizeJsonObject(config.mcp.imageRecognition.argsTemplate),
    },
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
  const [isRunningGovernance, setIsRunningGovernance] = React.useState(false);

  const [saveError, setSaveError] = React.useState<string | null>(null);
  const [modelFeedback, setModelFeedback] = React.useState<Feedback | null>(null);
  const [mcpResult, setMcpResult] = React.useState<McpConfigTestResult | null>(null);
  const [mcpError, setMcpError] = React.useState<string | null>(null);
  const [mcpJsonDraft, setMcpJsonDraft] = React.useState('');
  const [mcpJsonError, setMcpJsonError] = React.useState<string | null>(null);
  const [imageRecognitionArgsTemplateDraft, setImageRecognitionArgsTemplateDraft] = React.useState('');
  const [imageRecognitionArgsTemplateError, setImageRecognitionArgsTemplateError] = React.useState<string | null>(null);
  const [skillsResult, setSkillsResult] = React.useState<SkillScanResult | null>(null);
  const [skillsError, setSkillsError] = React.useState<string | null>(null);
  const [governanceReport, setGovernanceReport] = React.useState<GovernanceReport | null>(null);
  const [governanceError, setGovernanceError] = React.useState<string | null>(null);

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
    setImageRecognitionArgsTemplateDraft(
      JSON.stringify(config.mcp.imageRecognition.argsTemplate, null, 2)
    );
    setImageRecognitionArgsTemplateError(null);
    setSkillsResult(null);
    setSkillsError(null);
    setGovernanceReport(null);
    setGovernanceError(null);
  }, [config, isOpen]);

  const preset = getProviderPreset(localConfig.provider);
  const hasStreaming = Boolean(streamingTaskId);
  const disableSave =
    isSaving || isTestingModel || isTestingMcp || isScanningSkills || isRunningGovernance || hasStreaming;

  const updateServer = (index: number, updater: (server: McpServerConfig) => McpServerConfig) => {
    setLocalConfig((prev) => {
      const servers = [...prev.mcp.servers];
      const target = servers[index];
      if (!target) return prev;
      servers[index] = updater(target);
      return { ...prev, mcp: { ...prev.mcp, servers } };
    });
  };

  const normalizeConfigWithImageRecognitionTemplate = React.useCallback(
    (sourceConfig: AppConfig): AppConfig => {
      let parsedTemplate: unknown;
      try {
        parsedTemplate = JSON.parse(imageRecognitionArgsTemplateDraft);
      } catch {
        throw new Error('图像识别 Args Template 必须是合法 JSON 对象。');
      }

      if (!parsedTemplate || typeof parsedTemplate !== 'object' || Array.isArray(parsedTemplate)) {
        throw new Error('图像识别 Args Template 必须是 JSON 对象。');
      }

      setImageRecognitionArgsTemplateError(null);
      return {
        ...sourceConfig,
        mcp: {
          ...sourceConfig.mcp,
          imageRecognition: {
            ...sourceConfig.mcp.imageRecognition,
            argsTemplate: parsedTemplate as Record<string, unknown>,
          },
        },
      };
    },
    [imageRecognitionArgsTemplateDraft]
  );

  const runModelTest = async () => {
    if (hasStreaming) {
      setModelFeedback({ type: 'error', message: '当前有进行中的会话，请先停止生成再测试连接。' });
      return;
    }
    let normalized: AppConfig;
    try {
      normalized = normalizeConfigForSave(
        normalizeConfigWithImageRecognitionTemplate(localConfig)
      );
    } catch (error) {
      setImageRecognitionArgsTemplateError(getErrorMessage(error));
      setModelFeedback({ type: 'error', message: getErrorMessage(error) });
      return;
    }
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
      const normalized = normalizeConfigForSave(
        normalizeConfigWithImageRecognitionTemplate(localConfig)
      );
      setMcpResult(await TauriAPI.testMcpConfig(normalized));
    } catch (error) {
      const message = getErrorMessage(error);
      setImageRecognitionArgsTemplateError(message);
      setMcpError(message);
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
        setImageRecognitionArgsTemplateDraft(
          JSON.stringify(normalizedMcp.imageRecognition.argsTemplate, null, 2)
        );
        setImageRecognitionArgsTemplateError(null);
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
            '未识别到 MCP server 定义。可粘贴 {"server-name": {...}} 或完整 mcp 对象。'
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
      const normalized = normalizeConfigForSave(
        normalizeConfigWithImageRecognitionTemplate(localConfig)
      );
      setSkillsResult(await TauriAPI.scanSkillsConfig(normalized));
    } catch (error) {
      const message = getErrorMessage(error);
      setImageRecognitionArgsTemplateError(message);
      setSkillsError(message);
    } finally {
      setIsScanningSkills(false);
    }
  };

  const runGovernanceScan = async () => {
    if (hasStreaming) {
      setGovernanceError('当前有进行中的会话，请先停止生成再运行治理扫描。');
      return;
    }
    setIsRunningGovernance(true);
    setGovernanceError(null);
    try {
      const normalized = normalizeConfigForSave(
        normalizeConfigWithImageRecognitionTemplate(localConfig)
      );
      const report = await TauriAPI.runGovernanceScan(normalized);
      setGovernanceReport(report);
    } catch (error) {
      setGovernanceError(getErrorMessage(error));
    } finally {
      setIsRunningGovernance(false);
    }
  };

  const save = async () => {
    if (hasStreaming) {
      setSaveError('当前有进行中的会话，请先停止生成再保存设置。');
      return;
    }
    let normalized: AppConfig;
    try {
      normalized = normalizeConfigForSave(
        normalizeConfigWithImageRecognitionTemplate(localConfig)
      );
    } catch (error) {
      const message = getErrorMessage(error);
      setImageRecognitionArgsTemplateError(message);
      setSaveError(message);
      setTab('mcp');
      return;
    }
    if (!normalized.model) {
      setSaveError('模型名称不能为空。');
      setTab('model');
      return;
    }
    setIsSaving(true);
    setSaveError(null);
    try {
      const updateReport = await TauriAPI.updateRuntimeConfig(normalized);
      setGovernanceReport(updateReport.after);
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
                <label className="switch-row">
                  <span className="field-label">当前模型支持图像输入</span>
                  <input
                    type="checkbox"
                    checked={localConfig.modelSupportsImageInput}
                    onChange={(event) =>
                      setLocalConfig((prev) => ({
                        ...prev,
                        modelSupportsImageInput: event.target.checked,
                      }))
                    }
                  />
                </label>
                <p className="field-help">开启后图片会直接发送给模型；关闭后可走 MCP 图像识别兜底。</p>
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
                <p className="field-help">MCP Server 配置已简化为 command、args、env 三项核心字段。</p>
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
                  placeholder={'支持：{"zai-mcp-server":{"command":"npx","args":["-y","@z_ai/mcp-server"],"env":{"Z_AI_API_KEY":"***"}}}'}
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
                            <label className="field-label" htmlFor={`command-${index}`}>Command</label>
                            <input id={`command-${index}`} className="field-input" value={server.command ?? ''} onChange={(event) => updateServer(index, (item) => ({ ...item, command: event.target.value }))} />
                          </div>
                        </div>

                        <label className="field-label" htmlFor={`args-${index}`}>Args（每行一个）</label>
                        <textarea id={`args-${index}`} className="field-textarea compact" value={listToText(server.args)} onChange={(event) => updateServer(index, (item) => ({ ...item, args: parseLineList(event.target.value) }))} />

                        <label className="field-label" htmlFor={`env-${index}`}>环境变量（每行 KEY=VALUE）</label>
                        <textarea id={`env-${index}`} className="field-textarea compact" value={mapToText(server.env)} onChange={(event) => updateServer(index, (item) => ({ ...item, env: textToMap(event.target.value) }))} />
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

              <section className="field-group">
                <div className="settings-inline-head">
                  <p className="settings-inline-title">图像识别兜底</p>
                </div>
                <label className="switch-row compact">
                  <span>启用 imageRecognition</span>
                  <input
                    type="checkbox"
                    checked={localConfig.mcp.imageRecognition.enabled}
                    onChange={(event) =>
                      setLocalConfig((prev) => ({
                        ...prev,
                        mcp: {
                          ...prev.mcp,
                          imageRecognition: {
                            ...prev.mcp.imageRecognition,
                            enabled: event.target.checked,
                          },
                        },
                      }))
                    }
                  />
                </label>

                <div className="settings-grid-2">
                  <div>
                    <label className="field-label" htmlFor="image-recognition-server">
                      serverName
                    </label>
                    <input
                      id="image-recognition-server"
                      className="field-input"
                      value={localConfig.mcp.imageRecognition.serverName}
                      onChange={(event) =>
                        setLocalConfig((prev) => ({
                          ...prev,
                          mcp: {
                            ...prev.mcp,
                            imageRecognition: {
                              ...prev.mcp.imageRecognition,
                              serverName: event.target.value,
                            },
                          },
                        }))
                      }
                    />
                  </div>
                  <div>
                    <label className="field-label" htmlFor="image-recognition-tool">
                      toolName
                    </label>
                    <input
                      id="image-recognition-tool"
                      className="field-input"
                      value={localConfig.mcp.imageRecognition.toolName}
                      onChange={(event) =>
                        setLocalConfig((prev) => ({
                          ...prev,
                          mcp: {
                            ...prev.mcp,
                            imageRecognition: {
                              ...prev.mcp.imageRecognition,
                              toolName: event.target.value,
                            },
                          },
                        }))
                      }
                    />
                  </div>
                </div>

                <label className="field-label" htmlFor="image-recognition-args-template">
                  argsTemplate (JSON)
                </label>
                <textarea
                  id="image-recognition-args-template"
                  className="field-textarea compact"
                  value={imageRecognitionArgsTemplateDraft}
                  onChange={(event) => {
                    const next = event.target.value;
                    setImageRecognitionArgsTemplateDraft(next);
                    try {
                      const parsed = JSON.parse(next);
                      if (!parsed || typeof parsed !== 'object' || Array.isArray(parsed)) {
                        throw new Error('图像识别 Args Template 必须是 JSON 对象。');
                      }
                      setImageRecognitionArgsTemplateError(null);
                      setLocalConfig((prev) => ({
                        ...prev,
                        mcp: {
                          ...prev.mcp,
                          imageRecognition: {
                            ...prev.mcp.imageRecognition,
                            argsTemplate: parsed as Record<string, unknown>,
                          },
                        },
                      }));
                    } catch {
                      setImageRecognitionArgsTemplateError(
                        '图像识别 Args Template 必须是合法 JSON 对象。'
                      );
                    }
                  }}
                />
                <p className="field-help">
                  占位符：{'{{data_url}}'} / {'{{base64}}'} / {'{{mime_type}}'} / {'{{name}}'} / {'{{path}}'}。
                </p>
                {imageRecognitionArgsTemplateError && (
                  <p className="field-error">{imageRecognitionArgsTemplateError}</p>
                )}
              </section>

              <section className="field-group">
                <div className="settings-inline-head">
                  <p className="settings-inline-title">治理扫描</p>
                  <button
                    type="button"
                    className="btn-ghost"
                    onClick={runGovernanceScan}
                    disabled={isRunningGovernance || hasStreaming}
                  >
                    <PlugZap size={16} />{isRunningGovernance ? '扫描中...' : '运行治理扫描'}
                  </button>
                </div>

                {governanceError && <p className="field-error">{governanceError}</p>}
                {governanceReport && (
                  <div className="result-list">
                    <p
                      className={`field-status ${
                        governanceReport.blockerCount > 0
                          ? 'error'
                          : governanceReport.warningCount > 0
                            ? 'warning'
                            : 'success'
                      }`}
                    >
                      {governanceReport.summary}
                    </p>
                    <p className="result-sub">
                      blocker={governanceReport.blockerCount} · warning={governanceReport.warningCount} · info={governanceReport.infoCount}
                    </p>
                    {governanceReport.issues.slice(0, 8).map((issue) => (
                      <div
                        key={`${issue.category}-${issue.code}-${issue.message}`}
                        className={`result-item ${
                          issue.severity === 'blocker'
                            ? 'error'
                            : issue.severity === 'warning'
                              ? 'warning'
                              : 'success'
                        }`}
                      >
                        <p className="result-title">
                          [{issue.severity}] {issue.code}
                        </p>
                        <p className="result-sub">{issue.message}</p>
                        {issue.evidence && <p className="result-sub">{issue.evidence}</p>}
                        {issue.suggestion && <p className="result-sub">{issue.suggestion}</p>}
                      </div>
                    ))}
                    {governanceReport.issues.length > 8 && (
                      <p className="result-sub">仅展示前 8 条，可通过协议面板查看完整结构化事件。</p>
                    )}
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

