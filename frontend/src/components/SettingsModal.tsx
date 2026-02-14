import React from 'react';
import { Eye, EyeOff, PlugZap, Save, X } from 'lucide-react';
import { applyProviderDefaults, getProviderPreset, PROVIDER_PRESETS } from '../config/providers';
import { TauriAPI } from '../lib/tauri';
import { useAppStore } from '../store/appStore';
import { AgentProvider, type AppConfig } from '../types';

interface SettingsModalProps {
  isOpen: boolean;
  onClose: () => void;
}

interface TestFeedback {
  type: 'success' | 'error';
  message: string;
}

const getErrorMessage = (error: unknown): string => {
  if (error instanceof Error && error.message) {
    return error.message;
  }
  if (typeof error === 'string' && error.trim()) {
    return error;
  }
  return '保存设置失败，请检查配置后重试。';
};

const normalizeConfigForSave = (config: AppConfig): AppConfig => ({
  ...config,
  model: config.model.trim(),
  apiKeyEnv: config.apiKeyEnv.trim(),
  apiKey: (config.apiKey ?? '').trim(),
  baseUrl: (config.baseUrl ?? '').trim(),
  systemPrompt: config.systemPrompt.trim(),
});

const SettingsModal: React.FC<SettingsModalProps> = ({ isOpen, onClose }) => {
  const { config, updateConfig, streamingTaskId } = useAppStore();
  const [localConfig, setLocalConfig] = React.useState(config);
  const [showApiKey, setShowApiKey] = React.useState(false);
  const [isSaving, setIsSaving] = React.useState(false);
  const [isTesting, setIsTesting] = React.useState(false);
  const [saveError, setSaveError] = React.useState<string | null>(null);
  const [testFeedback, setTestFeedback] = React.useState<TestFeedback | null>(null);

  React.useEffect(() => {
    setLocalConfig(config);
    setShowApiKey(false);
    setSaveError(null);
    setTestFeedback(null);
    setIsTesting(false);
  }, [config, isOpen]);

  const providerPreset = getProviderPreset(localConfig.provider);

  const handleProviderChange = (provider: AgentProvider) => {
    setLocalConfig((prev) => applyProviderDefaults(prev, provider));
    setShowApiKey(false);
    setSaveError(null);
    setTestFeedback(null);
  };

  const handleTestConnection = async () => {
    if (streamingTaskId) {
      setTestFeedback({
        type: 'error',
        message: '当前有进行中的会话，请先停止生成再测试连接。',
      });
      return;
    }

    const normalized = normalizeConfigForSave(localConfig);
    if (!normalized.model) {
      setTestFeedback({ type: 'error', message: '模型名称不能为空。' });
      return;
    }

    setIsTesting(true);
    setSaveError(null);
    setTestFeedback(null);

    try {
      const result = await TauriAPI.testRuntimeConfig(normalized);
      setTestFeedback({
        type: result.success ? 'success' : 'error',
        message: `${result.message}（${result.latencyMs} ms）`,
      });
    } catch (error) {
      setTestFeedback({ type: 'error', message: getErrorMessage(error) });
    } finally {
      setIsTesting(false);
    }
  };

  const handleSave = async () => {
    if (streamingTaskId) {
      setSaveError('当前有进行中的会话，请先停止生成再保存设置。');
      return;
    }

    const normalized = normalizeConfigForSave(localConfig);
    if (!normalized.model) {
      setSaveError('模型名称不能为空。');
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

  if (!isOpen) {
    return null;
  }

  return (
    <div className="modal-overlay" onClick={onClose}>
      <div
        className="settings-modal"
        onClick={(event) => {
          event.stopPropagation();
        }}
      >
        <div className="modal-header">
          <h2 className="modal-title">助手设置</h2>
          <button type="button" onClick={onClose} className="modal-close" aria-label="关闭设置">
            <X size={18} />
          </button>
        </div>

        <div className="modal-body">
          <section className="field-group">
            <label className="field-label" htmlFor="provider-select">
              模型提供商
            </label>
            <select
              id="provider-select"
              value={localConfig.provider}
              onChange={(event) => handleProviderChange(event.target.value as AgentProvider)}
              className="field-select"
            >
              {PROVIDER_PRESETS.map((preset) => (
                <option key={preset.value} value={preset.value}>
                  {preset.label}
                </option>
              ))}
            </select>
            <p className="field-help">{providerPreset.description}</p>
          </section>

          <section className="field-group">
            <label className="field-label" htmlFor="model-input">
              模型名称
            </label>
            <input
              id="model-input"
              type="text"
              value={localConfig.model}
              onChange={(event) => setLocalConfig({ ...localConfig, model: event.target.value })}
              placeholder={providerPreset.defaultModel}
              className="field-input"
            />
            <p className="field-help">{providerPreset.modelHint}</p>
          </section>

          <section className="field-group">
            <label className="field-label" htmlFor="api-key-env-input">
              API Key 环境变量
            </label>
            <input
              id="api-key-env-input"
              type="text"
              value={localConfig.apiKeyEnv}
              onChange={(event) => setLocalConfig({ ...localConfig, apiKeyEnv: event.target.value })}
              placeholder={providerPreset.defaultApiKeyEnv}
              className="field-input"
            />
            <p className="field-help">未填写 API Key 时，后端将从该环境变量读取。</p>
          </section>

          {providerPreset.supportsApiKey && (
            <section className="field-group">
              <label className="field-label" htmlFor="provider-api-key">
                {providerPreset.apiKeyLabel ?? 'API Key'}
              </label>
              <div className="password-wrap">
                <input
                  id="provider-api-key"
                  type={showApiKey ? 'text' : 'password'}
                  value={localConfig.apiKey || ''}
                  onChange={(event) => setLocalConfig({ ...localConfig, apiKey: event.target.value })}
                  placeholder={providerPreset.apiKeyPlaceholder ?? '请输入 API Key'}
                  className="field-input"
                />
                <button
                  type="button"
                  className="toggle-visibility"
                  onClick={() => setShowApiKey((prev) => !prev)}
                  aria-label="切换密钥显示"
                >
                  {showApiKey ? <EyeOff size={16} /> : <Eye size={16} />}
                </button>
              </div>
              <p className="field-help">可留空，留空时将读取上面的环境变量。</p>
            </section>
          )}

          {providerPreset.supportsBaseUrl && (
            <section className="field-group">
              <label className="field-label" htmlFor="base-url-input">
                {providerPreset.baseUrlLabel ?? 'Base URL'}
              </label>
              <input
                id="base-url-input"
                type="text"
                value={localConfig.baseUrl || ''}
                onChange={(event) => setLocalConfig({ ...localConfig, baseUrl: event.target.value })}
                placeholder={providerPreset.baseUrlPlaceholder ?? 'https://example.com/v1'}
                className="field-input"
              />
              <p className="field-help">留空时将使用 provider 的默认地址。</p>
            </section>
          )}

          {providerPreset.supportsMaxTokens && (
            <section className="field-group">
              <label className="field-label" htmlFor="max-tokens-input">
                最大输出 Token
              </label>
              <input
                id="max-tokens-input"
                type="number"
                min={1}
                step={1}
                value={localConfig.maxTokens ?? ''}
                onChange={(event) => {
                  const raw = event.target.value.trim();
                  if (!raw) {
                    setLocalConfig({ ...localConfig, maxTokens: undefined });
                    return;
                  }
                  const parsed = Number.parseInt(raw, 10);
                  setLocalConfig({
                    ...localConfig,
                    maxTokens: Number.isFinite(parsed) && parsed > 0 ? parsed : localConfig.maxTokens,
                  });
                }}
                className="field-input"
              />
              <p className="field-help">仅 Anthropic 生效，留空则使用默认值。</p>
            </section>
          )}

          <section className="field-group">
            <label className="field-label" htmlFor="system-prompt">
              系统提示词（System Prompt）
            </label>
            <textarea
              id="system-prompt"
              value={localConfig.systemPrompt}
              onChange={(event) => setLocalConfig({ ...localConfig, systemPrompt: event.target.value })}
              placeholder="你是一个高效、严谨的桌面编码助手..."
              className="field-textarea"
            />
            <p className="field-help">{providerPreset.promptHint}</p>
          </section>

          {testFeedback && (
            <p className={`field-status ${testFeedback.type === 'success' ? 'success' : 'error'}`}>
              {testFeedback.message}
            </p>
          )}
          {saveError && <p className="field-error">{saveError}</p>}
        </div>

        <div className="modal-footer">
          <button type="button" onClick={onClose} className="btn-ghost" disabled={isSaving || isTesting}>
            取消
          </button>
          <button
            type="button"
            onClick={handleTestConnection}
            className="btn-ghost"
            disabled={isSaving || isTesting || Boolean(streamingTaskId)}
          >
            <PlugZap size={16} />
            {isTesting ? '测试中...' : '测试连接'}
          </button>
          <button
            type="button"
            onClick={handleSave}
            className="btn-primary"
            disabled={isSaving || isTesting || Boolean(streamingTaskId)}
          >
            <Save size={16} />
            {isSaving ? '保存中...' : '保存设置'}
          </button>
        </div>
      </div>
    </div>
  );
};

export default SettingsModal;
