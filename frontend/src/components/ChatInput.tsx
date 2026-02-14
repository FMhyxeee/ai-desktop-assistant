import React, { useEffect, useRef, useState } from 'react';
import { Send, Square } from 'lucide-react';
import type { InputCard } from '../types';
import { InputCardKind } from '../types';

interface ChatInputProps {
  onSend: (input: InputCard) => void;
  disabled?: boolean;
  isStreaming?: boolean;
  onCancel?: () => void;
  draftInput?: InputCard | null;
  onDraftConsumed?: () => void;
}

const ChatInput: React.FC<ChatInputProps> = ({
  onSend,
  disabled = false,
  isStreaming = false,
  onCancel,
  draftInput,
  onDraftConsumed,
}) => {
  const [input, setInput] = useState('');
  const [inputMode, setInputMode] = useState<InputCardKind>(InputCardKind.Text);
  const textareaRef = useRef<HTMLTextAreaElement>(null);

  useEffect(() => {
    if (!textareaRef.current) {
      return;
    }
    textareaRef.current.style.height = 'auto';
    textareaRef.current.style.height = `${Math.min(textareaRef.current.scrollHeight, 220)}px`;
  }, [input]);

  useEffect(() => {
    if (!draftInput) {
      return;
    }
    setInputMode(draftInput.kind);
    setInput(draftInput.content);
    onDraftConsumed?.();
    requestAnimationFrame(() => {
      textareaRef.current?.focus();
      const textarea = textareaRef.current;
      if (!textarea) {
        return;
      }
      const length = textarea.value.length;
      textarea.setSelectionRange(length, length);
    });
  }, [draftInput, onDraftConsumed]);

  const handleSend = () => {
    const trimmed = input.trim();
    if (!trimmed || disabled) {
      return;
    }

    if (inputMode === InputCardKind.Command) {
      const confirmed = window.confirm(`确认执行命令？\n\n${trimmed}`);
      if (!confirmed) {
        return;
      }
    }

    onSend({
      kind: inputMode,
      content: trimmed,
    });
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
        <div className="composer-mode">
          <button
            type="button"
            className={`mode-chip ${inputMode === InputCardKind.Text ? 'active' : ''}`}
            onClick={() => setInputMode(InputCardKind.Text)}
            disabled={disabled || isStreaming}
          >
            Text
          </button>
          <button
            type="button"
            className={`mode-chip ${inputMode === InputCardKind.Command ? 'active' : ''}`}
            onClick={() => setInputMode(InputCardKind.Command)}
            disabled={disabled || isStreaming}
          >
            Command
          </button>
        </div>

        <textarea
          ref={textareaRef}
          value={input}
          onChange={(event) => setInput(event.target.value)}
          onKeyDown={handleKeyDown}
          placeholder={
            inputMode === InputCardKind.Command
              ? '输入命令...（发送前会二次确认）'
              : '请输入你的问题...（Enter 发送，Shift+Enter 换行）'
          }
          disabled={disabled}
          rows={1}
          className="composer-input"
        />

        <div className="composer-controls">
          <div className="composer-hint">
            <span>
              {isStreaming
                ? '正在流式回复中...'
                : inputMode === InputCardKind.Command
                  ? '命令模式：将发送 RunUserShellCommand'
                  : '文本模式：将发送 UserTurn'}
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
