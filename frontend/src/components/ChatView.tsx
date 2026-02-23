import React, { useCallback, useEffect, useRef, useState } from 'react';
import { open as openDialog } from '@tauri-apps/plugin-dialog';
import { FolderOpen, Menu, PanelRight, Save, Settings } from 'lucide-react';
import MessageList from './MessageList';
import ChatInput from './ChatInput';
import ProtocolPanel from './ProtocolPanel';
import Sidebar from './Sidebar';
import SettingsModal from './SettingsModal';
import type {
  AgentEvent,
  AgentHistoryMessage,
  InputCard,
  Message,
  ProtocolEventPayload,
  ProtocolOpPayload,
} from '../types';
import { AgentEventType } from '../types';
import { parseSlashInput } from '../lib/input';
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

const DEFAULT_WORKSPACE_DISPLAY = '~/.ai-helper/workspaces/default';

const normalizeWorkspaceRoot = (value: string): string => value.trim();

const workspaceDisplayText = (value: string): string =>
  value.length > 0 ? value : `自动默认目录 (${DEFAULT_WORKSPACE_DISPLAY})`;

const getErrorMessage = (error: unknown): string => {
  if (error instanceof Error && error.message) {
    return error.message;
  }
  if (typeof error === 'string' && error.trim().length > 0) {
    return error;
  }
  return 'Operation failed. Please retry.';
};

type WorkspaceFeedback = {
  type: 'success' | 'error';
  message: string;
};

const ChatView: React.FC = () => {
  const {
    config,
    updateConfig,
    conversations,
    currentConversationId,
    createConversation,
    ensureConversation,
    renameConversation,
    toggleConversationPinned,
    addMessage,
    addOpCard,
    addProtocolEventCard,
    beginStream,
    appendStreamDelta,
    completeStream,
    failStream,
    cancelStream,
    setCurrentConversation,
    activeThinkTaskId,
    streamingTaskId,
    exportConversation,
    deleteConversation,
    retryFromCard,
    hydrate,
    reloadWorkspaceConversations,
    isHydrated,
  } = useAppStore();

  const [isCompactLayout, setIsCompactLayout] = useState(detectCompactLayout);
  const [sidebarOpen, setSidebarOpen] = useState(() => !detectCompactLayout());
  const [settingsOpen, setSettingsOpen] = useState(false);
  const [protocolOpen, setProtocolOpen] = useState(false);
  const [draftInput, setDraftInput] = useState<InputCard | null>(null);
  const [workspaceEditorOpen, setWorkspaceEditorOpen] = useState(false);
  const [workspaceDraft, setWorkspaceDraft] = useState(() => config.workspace.rootDir ?? '');
  const [workspaceFeedback, setWorkspaceFeedback] = useState<WorkspaceFeedback | null>(null);
  const [isApplyingWorkspace, setIsApplyingWorkspace] = useState(false);
  const unlistenRef = useRef<(() => void) | null>(null);
  const pendingDeltaByTaskRef = useRef<Map<string, string>>(new Map());
  const deltaFlushRafRef = useRef<number | null>(null);

  const currentConversation = conversations.find(
    (conversation) => conversation.id === currentConversationId
  );
  const appliedWorkspaceRoot = normalizeWorkspaceRoot(config.workspace.rootDir ?? '');
  const workspaceDraftNormalized = normalizeWorkspaceRoot(workspaceDraft);
  const workspaceChanged = workspaceDraftNormalized !== appliedWorkspaceRoot;

  const flushPendingDeltas = useCallback(() => {
    deltaFlushRafRef.current = null;
    const pendingDeltas = pendingDeltaByTaskRef.current;
    if (pendingDeltas.size === 0) {
      return;
    }

    pendingDeltas.forEach((pendingChunk, taskId) => {
      if (pendingChunk.length > 0) {
        appendStreamDelta(taskId, pendingChunk);
      }
    });

    pendingDeltas.clear();
  }, [appendStreamDelta]);

  const flushPendingDeltaForTask = useCallback(
    (taskId: string) => {
      const pendingDeltas = pendingDeltaByTaskRef.current;
      const pendingChunk = pendingDeltas.get(taskId);
      if (!pendingChunk || pendingChunk.length === 0) {
        pendingDeltas.delete(taskId);
        return;
      }
      appendStreamDelta(taskId, pendingChunk);
      pendingDeltas.delete(taskId);
    },
    [appendStreamDelta]
  );

  const queueDeltaChunk = useCallback(
    (taskId: string, chunk: string) => {
      if (!chunk) {
        return;
      }

      const pendingDeltas = pendingDeltaByTaskRef.current;
      pendingDeltas.set(taskId, `${pendingDeltas.get(taskId) ?? ''}${chunk}`);

      if (typeof window === 'undefined') {
        flushPendingDeltas();
        return;
      }

      if (deltaFlushRafRef.current === null) {
        deltaFlushRafRef.current = window.requestAnimationFrame(flushPendingDeltas);
      }
    },
    [flushPendingDeltas]
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
    setWorkspaceDraft(config.workspace.rootDir ?? '');
  }, [config.workspace.rootDir]);

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

  useEffect(() => {
    const handleKeydown = (event: KeyboardEvent) => {
      const target = event.target as HTMLElement | null;
      const isTyping =
        target instanceof HTMLInputElement ||
        target instanceof HTMLTextAreaElement ||
        target instanceof HTMLSelectElement ||
        Boolean(target?.isContentEditable);

      if (event.key === 'Escape') {
        let handled = false;
        if (workspaceEditorOpen) {
          setWorkspaceEditorOpen(false);
          handled = true;
        }
        if (settingsOpen) {
          setSettingsOpen(false);
          handled = true;
        }
        if (sidebarOpen) {
          setSidebarOpen(false);
          handled = true;
        }
        if (protocolOpen) {
          setProtocolOpen(false);
          handled = true;
        }
        if (handled) {
          event.preventDefault();
        }
        return;
      }

      if (isTyping) {
        return;
      }

      const withModifier = event.ctrlKey || event.metaKey;
      if (!withModifier || event.shiftKey || event.altKey) {
        return;
      }

      if (event.key.toLowerCase() === 'b') {
        event.preventDefault();
        setSidebarOpen((value) => !value);
        return;
      }

      if (event.key === '\\') {
        event.preventDefault();
        setProtocolOpen((value) => !value);
      }
    };

    window.addEventListener('keydown', handleKeydown);
    return () => window.removeEventListener('keydown', handleKeydown);
  }, [protocolOpen, settingsOpen, sidebarOpen, workspaceEditorOpen]);

  const handleAgentEvent = useCallback(
    (event: AgentEvent) => {
      switch (event.type) {
        case AgentEventType.Started: {
          logger.debug('Agent stream started event', { taskId: event.task_id });
          break;
        }
        case AgentEventType.Delta: {
          if (typeof event.chunk === 'string' && event.chunk.length > 0) {
            queueDeltaChunk(event.task_id, event.chunk);
          }
          break;
        }
        case AgentEventType.Completed: {
          flushPendingDeltaForTask(event.task_id);
          completeStream(event.task_id, event.output ?? '');
          break;
        }
        case AgentEventType.Error: {
          flushPendingDeltaForTask(event.task_id);
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
            const payload = event.payload as ProtocolEventPayload;
            addProtocolEventCard(event.task_id, event.seq, payload);
            if (payload.type === 'conversation_title_suggestion') {
              renameConversation(payload.conversation_id, payload.title);
            }
          }
          break;
        }
        default: {
          logger.warn('Unknown agent event type', { event });
        }
      }
    },
    [
      addOpCard,
      addProtocolEventCard,
      completeStream,
      failStream,
      flushPendingDeltaForTask,
      queueDeltaChunk,
      renameConversation,
    ]
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

  useEffect(() => {
    const pendingDeltaByTask = pendingDeltaByTaskRef.current;
    return () => {
      if (typeof window !== 'undefined' && deltaFlushRafRef.current !== null) {
        window.cancelAnimationFrame(deltaFlushRafRef.current);
      }
      deltaFlushRafRef.current = null;
      pendingDeltaByTask.clear();
    };
  }, []);

  const handleSendMessage = useCallback(
    async (input: InputCard) => {
      if (!isHydrated || streamingTaskId) {
        return;
      }

      const conversationId = ensureConversation();
      if (currentConversationId !== conversationId) {
        setCurrentConversation(conversationId);
      }
      const historyWindowTurns = 8;
      const recentMessages: AgentHistoryMessage[] = (
        conversations.find((conversation) => conversation.id === conversationId)?.messages ?? []
      )
        .filter((message) => message.content.trim().length > 0)
        .filter(
          (message): message is Message & { role: 'user' | 'assistant' | 'system' } =>
            message.role === 'user' || message.role === 'assistant' || message.role === 'system'
        )
        .slice(-historyWindowTurns * 2)
        .map((message) => ({
          role: message.role,
          content: message.content,
        }));

      const parsedInput = parseSlashInput(input.content);
      const outboundInput: InputCard = parsedInput.isCommand
        ? {
            content: parsedInput.normalizedContent,
          }
        : {
            content: parsedInput.normalizedContent,
            images: input.images,
          };

      const userContent = parsedInput.isCommand
        ? `$ ${parsedInput.command ?? ''}`
        : parsedInput.normalizedContent;
      const userMessageId = addMessage(conversationId, {
        role: 'user',
        content: userContent,
        images: outboundInput.images,
      });

      if (!userMessageId) {
        logger.error('Failed to append user message', { conversationId });
        return;
      }

      const taskId = createTaskId();
      const assistantMessageId = beginStream(conversationId, taskId, outboundInput);
      if (!assistantMessageId) {
        logger.error('Failed to create stream placeholder message', {
          conversationId,
          taskId,
        });
        return;
      }

      try {
        await TauriAPI.startAgentStream(outboundInput, taskId, {
          conversationId,
          recentMessages,
        });
        logger.info('Agent stream request sent', {
          taskId,
          conversationId,
          assistantMessageId,
          inputMode: parsedInput.isCommand ? 'command' : 'message',
          imageCount: outboundInput.images?.length ?? 0,
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
      conversations,
      currentConversationId,
      ensureConversation,
      failStream,
      isHydrated,
      setCurrentConversation,
      streamingTaskId,
    ]
  );

  const toInputCardFromUserMessage = useCallback((message: Message): InputCard => {
    if (message.content.startsWith('$ ')) {
      return {
        content: `/${message.content.slice(2).trimStart()}`,
      };
    }
    return {
      content: message.content,
      images: message.images,
    };
  }, []);

  const handleContinueFromMessage = useCallback(
    (messageId: string) => {
      const message = currentConversation?.messages.find((item) => item.id === messageId);
      if (!message) {
        return;
      }

      const prefix =
        message.role === 'assistant'
          ? 'Continue based on this assistant reply:\n'
          : 'Continue based on this message:\n';
      setDraftInput({
        content: `${prefix}${message.content}\n`,
      });
    },
    [currentConversation]
  );

  const handleRegenerateMessage = useCallback(
    (assistantMessageId: string) => {
      if (!currentConversation || streamingTaskId) {
        return;
      }

      const assistantIndex = currentConversation.messages.findIndex(
        (message) => message.id === assistantMessageId && message.role === 'assistant'
      );

      if (assistantIndex <= 0) {
        return;
      }

      const previousUserMessage = [...currentConversation.messages.slice(0, assistantIndex)]
        .reverse()
        .find((message) => message.role === 'user');

      if (!previousUserMessage) {
        return;
      }

      void handleSendMessage(toInputCardFromUserMessage(previousUserMessage));
    },
    [currentConversation, handleSendMessage, streamingTaskId, toInputCardFromUserMessage]
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

  const handleResolveConfigChangeRequest = useCallback(
    async (taskId: string, requestId: string, approved: boolean, persist: boolean) => {
      await TauriAPI.resolveConfigChangeRequest(taskId, requestId, approved, persist);
      logger.info('Config change request resolved', {
        taskId,
        requestId,
        approved,
        persist,
      });
    },
    []
  );

  const handlePickWorkspaceDirectory = useCallback(async () => {
    try {
      const defaultPath =
        workspaceDraftNormalized.length > 0 ? workspaceDraftNormalized : undefined;
      const selected = await openDialog({
        directory: true,
        multiple: false,
        defaultPath,
      });

      if (typeof selected === 'string' && selected.trim().length > 0) {
        setWorkspaceDraft(selected.trim());
        setWorkspaceFeedback(null);
      }
    } catch (error) {
      setWorkspaceFeedback({
        type: 'error',
        message: getErrorMessage(error),
      });
    }
  }, [workspaceDraftNormalized]);

  const handleApplyWorkspace = useCallback(async () => {
    if (streamingTaskId) {
      setWorkspaceFeedback({
        type: 'error',
        message: 'A session is running. Stop generation before changing workspace.',
      });
      return;
    }

    if (!workspaceChanged) {
      return;
    }

    setIsApplyingWorkspace(true);
    setWorkspaceFeedback(null);

    const nextRootDir = workspaceDraftNormalized;
    const nextWorkspaceConfig = {
      ...config.workspace,
      rootDir: nextRootDir,
    };

    try {
      await TauriAPI.updateRuntimeConfig({
        ...config,
        workspace: nextWorkspaceConfig,
      });
      await reloadWorkspaceConversations();
      updateConfig({
        workspace: nextWorkspaceConfig,
      });
      setWorkspaceFeedback({
        type: 'success',
        message:
          nextRootDir.length > 0
            ? 'Workspace directory updated.'
            : 'Switched to default workspace directory.',
      });
    } catch (error) {
      setWorkspaceFeedback({
        type: 'error',
        message: getErrorMessage(error),
      });
    } finally {
      setIsApplyingWorkspace(false);
    }
  }, [
    config,
    reloadWorkspaceConversations,
    streamingTaskId,
    updateConfig,
    workspaceChanged,
    workspaceDraftNormalized,
  ]);

  const handleUseDefaultWorkspace = useCallback(() => {
    setWorkspaceDraft('');
    setWorkspaceFeedback(null);
  }, []);

  const isStreaming = streamingTaskId !== null;
  return (
    <div className="app-shell">
      <div className="app-canvas">
        {isCompactLayout && sidebarOpen && (
          <button
            type="button"
            aria-label="Close sidebar"
            className="app-overlay"
            onClick={() => setSidebarOpen(false)}
          />
        )}

        {isCompactLayout && protocolOpen && (
          <button
            type="button"
            aria-label="Close protocol panel"
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
            onRenameConversation={renameConversation}
            onToggleConversationPinned={toggleConversationPinned}
            onDeleteConversation={deleteConversation}
            onExportConversation={exportConversation}
            onClose={() => setSidebarOpen(false)}
          />
        </aside>

        <main className="app-main">
          <header className="topbar">
            <div className="topbar-main">
              <div className="topbar-left">
                <button
                  type="button"
                  onClick={() => setSidebarOpen((value) => !value)}
                  className={`icon-btn ${sidebarOpen ? 'is-active' : ''}`}
                  title={sidebarOpen ? 'Hide sidebar' : 'Show sidebar'}
                  aria-label={sidebarOpen ? 'Hide sidebar' : 'Show sidebar'}
                >
                  <Menu size={20} />
                </button>

                <div className="topbar-meta">
                  <h1 className="topbar-title">{currentConversation?.title || 'AI Desktop Assistant'}</h1>
                  <p className="topbar-subtitle">
                    {isStreaming
                      ? 'Streaming response with protocol events...'
                      : 'Chat on the left, protocol timeline on the right'}
                  </p>

                  <div className="workspace-inline">
                    <span className="workspace-inline-label">Workspace</span>
                    <code className="workspace-inline-path" title={workspaceDisplayText(appliedWorkspaceRoot)}>
                      {workspaceDisplayText(appliedWorkspaceRoot)}
                    </code>
                    <button
                      type="button"
                      className="workspace-inline-toggle"
                      onClick={() => setWorkspaceEditorOpen((value) => !value)}
                    >
                      {workspaceEditorOpen ? '收起' : '修改目录'}
                    </button>
                  </div>
                </div>
              </div>

              <div className="topbar-actions">
                <span className={`status-pill ${isStreaming ? 'live' : ''}`}>
                  {isStreaming ? 'Running' : 'Ready'}
                </span>

                <button
                  type="button"
                  onClick={() => setProtocolOpen((value) => !value)}
                  className={`icon-btn ${protocolOpen ? 'is-active' : ''}`}
                  title={protocolOpen ? 'Hide protocol panel' : 'Show protocol panel'}
                  aria-label={protocolOpen ? 'Hide protocol panel' : 'Show protocol panel'}
                >
                  <PanelRight size={20} />
                </button>

                <button
                  type="button"
                  onClick={() => setSettingsOpen(true)}
                  className="icon-btn"
                  title="Settings"
                  aria-label="Open settings"
                >
                  <Settings size={20} />
                </button>
              </div>
            </div>

            {workspaceEditorOpen && (
              <div className="workspace-editor-panel">
                <div className="workspace-editor-controls">
                  <input
                    type="text"
                    className="workspace-editor-input"
                    value={workspaceDraft}
                    onChange={(event) => {
                      setWorkspaceDraft(event.target.value);
                      setWorkspaceFeedback(null);
                    }}
                    onKeyDown={(event) => {
                      if (event.key === 'Enter') {
                        event.preventDefault();
                        void handleApplyWorkspace();
                      }
                    }}
                    placeholder={`留空使用默认目录（${DEFAULT_WORKSPACE_DISPLAY}）`}
                    disabled={isStreaming || isApplyingWorkspace}
                  />

                  <div className="workspace-editor-actions">
                    <button
                      type="button"
                      className="btn-ghost"
                      onClick={() => void handlePickWorkspaceDirectory()}
                      disabled={isStreaming || isApplyingWorkspace}
                    >
                      <FolderOpen size={16} />选择目录
                    </button>
                    <button
                      type="button"
                      className="btn-ghost"
                      onClick={handleUseDefaultWorkspace}
                      disabled={isStreaming || isApplyingWorkspace}
                    >
                      使用默认
                    </button>
                    <button
                      type="button"
                      className="btn-primary"
                      onClick={() => void handleApplyWorkspace()}
                      disabled={isStreaming || isApplyingWorkspace || !workspaceChanged}
                    >
                      <Save size={16} />
                      {isApplyingWorkspace ? '应用中...' : '应用'}
                    </button>
                  </div>
                </div>

                <p className="workspace-editor-note">
                  运行时文件会写入 {'<workspace>/.ah'}（config、screenshots、tmp、image-recognition）。
                </p>
                {workspaceFeedback && (
                  <p className={`workspace-editor-status ${workspaceFeedback.type}`}>{workspaceFeedback.message}</p>
                )}
              </div>
            )}
          </header>

          <div className="workspace">
            <section className="chat-column">
              <MessageList
                messages={currentConversation?.messages || []}
                protocolCards={currentConversation?.protocolCards || []}
                streamingTaskId={streamingTaskId}
                activeThinkTaskId={activeThinkTaskId}
                onRegenerateMessage={handleRegenerateMessage}
                onContinueFromMessage={handleContinueFromMessage}
              />
              <ChatInput
                onSend={handleSendMessage}
                disabled={!isHydrated || isStreaming}
                isStreaming={isStreaming}
                onCancel={handleCancelStream}
                draftInput={draftInput}
                onDraftConsumed={() => setDraftInput(null)}
              />
            </section>

            <section className={`protocol-column ${protocolOpen ? 'open' : ''}`}>
              <ProtocolPanel
                cards={currentConversation?.protocolCards || []}
                onRetry={handleRetryCard}
                onResolveConfigChangeRequest={handleResolveConfigChangeRequest}
              />
            </section>
          </div>
        </main>
      </div>

      <SettingsModal isOpen={settingsOpen} onClose={() => setSettingsOpen(false)} />
    </div>
  );
};

export default ChatView;

