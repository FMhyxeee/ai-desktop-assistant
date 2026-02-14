import React, { useCallback, useEffect, useRef, useState } from 'react';
import { Menu, Settings } from 'lucide-react';
import MessageList from './MessageList';
import ChatInput from './ChatInput';
import Sidebar from './Sidebar';
import SettingsModal from './SettingsModal';
import type { AgentEvent } from '../types';
import { AgentEventType } from '../types';
import { TauriAPI } from '../lib/tauri';
import { logger } from '../lib/logger';
import { useAppStore } from '../store/appStore';

const createTaskId = (): string => {
  if (typeof crypto !== 'undefined' && typeof crypto.randomUUID === 'function') {
    return crypto.randomUUID();
  }
  return `task-${Date.now()}-${Math.random().toString(36).slice(2)}`;
};

const ChatView: React.FC = () => {
  const {
    conversations,
    currentConversationId,
    createConversation,
    ensureConversation,
    addMessage,
    beginStream,
    appendStreamDelta,
    completeStream,
    failStream,
    cancelStream,
    setCurrentConversation,
    streamingTaskId,
    exportConversation,
    deleteConversation,
    hydrate,
    isHydrated,
  } = useAppStore();

  const [sidebarOpen, setSidebarOpen] = useState(false);
  const [settingsOpen, setSettingsOpen] = useState(false);
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
        default: {
          logger.warn('Unknown agent event type', { event });
        }
      }
    },
    [appendStreamDelta, completeStream, failStream]
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
    async (content: string) => {
      if (!isHydrated || streamingTaskId) {
        return;
      }

      const conversationId = ensureConversation();
      if (currentConversationId !== conversationId) {
        setCurrentConversation(conversationId);
      }

      const userMessageId = addMessage(conversationId, {
        role: 'user',
        content,
      });

      if (!userMessageId) {
        logger.error('Failed to append user message', { conversationId });
        return;
      }

      const taskId = createTaskId();
      const assistantMessageId = beginStream(conversationId, taskId);
      if (!assistantMessageId) {
        logger.error('Failed to create stream placeholder message', {
          conversationId,
          taskId,
        });
        return;
      }

      try {
        await TauriAPI.startAgentStream(content, taskId);
        logger.info('Agent stream request sent', {
          taskId,
          conversationId,
          assistantMessageId,
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
        {sidebarOpen && (
          <button
            type="button"
            aria-label="关闭侧边栏"
            className="app-overlay"
            onClick={() => setSidebarOpen(false)}
          />
        )}

        <aside className={`app-sidebar ${sidebarOpen ? 'open' : ''}`}>
          <Sidebar
            conversations={conversations}
            currentConversationId={currentConversationId}
            onNewConversation={() => {
              createConversation();
              setSidebarOpen(false);
            }}
            onSelectConversation={(id) => {
              setCurrentConversation(id);
              setSidebarOpen(false);
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
                onClick={() => setSidebarOpen(true)}
                className="icon-btn mobile-only"
                aria-label="打开侧边栏"
              >
                <Menu size={20} />
              </button>

              <div>
                <h1 className="topbar-title">
                  {currentConversation?.title || 'AI 桌面助手'}
                </h1>
                <p className="topbar-subtitle">
                  {isStreaming
                    ? '正在实时生成回复...'
                    : '支持流式输出与 Markdown 渲染的结构化对话工作区'}
                </p>
              </div>
            </div>

            <div className="topbar-actions">
              <span className={`status-pill ${isStreaming ? 'live' : ''}`}>
                {isStreaming ? '进行中' : '就绪'}
              </span>

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

          <MessageList messages={currentConversation?.messages || []} />

          <ChatInput
            onSend={handleSendMessage}
            disabled={!isHydrated || isStreaming}
            isStreaming={isStreaming}
            onCancel={handleCancelStream}
          />
        </main>
      </div>

      <SettingsModal isOpen={settingsOpen} onClose={() => setSettingsOpen(false)} />
    </div>
  );
};

export default ChatView;
