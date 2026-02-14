import React, { useCallback, useEffect, useRef, useState } from 'react';
import { Menu, PanelRight, Settings } from 'lucide-react';
import MessageList from './MessageList';
import ChatInput from './ChatInput';
import ProtocolPanel from './ProtocolPanel';
import Sidebar from './Sidebar';
import SettingsModal from './SettingsModal';
import type { AgentEvent, InputCard, ProtocolEventPayload, ProtocolOpPayload } from '../types';
import { AgentEventType } from '../types';
import { TauriAPI } from '../lib/tauri';
import { logger } from '../lib/logger';
import { useAppStore } from '../store/appStore';

const COMPACT_LAYOUT_QUERY = '(max-width: 1024px)';

const detectCompactLayout = (): boolean => {
  if (typeof window === 'undefined') {
    return false;
  }
  return window.matchMedia(COMPACT_LAYOUT_QUERY).matches;
};

const createTaskId = (): string => {
  if (typeof crypto !== 'undefined' && typeof crypto.randomUUID === 'function') {
    return crypto.randomUUID();
  }
  return `task-${Date.now()}-${Math.random().toString(36).slice(2)}`;
};

const ChatView: React.FC = () => {
  const {
    config,
    conversations,
    currentConversationId,
    createConversation,
    ensureConversation,
    addMessage,
    addOpCard,
    addProtocolEventCard,
    beginStream,
    appendStreamDelta,
    completeStream,
    failStream,
    cancelStream,
    setCurrentConversation,
    streamingTaskId,
    exportConversation,
    deleteConversation,
    retryFromCard,
    hydrate,
    isHydrated,
  } = useAppStore();

  const [isCompactLayout, setIsCompactLayout] = useState(detectCompactLayout);
  const [sidebarOpen, setSidebarOpen] = useState(() => !detectCompactLayout());
  const [settingsOpen, setSettingsOpen] = useState(false);
  const [protocolOpen, setProtocolOpen] = useState(() => !detectCompactLayout());
  const unlistenRef = useRef<(() => void) | null>(null);

  const currentConversation = conversations.find(
    (conversation) => conversation.id === currentConversationId
  );

  useEffect(() => {
    void hydrate();
  }, [hydrate]);

  useEffect(() => {
    if (isHydrated && conversations.length === 0 && !currentConversationId) {
      createConversation();
    }
  }, [isHydrated, conversations.length, currentConversationId, createConversation]);

  useEffect(() => {
    if (!isHydrated) {
      return;
    }

    void TauriAPI.updateRuntimeConfig(config).catch((error) => {
      logger.error('Failed to sync runtime config', {
        error,
        provider: config.provider,
        model: config.model,
      });
    });
  }, [config, isHydrated]);

  useEffect(() => {
    if (typeof window === 'undefined') {
      return;
    }

    const mediaQuery = window.matchMedia(COMPACT_LAYOUT_QUERY);
    const handleChange = (event: MediaQueryListEvent) => {
      setIsCompactLayout(event.matches);
    };

    setIsCompactLayout(mediaQuery.matches);
    mediaQuery.addEventListener('change', handleChange);
    return () => mediaQuery.removeEventListener('change', handleChange);
  }, []);

  useEffect(() => {
    if (!isCompactLayout) {
      return;
    }
    setSidebarOpen(false);
    setProtocolOpen(false);
  }, [isCompactLayout]);

  const handleAgentEvent = useCallback(
    (event: AgentEvent) => {
      switch (event.type) {
        case AgentEventType.Started: {
          logger.debug('Agent stream started event', { taskId: event.task_id });
          break;
        }
        case AgentEventType.Delta: {
          if (typeof event.chunk === 'string' && event.chunk.length > 0) {
            appendStreamDelta(event.task_id, event.chunk);
          }
          break;
        }
        case AgentEventType.Completed: {
          completeStream(event.task_id, event.output ?? '');
          break;
        }
        case AgentEventType.Error: {
          const errorMessage = event.message ?? 'Unknown streaming error';
          failStream(event.task_id, errorMessage);
          logger.error('Agent stream error event', {
            taskId: event.task_id,
            code: event.code,
            message: errorMessage,
          });
          break;
        }
        case AgentEventType.OpSubmitted: {
          if (event.payload && typeof event.seq === 'number') {
            addOpCard(event.task_id, event.seq, event.payload as ProtocolOpPayload);
          }
          break;
        }
        case AgentEventType.ProtocolEvent: {
          if (event.payload && typeof event.seq === 'number') {
            addProtocolEventCard(event.task_id, event.seq, event.payload as ProtocolEventPayload);
          }
          break;
        }
        default: {
          logger.warn('Unknown agent event type', { event });
        }
      }
    },
    [addOpCard, addProtocolEventCard, appendStreamDelta, completeStream, failStream]
  );

  useEffect(() => {
    let active = true;

    void TauriAPI.listenAgentEvents((event) => {
      if (!active) {
        return;
      }
      handleAgentEvent(event);
    })
      .then((unlisten) => {
        if (!active) {
          unlisten();
          return;
        }
        unlistenRef.current = unlisten;
      })
      .catch((error) => {
        logger.error('Failed to setup agent event listener', { error });
      });

    return () => {
      active = false;
      if (unlistenRef.current) {
        unlistenRef.current();
        unlistenRef.current = null;
      }
    };
  }, [handleAgentEvent]);

  const handleSendMessage = useCallback(
    async (input: InputCard) => {
      if (!isHydrated || streamingTaskId) {
        return;
      }

      const conversationId = ensureConversation();
      if (currentConversationId !== conversationId) {
        setCurrentConversation(conversationId);
      }

      const userContent = input.kind === 'command' ? `$ ${input.content}` : input.content;
      const userMessageId = addMessage(conversationId, {
        role: 'user',
        content: userContent,
      });

      if (!userMessageId) {
        logger.error('Failed to append user message', { conversationId });
        return;
      }

      const taskId = createTaskId();
      const assistantMessageId = beginStream(conversationId, taskId, input);
      if (!assistantMessageId) {
        logger.error('Failed to create stream placeholder message', {
          conversationId,
          taskId,
        });
        return;
      }

      try {
        await TauriAPI.startAgentStream(input, taskId);
        logger.info('Agent stream request sent', {
          taskId,
          conversationId,
          assistantMessageId,
          inputKind: input.kind,
        });
      } catch (error) {
        const errorMessage = error instanceof Error ? error.message : 'Unknown error';
        failStream(taskId, errorMessage);
        logger.error('Failed to start agent stream', {
          taskId,
          conversationId,
          error,
        });
      }
    },
    [
      addMessage,
      beginStream,
      currentConversationId,
      ensureConversation,
      failStream,
      isHydrated,
      setCurrentConversation,
      streamingTaskId,
    ]
  );

  const handleRetryCard = useCallback(
    (cardId: string) => {
      if (!currentConversationId || streamingTaskId) {
        return;
      }
      const input = retryFromCard(currentConversationId, cardId);
      if (input) {
        void handleSendMessage(input);
      }
    },
    [currentConversationId, handleSendMessage, retryFromCard, streamingTaskId]
  );

  const handleCancelStream = useCallback(async () => {
    if (!streamingTaskId) {
      return;
    }

    try {
      await TauriAPI.cancelAgentTask(streamingTaskId);
    } catch (error) {
      logger.error('Failed to cancel stream task', {
        taskId: streamingTaskId,
        error,
      });
    } finally {
      cancelStream(streamingTaskId);
    }
  }, [cancelStream, streamingTaskId]);

  const isStreaming = streamingTaskId !== null;

  return (
    <div className="app-shell">
      <div className="app-canvas">
        {isCompactLayout && sidebarOpen && (
          <button
            type="button"
            aria-label="关闭侧边栏"
            className="app-overlay"
            onClick={() => setSidebarOpen(false)}
          />
        )}

        {isCompactLayout && protocolOpen && (
          <button
            type="button"
            aria-label="关闭协议面板"
            className="protocol-overlay"
            onClick={() => setProtocolOpen(false)}
          />
        )}

        <aside className={`app-sidebar ${sidebarOpen ? 'open' : ''}`}>
          <Sidebar
            conversations={conversations}
            currentConversationId={currentConversationId}
            onNewConversation={() => {
              createConversation();
              if (isCompactLayout) {
                setSidebarOpen(false);
              }
            }}
            onSelectConversation={(id) => {
              setCurrentConversation(id);
              if (isCompactLayout) {
                setSidebarOpen(false);
              }
            }}
            onDeleteConversation={deleteConversation}
            onExportConversation={exportConversation}
            onClose={() => setSidebarOpen(false)}
          />
        </aside>

        <main className="app-main">
          <header className="topbar">
            <div className="topbar-left">
              <button
                type="button"
                onClick={() => setSidebarOpen((value) => !value)}
                className={`icon-btn ${sidebarOpen ? 'is-active' : ''}`}
                title={sidebarOpen ? '隐藏侧边栏' : '显示侧边栏'}
                aria-label={sidebarOpen ? '隐藏侧边栏' : '显示侧边栏'}
              >
                <Menu size={20} />
              </button>

              <div>
                <h1 className="topbar-title">{currentConversation?.title || 'AI 桌面助手'}</h1>
                <p className="topbar-subtitle">
                  {isStreaming
                    ? '正在实时生成回复与协议事件...'
                    : '左侧对话 + 右侧 Op/Event 协议卡片时间线'}
                </p>
              </div>
            </div>

            <div className="topbar-actions">
              <span className={`status-pill ${isStreaming ? 'live' : ''}`}>
                {isStreaming ? '进行中' : '就绪'}
              </span>

              <button
                type="button"
                onClick={() => setProtocolOpen((value) => !value)}
                className={`icon-btn ${protocolOpen ? 'is-active' : ''}`}
                title={protocolOpen ? '隐藏协议面板' : '显示协议面板'}
                aria-label={protocolOpen ? '隐藏协议面板' : '显示协议面板'}
              >
                <PanelRight size={20} />
              </button>

              <button
                type="button"
                onClick={() => setSettingsOpen(true)}
                className="icon-btn"
                title="设置"
                aria-label="打开设置"
              >
                <Settings size={20} />
              </button>
            </div>
          </header>

          <div className="workspace">
            <section className="chat-column">
              <MessageList messages={currentConversation?.messages || []} />
              <ChatInput
                onSend={handleSendMessage}
                disabled={!isHydrated || isStreaming}
                isStreaming={isStreaming}
                onCancel={handleCancelStream}
              />
            </section>

            <section className={`protocol-column ${protocolOpen ? 'open' : ''}`}>
              <ProtocolPanel cards={currentConversation?.protocolCards || []} onRetry={handleRetryCard} />
            </section>
          </div>
        </main>
      </div>

      <SettingsModal isOpen={settingsOpen} onClose={() => setSettingsOpen(false)} />
    </div>
  );
};

export default ChatView;
