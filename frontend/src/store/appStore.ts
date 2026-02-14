import { v4 as uuidv4 } from 'uuid';
import { create } from 'zustand';
import { logger } from '../lib/logger';
import { TauriAPI } from '../lib/tauri';
import type { AppConfig, Conversation, Message } from '../types';
import { AgentProvider } from '../types';

type NewMessage = Omit<Message, 'id' | 'timestamp'>;

interface ActiveStream {
  taskId: string;
  conversationId: string;
  assistantMessageId: string;
}

interface AppState {
  config: AppConfig;
  updateConfig: (config: Partial<AppConfig>) => void;

  conversations: Conversation[];
  currentConversationId: string | null;
  setCurrentConversation: (id: string | null) => void;
  createConversation: () => string;
  ensureConversation: () => string;
  deleteConversation: (id: string) => void;
  clearConversation: (id: string) => void;

  addMessage: (conversationId: string, message: NewMessage) => string | null;
  updateMessage: (conversationId: string, messageId: string, content: string) => void;

  activeStream: ActiveStream | null;
  streamingTaskId: string | null;
  lastError: string | null;
  beginStream: (conversationId: string, taskId: string) => string | null;
  appendStreamDelta: (taskId: string, chunk: string) => void;
  completeStream: (taskId: string, output: string) => void;
  failStream: (taskId: string, message: string) => void;
  cancelStream: (taskId: string) => void;

  isHydrated: boolean;
  hydrate: () => Promise<void>;

  exportConversation: (conversationId: string) => void;
}

const defaultConfig: AppConfig = {
  provider: AgentProvider.OpenAi,
  model: 'gpt-4o-mini',
  apiKeyEnv: 'OPENAI_API_KEY',
  systemPrompt: 'You are a helpful AI assistant.',
};

const DEFAULT_CONVERSATION_TITLE = '新对话';
const MAX_TITLE_LENGTH = 50;

const deriveConversationTitle = (content: string): string => {
  const text = content.trim();
  if (!text) {
    return DEFAULT_CONVERSATION_TITLE;
  }
  if (text.length <= MAX_TITLE_LENGTH) {
    return text;
  }
  return `${text.slice(0, MAX_TITLE_LENGTH)}...`;
};

const normalizeConversation = (conversation: Conversation): Conversation => ({
  ...conversation,
  messages: conversation.messages.map((message) => ({
    ...message,
    isStreaming: false,
  })),
});

const findConversation = (conversations: Conversation[], id: string | null) =>
  id ? conversations.find((conversation) => conversation.id === id) : undefined;

export const useAppStore = create<AppState>((set, get) => ({
  config: defaultConfig,
  updateConfig: (newConfig) => {
    set((state) => ({
      config: { ...state.config, ...newConfig },
    }));
  },

  conversations: [],
  currentConversationId: null,
  setCurrentConversation: (id) => {
    set((state) => {
      if (!id) {
        return { currentConversationId: null };
      }
      const conversationExists = state.conversations.some((conversation) => conversation.id === id);
      if (!conversationExists) {
        return state;
      }
      return { currentConversationId: id };
    });
  },
  createConversation: () => {
    const id = uuidv4();
    const now = Date.now();
    const newConversation: Conversation = {
      id,
      title: DEFAULT_CONVERSATION_TITLE,
      messages: [],
      createdAt: now,
      updatedAt: now,
    };
    set((state) => ({
      conversations: [newConversation, ...state.conversations],
      currentConversationId: id,
      lastError: null,
    }));
    return id;
  },
  ensureConversation: () => {
    const state = get();
    const currentConversation = findConversation(state.conversations, state.currentConversationId);
    if (currentConversation) {
      return currentConversation.id;
    }
    return state.createConversation();
  },
  deleteConversation: (id) => {
    set((state) => {
      const remainingConversations = state.conversations.filter((conversation) => conversation.id !== id);
      const nextCurrentConversationId =
        state.currentConversationId === id
          ? (remainingConversations[0]?.id ?? null)
          : state.currentConversationId;

      const shouldResetStream = state.activeStream?.conversationId === id;

      return {
        conversations: remainingConversations,
        currentConversationId: nextCurrentConversationId,
        activeStream: shouldResetStream ? null : state.activeStream,
        streamingTaskId: shouldResetStream ? null : state.streamingTaskId,
      };
    });
  },
  clearConversation: (conversationId) => {
    set((state) => {
      const shouldResetStream = state.activeStream?.conversationId === conversationId;
      return {
        conversations: state.conversations.map((conversation) => {
          if (conversation.id !== conversationId) {
            return conversation;
          }
          return {
            ...conversation,
            messages: [],
            updatedAt: Date.now(),
            title: DEFAULT_CONVERSATION_TITLE,
          };
        }),
        activeStream: shouldResetStream ? null : state.activeStream,
        streamingTaskId: shouldResetStream ? null : state.streamingTaskId,
      };
    });
  },

  addMessage: (conversationId, message) => {
    const messageId = uuidv4();
    const now = Date.now();
    let added = false;

    set((state) => ({
      conversations: state.conversations.map((conversation) => {
        if (conversation.id !== conversationId) {
          return conversation;
        }
        added = true;
        const nextMessage: Message = {
          ...message,
          id: messageId,
          timestamp: now,
        };
        const shouldUpdateTitle = message.role === 'user' && conversation.messages.length === 0;
        return {
          ...conversation,
          title: shouldUpdateTitle ? deriveConversationTitle(message.content) : conversation.title,
          messages: [...conversation.messages, nextMessage],
          updatedAt: now,
        };
      }),
    }));

    return added ? messageId : null;
  },
  updateMessage: (conversationId, messageId, content) => {
    set((state) => ({
      conversations: state.conversations.map((conversation) => {
        if (conversation.id !== conversationId) {
          return conversation;
        }
        return {
          ...conversation,
          messages: conversation.messages.map((message) =>
            message.id === messageId ? { ...message, content } : message
          ),
          updatedAt: Date.now(),
        };
      }),
    }));
  },

  activeStream: null,
  streamingTaskId: null,
  lastError: null,
  beginStream: (conversationId, taskId) => {
    const assistantMessageId = uuidv4();
    const now = Date.now();
    let initialized = false;

    set((state) => ({
      conversations: state.conversations.map((conversation) => {
        if (conversation.id !== conversationId) {
          return conversation;
        }
        initialized = true;
        const streamMessage: Message = {
          id: assistantMessageId,
          role: 'assistant',
          content: '',
          timestamp: now,
          isStreaming: true,
        };
        return {
          ...conversation,
          messages: [...conversation.messages, streamMessage],
          updatedAt: now,
        };
      }),
      activeStream: initialized
        ? {
            taskId,
            conversationId,
            assistantMessageId,
          }
        : null,
      streamingTaskId: initialized ? taskId : null,
      lastError: null,
    }));

    return initialized ? assistantMessageId : null;
  },
  appendStreamDelta: (taskId, chunk) => {
    if (!chunk) {
      return;
    }
    set((state) => {
      const stream = state.activeStream;
      if (!stream || stream.taskId !== taskId) {
        return state;
      }

      return {
        conversations: state.conversations.map((conversation) => {
          if (conversation.id !== stream.conversationId) {
            return conversation;
          }
          return {
            ...conversation,
            messages: conversation.messages.map((message) => {
              if (message.id !== stream.assistantMessageId) {
                return message;
              }
              return {
                ...message,
                content: `${message.content}${chunk}`,
                isStreaming: true,
              };
            }),
            updatedAt: Date.now(),
          };
        }),
      };
    });
  },
  completeStream: (taskId, output) => {
    set((state) => {
      const stream = state.activeStream;
      if (!stream || stream.taskId !== taskId) {
        return state;
      }
      return {
        conversations: state.conversations.map((conversation) => {
          if (conversation.id !== stream.conversationId) {
            return conversation;
          }
          return {
            ...conversation,
            messages: conversation.messages.map((message) => {
              if (message.id !== stream.assistantMessageId) {
                return message;
              }
              return {
                ...message,
                content: output || message.content,
                isStreaming: false,
              };
            }),
            updatedAt: Date.now(),
          };
        }),
        activeStream: null,
        streamingTaskId: null,
      };
    });
  },
  failStream: (taskId, message) => {
    set((state) => {
      const stream = state.activeStream;
      if (!stream || stream.taskId !== taskId) {
        return {
          lastError: message,
        };
      }
      return {
        conversations: state.conversations.map((conversation) => {
          if (conversation.id !== stream.conversationId) {
            return conversation;
          }
          return {
            ...conversation,
            messages: conversation.messages.map((item) => {
              if (item.id !== stream.assistantMessageId) {
                return item;
              }
              if (item.content.trim().length > 0) {
                return { ...item, isStreaming: false };
              }
              return {
                ...item,
                content: `请求失败：${message}`,
                isStreaming: false,
              };
            }),
            updatedAt: Date.now(),
          };
        }),
        activeStream: null,
        streamingTaskId: null,
        lastError: message,
      };
    });
  },
  cancelStream: (taskId) => {
    set((state) => {
      const stream = state.activeStream;
      if (!stream || stream.taskId !== taskId) {
        return state;
      }
      return {
        conversations: state.conversations.map((conversation) => {
          if (conversation.id !== stream.conversationId) {
            return conversation;
          }
          return {
            ...conversation,
            messages: conversation.messages.map((item) => {
              if (item.id !== stream.assistantMessageId) {
                return item;
              }
              if (item.content.trim().length > 0) {
                return {
                  ...item,
                  isStreaming: false,
                };
              }
              return {
                ...item,
                content: '[已取消]',
                isStreaming: false,
              };
            }),
            updatedAt: Date.now(),
          };
        }),
        activeStream: null,
        streamingTaskId: null,
      };
    });
  },

  isHydrated: false,
  hydrate: async () => {
    if (get().isHydrated) {
      return;
    }

    try {
      const [storedConfig, storedConversations, storedCurrentConversationId] = await Promise.all([
        TauriAPI.getStore<Partial<AppConfig>>('config', {}),
        TauriAPI.getStore<Conversation[]>('conversations', []),
        TauriAPI.getStore<string | null>('currentConversationId', null),
      ]);

      const conversations = Array.isArray(storedConversations)
        ? storedConversations.map(normalizeConversation)
        : [];

      const hasCurrentConversation = conversations.some(
        (conversation) => conversation.id === storedCurrentConversationId
      );

      set({
        config: { ...defaultConfig, ...storedConfig },
        conversations,
        currentConversationId: hasCurrentConversation
          ? storedCurrentConversationId
          : (conversations[0]?.id ?? null),
        activeStream: null,
        streamingTaskId: null,
        isHydrated: true,
      });
    } catch (error) {
      logger.error('Failed to hydrate app store', { error });
      set({ isHydrated: true });
    }
  },

  exportConversation: (conversationId) => {
    const conversation = get().conversations.find((item) => item.id === conversationId);
    if (!conversation) {
      return;
    }

    const data = JSON.stringify(conversation, null, 2);
    const blob = new Blob([data], { type: 'application/json' });
    const url = URL.createObjectURL(blob);
    const link = document.createElement('a');
    link.href = url;
    link.download = `conversation-${conversationId}.json`;
    link.click();
    URL.revokeObjectURL(url);
  },
}));

export const saveToTauriStore = (
  config: AppConfig,
  conversations: Conversation[],
  currentConversationId: string | null
) => {
  void TauriAPI.setStore('config', config);
  void TauriAPI.setStore('conversations', conversations);
  if (currentConversationId) {
    void TauriAPI.setStore('currentConversationId', currentConversationId);
  } else {
    void TauriAPI.deleteStore('currentConversationId');
  }
};

useAppStore.subscribe((state) => {
  if (!state.isHydrated) {
    return;
  }
  saveToTauriStore(state.config, state.conversations, state.currentConversationId);
});
