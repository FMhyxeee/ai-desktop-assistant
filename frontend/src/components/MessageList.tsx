import React, { Suspense, lazy, useEffect, useMemo, useRef, useState } from 'react';
import { Bot, Copy, CornerDownRight, RotateCcw, Sparkles, User } from 'lucide-react';
import type { Message } from '../types';

const MarkdownMessage = lazy(() => import('./MarkdownMessage'));

interface MessageListProps {
  messages: Message[];
  streamingMessage?: Message;
  onRegenerateMessage?: (messageId: string) => void;
  onContinueFromMessage?: (messageId: string) => void;
}

const MessageList: React.FC<MessageListProps> = ({
  messages,
  streamingMessage,
  onRegenerateMessage,
  onContinueFromMessage,
}) => {
  const viewportRef = useRef<HTMLDivElement>(null);
  const messagesEndRef = useRef<HTMLDivElement>(null);
  const shouldAutoScrollRef = useRef(true);
  const [copiedMessageId, setCopiedMessageId] = useState<string | null>(null);

  const allMessages = useMemo(
    () => [...messages, ...(streamingMessage ? [streamingMessage] : [])],
    [messages, streamingMessage]
  );

  useEffect(() => {
    const viewport = viewportRef.current;
    if (!viewport || !shouldAutoScrollRef.current) {
      return;
    }

    const isStreaming = allMessages[allMessages.length - 1]?.isStreaming === true;
    messagesEndRef.current?.scrollIntoView({ behavior: isStreaming ? 'auto' : 'smooth' });
  }, [allMessages]);

  const handleViewportScroll = () => {
    const viewport = viewportRef.current;
    if (!viewport) {
      return;
    }

    const distanceToBottom = viewport.scrollHeight - viewport.scrollTop - viewport.clientHeight;
    shouldAutoScrollRef.current = distanceToBottom <= 120;
  };

  const copyMessage = (messageId: string, content: string) => {
    void navigator.clipboard.writeText(content).then(() => {
      setCopiedMessageId(messageId);
      window.setTimeout(() => {
        setCopiedMessageId((value) => (value === messageId ? null : value));
      }, 1000);
    });
  };

  return (
    <div className="message-viewport" ref={viewportRef} onScroll={handleViewportScroll}>
      {allMessages.length === 0 && (
        <section className="empty-state">
          <div className="empty-icon">
            <Bot size={30} />
          </div>
          <h2 className="empty-title">从这里开始你的高质量对话</h2>
          <p className="empty-description">
            支持流式输出、Markdown 渲染与多会话切换，让你在一个界面里完成思考、实现与迭代。
          </p>
          <ul className="empty-tips">
            <li>
              <Sparkles size={14} /> 先让助手给出实现计划，再进入编码阶段。
            </li>
            <li>
              <Sparkles size={14} /> 让助手按文件输出变更，便于快速评审。
            </li>
            <li>
              <Sparkles size={14} /> 尽量使用简短明确的指令，回复会更精准。
            </li>
          </ul>
        </section>
      )}

      <div className="message-stack">
        {allMessages.map((message, index) => {
          const isUser = message.role === 'user';
          const isAssistant = message.role === 'assistant';
          const messageImages = message.images ?? [];

          return (
            <article
              key={message.id}
              className={`message-row ${isUser ? 'user' : 'assistant'}`}
              style={{ animationDelay: `${index * 40}ms` }}
            >
              {isAssistant && (
                <div className="message-avatar assistant" aria-hidden="true">
                  <Bot size={16} />
                </div>
              )}

              <div className={`message-bubble ${isUser ? 'user' : 'assistant'}`}>
                {isUser ? (
                  <div>
                    {messageImages.length > 0 && (
                      <div className="message-images">
                        {messageImages.map((image) => (
                          <img
                            key={image.id}
                            src={image.dataUrl}
                            alt={image.name}
                            className="message-image"
                          />
                        ))}
                      </div>
                    )}
                    {message.content.length > 0 && <p className="message-content">{message.content}</p>}
                  </div>
                ) : (
                  <div>
                    {messageImages.length > 0 && (
                      <div className="message-images">
                        {messageImages.map((image) => (
                          <img
                            key={image.id}
                            src={image.dataUrl}
                            alt={image.name}
                            className="message-image"
                          />
                        ))}
                      </div>
                    )}
                    {message.isStreaming ? (
                      <p className="message-content">{message.content}</p>
                    ) : (
                      <Suspense fallback={<p className="message-content">{message.content}</p>}>
                        <MarkdownMessage content={message.content} />
                      </Suspense>
                    )}
                    {message.isStreaming && <span className="stream-cursor" />}
                  </div>
                )}

                <div className="message-tools">
                  <button
                    type="button"
                    className="message-tool-btn"
                    onClick={() => copyMessage(message.id, message.content)}
                  >
                    <Copy size={13} />
                    {copiedMessageId === message.id ? '已复制' : '复制'}
                  </button>

                  {onContinueFromMessage && (
                    <button
                      type="button"
                      className="message-tool-btn"
                      onClick={() => onContinueFromMessage(message.id)}
                    >
                      <CornerDownRight size={13} />
                      继续追问
                    </button>
                  )}

                  {isAssistant && onRegenerateMessage && !message.isStreaming && (
                    <button
                      type="button"
                      className="message-tool-btn"
                      onClick={() => onRegenerateMessage(message.id)}
                    >
                      <RotateCcw size={13} />
                      重新生成
                    </button>
                  )}
                </div>
              </div>

              {isUser && (
                <div className="message-avatar user" aria-hidden="true">
                  <User size={16} />
                </div>
              )}
            </article>
          );
        })}
      </div>

      <div ref={messagesEndRef} />
    </div>
  );
};

export default MessageList;
