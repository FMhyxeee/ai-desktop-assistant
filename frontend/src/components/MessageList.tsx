import React, { Suspense, lazy, useEffect, useMemo, useRef, useState } from 'react';
import {
  Bot,
  Brain,
  ChevronDown,
  ChevronRight,
  Copy,
  CornerDownRight,
  RotateCcw,
  Sparkles,
  Wrench,
} from 'lucide-react';
import { logger } from '../lib/logger';
import type { Message, ProtocolCard } from '../types';

const MarkdownMessage = lazy(() => import('./MarkdownMessage'));

interface MessageListProps {
  messages: Message[];
  protocolCards: ProtocolCard[];
  streamingMessage?: Message;
  streamingTaskIds?: Set<string>;
  activeThinkTaskId?: string | null;
  onRegenerateMessage?: (messageId: string) => void;
  onContinueFromMessage?: (messageId: string) => void;
}

interface ThinkTimelineEntry {
  id: string;
  label: string;
  timestamp: number;
  durationMs?: number;
}

interface ThinkTimeline {
  entries: ThinkTimelineEntry[];
  isRunning: boolean;
  runningSinceMs: number | null;
}

const formatTime = (timestamp: number): string =>
  new Date(timestamp).toLocaleTimeString('zh-CN', {
    hour: '2-digit',
    minute: '2-digit',
    second: '2-digit',
  });

const formatDuration = (durationMs: number): string => {
  if (durationMs < 1000) {
    return `${durationMs}ms`;
  }

  const seconds = durationMs / 1000;
  if (seconds < 60) {
    return `${seconds >= 10 ? seconds.toFixed(0) : seconds.toFixed(1)}s`;
  }

  const minutes = Math.floor(seconds / 60);
  const remainSeconds = Math.round(seconds % 60);
  return `${minutes}m ${remainSeconds}s`;
};

const payloadToText = (payload: unknown): string => {
  try {
    return JSON.stringify(payload, null, 2);
  } catch {
    return String(payload);
  }
};

const EXCLUDED_TOOL_EVENT_TYPES = new Set<string>([
  'model_streaming',
  'model_complete',
  'turn_started',
  'turn_complete',
  'think_status',
  'guidance_context',
  'governance_report',
  'conversation_title_suggestion',
  'control_decision',
  'config_change_request',
  'config_change_result',
]);

const isToolCard = (card: ProtocolCard): boolean => {
  if (card.direction === 'op') {
    return card.type !== 'user_turn';
  }
  if (card.direction === 'event') {
    return !EXCLUDED_TOOL_EVENT_TYPES.has(card.type);
  }
  return false;
};

const buildThinkTimeline = (
  taskCards: ProtocolCard[],
  taskId: string | undefined,
  activeThinkTaskId: string | null | undefined
): ThinkTimeline => {
  const thinkCards = taskCards.filter((card) => card.direction === 'event' && card.type === 'think_status');
  const entries: ThinkTimelineEntry[] = [];
  let startedAt: number | null = null;

  thinkCards.forEach((card) => {
    const payload = card.payload as { type: 'think_status'; active: boolean };
    if (payload.active) {
      startedAt = card.timestamp;
      entries.push({
        id: `${card.id}-start`,
        label: 'Started',
        timestamp: card.timestamp,
      });
      return;
    }

    const durationMs = startedAt === null ? undefined : Math.max(0, card.timestamp - startedAt);
    entries.push({
      id: `${card.id}-stop`,
      label: 'Stopped',
      timestamp: card.timestamp,
      durationMs,
    });
    startedAt = null;
  });

  const isRunning = Boolean((taskId && activeThinkTaskId === taskId) || startedAt !== null);

  return {
    entries,
    isRunning,
    runningSinceMs: startedAt,
  };
};

const MessageList: React.FC<MessageListProps> = ({
  messages,
  protocolCards,
  streamingMessage,
  streamingTaskIds,
  activeThinkTaskId,
  onRegenerateMessage,
  onContinueFromMessage,
}) => {
  const viewportRef = useRef<HTMLDivElement>(null);
  const messagesEndRef = useRef<HTMLDivElement>(null);
  const shouldAutoScrollRef = useRef(true);
  const lastBindingWarningRef = useRef<string | null>(null);
  const [copiedMessageId, setCopiedMessageId] = useState<string | null>(null);
  const [toolSectionExpanded, setToolSectionExpanded] = useState<Record<string, boolean>>({});
  const [thinkSectionExpanded, setThinkSectionExpanded] = useState<Record<string, boolean>>({});

  const allMessages = useMemo(
    () => [...messages, ...(streamingMessage ? [streamingMessage] : [])],
    [messages, streamingMessage]
  );

  const sortedProtocolCards = useMemo(
    () =>
      [...protocolCards].sort((a, b) => {
        if (a.timestamp === b.timestamp) {
          return a.seq - b.seq;
        }
        return a.timestamp - b.timestamp;
      }),
    [protocolCards]
  );

  const protocolCardsByTask = useMemo(() => {
    const grouped = new Map<string, ProtocolCard[]>();
    sortedProtocolCards.forEach((card) => {
      if (!grouped.has(card.taskId)) {
        grouped.set(card.taskId, []);
      }
      grouped.get(card.taskId)?.push(card);
    });
    return grouped;
  }, [sortedProtocolCards]);

  const assistantTaskBinding = useMemo(() => {
    const assistantMessages = allMessages.filter((message) => message.role === 'assistant');
    const taskOrder: string[] = [];
    const seenTaskIds = new Set<string>();

    sortedProtocolCards.forEach((card) => {
      if (seenTaskIds.has(card.taskId)) {
        return;
      }
      seenTaskIds.add(card.taskId);
      taskOrder.push(card.taskId);
    });

    const messageTaskMap = new Map<string, string>();
    const pairCount = Math.min(assistantMessages.length, taskOrder.length);

    for (let index = 0; index < pairCount; index += 1) {
      messageTaskMap.set(assistantMessages[index].id, taskOrder[index]);
    }

    let forcedStreamingBind = false;
    // Handle multiple concurrent streaming tasks
    if (streamingTaskIds && streamingTaskIds.size > 0) {
      const streamingAssistantMessages = [...assistantMessages]
        .reverse()
        .filter((message) => message.isStreaming);

      // Bind each streaming message to a task ID from the set
      streamingAssistantMessages.forEach((streamingMessage, index) => {
        if (!messageTaskMap.has(streamingMessage.id)) {
          // Get the corresponding task ID from the set
          // In order, assign streaming messages to task IDs
          const taskIdArray = Array.from(streamingTaskIds);
          if (index < taskIdArray.length) {
            messageTaskMap.set(streamingMessage.id, taskIdArray[index]);
            forcedStreamingBind = true;
          }
        }
      });
    }

    const unboundAssistantMessageIds = assistantMessages
      .filter((message) => !messageTaskMap.has(message.id))
      .map((message) => message.id);

    return {
      messageTaskMap,
      assistantCount: assistantMessages.length,
      taskCount: taskOrder.length,
      forcedStreamingBind,
      unboundAssistantMessageIds,
    };
  }, [allMessages, sortedProtocolCards, streamingTaskIds]);

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

  useEffect(() => {
    const { assistantCount, taskCount, forcedStreamingBind, unboundAssistantMessageIds } = assistantTaskBinding;
    const hasMismatch = assistantCount !== taskCount || unboundAssistantMessageIds.length > 0;
    if (!hasMismatch && !forcedStreamingBind) {
      lastBindingWarningRef.current = null;
      return;
    }

    const warningSignature = JSON.stringify({
      assistantCount,
      taskCount,
      forcedStreamingBind,
      unboundAssistantMessageIds,
    });

    if (lastBindingWarningRef.current === warningSignature) {
      return;
    }

    lastBindingWarningRef.current = warningSignature;
    logger.warn('Assistant message to task binding mismatch', {
      assistantCount,
      taskCount,
      forcedStreamingBind,
      unboundAssistantMessageIds,
    });
  }, [assistantTaskBinding]);

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
          const taskId = isAssistant ? assistantTaskBinding.messageTaskMap.get(message.id) : undefined;
          const taskCards = taskId ? protocolCardsByTask.get(taskId) ?? [] : [];
          const thinkTimeline = buildThinkTimeline(taskCards, taskId, activeThinkTaskId);
          const toolCards = taskCards.filter(isToolCard);

          const hasThinkSection = thinkTimeline.entries.length > 0 || thinkTimeline.isRunning;
          const hasToolSection = toolCards.length > 0;
          const isThinkExpanded = thinkTimeline.isRunning
            ? true
            : (thinkSectionExpanded[message.id] ?? false);
          const isToolExpanded = toolSectionExpanded[message.id] ?? false;
          const showStreamingCursor = message.isStreaming && message.content.trim().length > 0;

          return (
            <article
              key={message.id}
              className={`message-row ${isUser ? 'user' : 'assistant'}`}
              style={{ animationDelay: `${index * 40}ms` }}
            >
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
                      <p className={`message-content ${message.content.length === 0 ? 'message-content-muted' : ''}`}>
                        {message.content || 'Waiting for response...'}
                      </p>
                    ) : (
                      <Suspense fallback={<p className="message-content">{message.content}</p>}>
                        <MarkdownMessage content={message.content} />
                      </Suspense>
                    )}
                    {showStreamingCursor && <span className="stream-cursor" />}

                    {hasThinkSection && (
                      <section className={`assistant-detail-section ${isThinkExpanded ? 'open' : ''}`}>
                        <button
                          type="button"
                          className="assistant-detail-toggle"
                          onClick={() => {
                            if (thinkTimeline.isRunning) {
                              return;
                            }
                            setThinkSectionExpanded((state) => ({
                              ...state,
                              [message.id]: !(state[message.id] ?? false),
                            }));
                          }}
                          aria-expanded={isThinkExpanded}
                        >
                          <span className="assistant-detail-title">
                            <Brain size={14} />
                            Think
                          </span>
                          <span className={`assistant-detail-badge ${thinkTimeline.isRunning ? 'live' : ''}`}>
                            {thinkTimeline.isRunning
                              ? 'running'
                              : `${thinkTimeline.entries.length} event${thinkTimeline.entries.length > 1 ? 's' : ''}`}
                          </span>
                          {isThinkExpanded ? <ChevronDown size={14} /> : <ChevronRight size={14} />}
                        </button>

                        {isThinkExpanded && (
                          <div className="assistant-detail-body">
                            <ul className="assistant-think-timeline">
                              {thinkTimeline.entries.map((entry) => (
                                <li key={entry.id} className="assistant-think-item">
                                  <span className="assistant-think-label">{entry.label}</span>
                                  <span className="assistant-think-time">{formatTime(entry.timestamp)}</span>
                                  {typeof entry.durationMs === 'number' && (
                                    <span className="assistant-think-duration">
                                      {formatDuration(entry.durationMs)}
                                    </span>
                                  )}
                                </li>
                              ))}
                              {thinkTimeline.isRunning && (
                                <li className="assistant-think-item running">
                                  <span className="assistant-think-label">Running</span>
                                  <span className="assistant-think-time">
                                    {thinkTimeline.runningSinceMs
                                      ? `since ${formatTime(thinkTimeline.runningSinceMs)}`
                                      : 'active'}
                                  </span>
                                </li>
                              )}
                            </ul>
                          </div>
                        )}
                      </section>
                    )}

                    {hasToolSection && (
                      <section className={`assistant-detail-section ${isToolExpanded ? 'open' : ''}`}>
                        <button
                          type="button"
                          className="assistant-detail-toggle"
                          onClick={() =>
                            setToolSectionExpanded((state) => ({
                              ...state,
                              [message.id]: !(state[message.id] ?? false),
                            }))
                          }
                          aria-expanded={isToolExpanded}
                        >
                          <span className="assistant-detail-title">
                            <Wrench size={14} />
                            Tool Calls
                          </span>
                          <span className="assistant-detail-badge">
                            {toolCards.length} call{toolCards.length > 1 ? 's' : ''}
                          </span>
                          {isToolExpanded ? <ChevronDown size={14} /> : <ChevronRight size={14} />}
                        </button>

                        {isToolExpanded && (
                          <div className="assistant-detail-body">
                            <ul className="assistant-tool-list">
                              {toolCards.map((card) => (
                                <li key={card.id} className="assistant-tool-item">
                                  <details className="assistant-tool-event">
                                    <summary>
                                      <span className="assistant-tool-meta">
                                        <span className={`assistant-tool-chip ${card.direction}`}>{card.direction}</span>
                                        <span className="assistant-tool-chip type">{card.type}</span>
                                        <span className="assistant-tool-time">{formatTime(card.timestamp)}</span>
                                      </span>
                                      <span className="assistant-tool-summary">{card.summary}</span>
                                    </summary>
                                    <pre className="assistant-tool-payload">
                                      <code>{payloadToText(card.payload)}</code>
                                    </pre>
                                  </details>
                                </li>
                              ))}
                            </ul>
                          </div>
                        )}
                      </section>
                    )}
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
            </article>
          );
        })}
      </div>

      <div ref={messagesEndRef} />
    </div>
  );
};

export default MessageList;
