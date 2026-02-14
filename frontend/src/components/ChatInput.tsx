import React, { useEffect, useRef, useState } from 'react';
import { Send, Square } from 'lucide-react';

interface ChatInputProps {
  onSend: (message: string) => void;
  disabled?: boolean;
  isStreaming?: boolean;
  onCancel?: () => void;
}

const ChatInput: React.FC<ChatInputProps> = ({
  onSend,
  disabled = false,
  isStreaming = false,
  onCancel,
}) => {
  const [input, setInput] = useState('');
  const textareaRef = useRef<HTMLTextAreaElement>(null);

  useEffect(() => {
    if (!textareaRef.current) {
      return;
    }
    textareaRef.current.style.height = 'auto';
    textareaRef.current.style.height = `${Math.min(textareaRef.current.scrollHeight, 220)}px`;
  }, [input]);

  const handleSend = () => {
    const trimmed = input.trim();
    if (!trimmed || disabled) {
      return;
    }

    onSend(trimmed);
    setInput('');

    if (textareaRef.current) {
      textareaRef.current.style.height = 'auto';
    }
  };

  const handleKeyDown = (event: React.KeyboardEvent<HTMLTextAreaElement>) => {
    if (event.key === 'Enter' && !event.shiftKey) {
      event.preventDefault();
      handleSend();
    }
  };

  return (
    <div className="composer-wrap">
      <div className="composer-panel">
        <textarea
          ref={textareaRef}
          value={input}
          onChange={(event) => setInput(event.target.value)}
          onKeyDown={handleKeyDown}
          placeholder="请输入你的问题...（Enter 发送，Shift+Enter 换行）"
          disabled={disabled}
          rows={1}
          className="composer-input"
        />

        <div className="composer-controls">
          <div className="composer-hint">
            <span>
              {isStreaming ? '正在流式回复中...' : '支持 Markdown 与代码块高亮'}
            </span>
            <span className="composer-count">{input.length}</span>
          </div>

          {isStreaming ? (
            <button
              type="button"
              onClick={onCancel}
              className="composer-button stop"
              disabled={!onCancel}
            >
              <Square size={16} />
              停止
            </button>
          ) : (
            <button
              type="button"
              onClick={handleSend}
              className="composer-button primary"
              disabled={disabled || input.trim().length === 0}
            >
              <Send size={16} />
              发送
            </button>
          )}
        </div>
      </div>
    </div>
  );
};

export default ChatInput;
