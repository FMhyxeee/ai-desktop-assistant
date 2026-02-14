import React, { Suspense, lazy, useEffect, useMemo, useRef } from 'react';
import { Bot, Sparkles, User } from 'lucide-react';
import type { Message } from '../types';

const MarkdownMessage = lazy(() => import('./MarkdownMessage'));

interface MessageListProps {
  messages: Message[];
  streamingMessage?: Message;
}

const MessageList: React.FC<MessageListProps> = ({ messages, streamingMessage }) => {
  const messagesEndRef = useRef<HTMLDivElement>(null);

  const allMessages = useMemo(
    () => [...messages, ...(streamingMessage ? [streamingMessage] : [])],
    [messages, streamingMessage]
  );

  useEffect(() => {
    messagesEndRef.current?.scrollIntoView({ behavior: 'smooth' });
  }, [allMessages]);

  return (
    <div className="message-viewport">
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
                  <p className="message-content">{message.content}</p>
                ) : (
                  <div>
                    <Suspense fallback={<p className="message-content">{message.content}</p>}>
                      <MarkdownMessage content={message.content} />
                    </Suspense>
                    {message.isStreaming && <span className="stream-cursor" />}
                  </div>
                )}
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
