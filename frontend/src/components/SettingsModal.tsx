import React from 'react';
import { Eye, EyeOff, Save, X } from 'lucide-react';
import { useAppStore } from '../store/appStore';
import { AgentProvider } from '../types';

interface SettingsModalProps {
  isOpen: boolean;
  onClose: () => void;
}

const SettingsModal: React.FC<SettingsModalProps> = ({ isOpen, onClose }) => {
  const { config, updateConfig } = useAppStore();
  const [localConfig, setLocalConfig] = React.useState(config);
  const [showApiKey, setShowApiKey] = React.useState(false);

  React.useEffect(() => {
    setLocalConfig(config);
  }, [config, isOpen]);

  const handleSave = () => {
    updateConfig(localConfig);
    onClose();
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
              onChange={(event) =>
                setLocalConfig({
                  ...localConfig,
                  provider: event.target.value as AgentProvider,
                })
              }
              className="field-select"
            >
              <option value={AgentProvider.OpenAi}>OpenAI</option>
              <option value={AgentProvider.Glm}>GLM</option>
            </select>
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
              placeholder="gpt-4o-mini / glm-4"
              className="field-input"
            />
            <p className="field-help">请填写当前提供商支持的模型名称。</p>
          </section>

          {localConfig.provider === AgentProvider.OpenAi ? (
            <section className="field-group">
              <label className="field-label" htmlFor="openai-key">
                OpenAI API Key
              </label>
              <div className="password-wrap">
                <input
                  id="openai-key"
                  type={showApiKey ? 'text' : 'password'}
                  value={localConfig.openAiApiKey || ''}
                  onChange={(event) =>
                    setLocalConfig({ ...localConfig, openAiApiKey: event.target.value })
                  }
                  placeholder="sk-..."
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
              <p className="field-help">留空时将使用环境变量 OPENAI_API_KEY。</p>
            </section>
          ) : (
            <>
              <section className="field-group">
                <label className="field-label" htmlFor="glm-key">
                  GLM API Key
                </label>
                <div className="password-wrap">
                  <input
                    id="glm-key"
                    type={showApiKey ? 'text' : 'password'}
                    value={localConfig.glmApiKey || ''}
                    onChange={(event) => setLocalConfig({ ...localConfig, glmApiKey: event.target.value })}
                    placeholder="请输入 GLM API Key"
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
              </section>

              <section className="field-group">
                <label className="field-label" htmlFor="glm-url">
                  GLM Base URL
                </label>
                <input
                  id="glm-url"
                  type="text"
                  value={localConfig.glmUrl || ''}
                  onChange={(event) => setLocalConfig({ ...localConfig, glmUrl: event.target.value })}
                  placeholder="https://open.bigmodel.cn/api/coding/paas/v4"
                  className="field-input"
                />
                <p className="field-help">留空时将使用环境变量中的 URL 配置。</p>
              </section>
            </>
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
            <p className="field-help">用于定义助手的默认行为与输出风格。</p>
          </section>
        </div>

        <div className="modal-footer">
          <button type="button" onClick={onClose} className="btn-ghost">
            取消
          </button>
          <button type="button" onClick={handleSave} className="btn-primary">
            <Save size={16} />
            保存设置
          </button>
        </div>
      </div>
    </div>
  );
};

export default SettingsModal;
