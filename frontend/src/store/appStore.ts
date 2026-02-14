import { v4 as uuidv4 } from 'uuid';
import { create } from 'zustand';
import { createDefaultConfig, normalizeStoredConfig } from '../config/providers';
import { logger } from '../lib/logger';
import { TauriAPI } from '../lib/tauri';
import type {
  AppConfig,
  Conversation,
  InputCard,
  Message,
  ProtocolCard,
  ProtocolCardLevel,
  ProtocolEventPayload,
  ProtocolOpPayload,
} from '../types';
import { InputCardKind } from '../types';

type NewMessage = Omit<Message, 'id' | 'timestamp'>;

interface ActiveStream {
  taskId: string;
  conversationId: string;
  assistantMessageId: string;
  inputCard: InputCard;
}

interface ActiveProtocolStreamingCard {
  taskId: string;
  conversationId: string;
  cardId: string;
}

interface AppState {
  config: AppConfig;
  updateConfig: (config: Partial<AppConfig>) => void;

  conversations: Conversation[];
  currentConversationId: string | null;
  setCurrentConversation: (id: string | null) => void;
  createConversation: () => string;
  ensureConversation: () => string;
  renameConversation: (id: string, title: string) => void;
  toggleConversationPinned: (id: string) => void;
  deleteConversation: (id: string) => void;
  clearConversation: (id: string) => void;

  addMessage: (conversationId: string, message: NewMessage) => string | null;
  updateMessage: (conversationId: string, messageId: string, content: string) => void;

  addOpCard: (taskId: string, seq: number, payload: ProtocolOpPayload) => string | null;
  addProtocolEventCard: (taskId: string, seq: number, payload: ProtocolEventPayload) => string | null;
  appendStreamingEventChunk: (taskId: string, seq: number, chunk: string) => void;
  retryFromCard: (conversationId: string, cardId: string) => InputCard | null;

  activeStream: ActiveStream | null;
  activeProtocolStreamingCard: ActiveProtocolStreamingCard | null;
  streamingTaskId: string | null;
  lastError: string | null;
  beginStream: (conversationId: string, taskId: string, inputCard: InputCard) => string | null;
  appendStreamDelta: (taskId: string, chunk: string) => void;
  completeStream: (taskId: string, output: string) => void;
  failStream: (taskId: string, message: string) => void;
  cancelStream: (taskId: string) => void;

  isHydrated: boolean;
  hydrate: () => Promise<void>;

  exportConversation: (conversationId: string) => void;
}

const defaultConfig: AppConfig = createDefaultConfig();
const STORAGE_SCHEMA_VERSION = 2;

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

const findConversation = (conversations: Conversation[], id: string | null) =>
  id ? conversations.find((conversation) => conversation.id === id) : undefined;

const toInputCardFromOpPayload = (payload: ProtocolOpPayload): InputCard | undefined => {
  if (payload.type === 'user_turn') {
    return {
      kind: InputCardKind.Text,
      content: payload.text,
    };
  }
  if (payload.type === 'run_user_shell_command') {
    return {
      kind: InputCardKind.Command,
      content: payload.command,
    };
  }
  return undefined;
};

const summarizeOpPayload = (payload: ProtocolOpPayload): string => {
  switch (payload.type) {
    case 'user_turn':
      return `UserTurn · ${payload.text.slice(0, 80)}`;
    case 'run_user_shell_command':
      return `RunUserShellCommand · ${payload.command}`;
    case 'interrupt':
      return 'Interrupt';
  }
  return 'UnknownOp';
};

const summarizeEventPayload = (payload: ProtocolEventPayload): string => {
  switch (payload.type) {
    case 'turn_started':
      return `TurnStarted · ${payload.turn_id}`;
    case 'model_streaming':
      return `ModelStreaming · ${payload.chunk.slice(0, 80)}`;
    case 'model_complete':
      return `ModelComplete · ${payload.usage.total_tokens} tokens`;
    case 'tool_call_requested':
      return `ToolCallRequested · ${payload.tool}`;
    case 'tool_call_result':
      return `ToolCallResult · ${payload.tool}`;
    case 'run_user_shell_command':
      return `RunUserShellCommand · ${payload.command}`;
    case 'warning':
      return `Warning · ${payload.message}`;
    case 'error':
      return `Error · ${payload.message}`;
    case 'turn_aborted':
      return `TurnAborted · ${payload.reason}`;
    case 'turn_complete':
      return 'TurnComplete';
  }
  return 'UnknownEvent';
};

const levelFromEventPayload = (payload: ProtocolEventPayload): ProtocolCardLevel => {
  switch (payload.type) {
    case 'error':
    case 'turn_aborted':
      return 'error';
    case 'warning':
      return 'warning';
    case 'model_complete':
    case 'turn_complete':
    case 'tool_call_result':
      return 'success';
    default:
      return 'info';
  }
};

const isTerminalEvent = (payload: ProtocolEventPayload): boolean =>
  payload.type === 'model_complete' ||
  payload.type === 'turn_complete' ||
  payload.type === 'turn_aborted' ||
  payload.type === 'error';

const normalizeConversation = (conversation: Conversation): Conversation => ({
  ...conversation,
  pinned: Boolean(conversation.pinned),
  messages: Array.isArray(conversation.messages)
    ? conversation.messages.map((message) => ({
        ...message,
        isStreaming: false,
      }))
    : [],
  protocolCards: Array.isArray(conversation.protocolCards)
    ? conversation.protocolCards.map((card) => ({
        ...card,
        streaming: false,
      }))
    : [],
});

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
      pinned: false,
      messages: [],
      protocolCards: [],
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
  renameConversation: (id, title) => {
    const normalizedTitle = title.trim().slice(0, MAX_TITLE_LENGTH);
    if (!normalizedTitle) {
      return;
    }

    set((state) => ({
      conversations: state.conversations.map((conversation) => {
        if (conversation.id !== id) {
          return conversation;
        }
        return {
          ...conversation,
          title: normalizedTitle,
          updatedAt: Date.now(),
        };
      }),
    }));
  },
  toggleConversationPinned: (id) => {
    set((state) => ({
      conversations: state.conversations.map((conversation) => {
        if (conversation.id !== id) {
          return conversation;
        }
        return {
          ...conversation,
          pinned: !conversation.pinned,
          updatedAt: Date.now(),
        };
      }),
    }));
  },
  deleteConversation: (id) => {
    set((state) => {
      const remainingConversations = state.conversations.filter((conversation) => conversation.id !== id);
      const nextCurrentConversationId =
        state.currentConversationId === id ? (remainingConversations[0]?.id ?? null) : state.currentConversationId;

      const shouldResetStream = state.activeStream?.conversationId === id;

      return {
        conversations: remainingConversations,
        currentConversationId: nextCurrentConversationId,
        activeStream: shouldResetStream ? null : state.activeStream,
        activeProtocolStreamingCard: shouldResetStream ? null : state.activeProtocolStreamingCard,
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
            protocolCards: [],
            updatedAt: Date.now(),
            title: DEFAULT_CONVERSATION_TITLE,
          };
        }),
        activeStream: shouldResetStream ? null : state.activeStream,
        activeProtocolStreamingCard: shouldResetStream ? null : state.activeProtocolStreamingCard,
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
          messages: conversation.messages.map((message) => (message.id === messageId ? { ...message, content } : message)),
          updatedAt: Date.now(),
        };
      }),
    }));
  },

  addOpCard: (taskId, seq, payload) => {
    let cardId: string | null = null;
    set((state) => {
      const stream = state.activeStream;
      if (!stream || stream.taskId !== taskId) {
        return state;
      }

      const now = Date.now();
      const retryInput = toInputCardFromOpPayload(payload);
      const card: ProtocolCard = {
        id: uuidv4(),
        taskId,
        seq,
        direction: 'op',
        type: payload.type,
        payload,
        level: 'info',
        summary: summarizeOpPayload(payload),
        timestamp: now,
        streaming: false,
        retryable: Boolean(retryInput),
        retryInput,
      };
      cardId = card.id;

      return {
        conversations: state.conversations.map((conversation) => {
          if (conversation.id !== stream.conversationId) {
            return conversation;
          }
          return {
            ...conversation,
            protocolCards: [...conversation.protocolCards, card],
            updatedAt: now,
          };
        }),
      };
    });
    return cardId;
  },
  addProtocolEventCard: (taskId, seq, payload) => {
    let cardId: string | null = null;
    set((state) => {
      const stream = state.activeStream;
      if (!stream || stream.taskId !== taskId) {
        return state;
      }

      const now = Date.now();
      const terminalEvent = isTerminalEvent(payload);
      const currentProtocolStream = state.activeProtocolStreamingCard;

      let nextActiveProtocolStream = state.activeProtocolStreamingCard;

      const updateConversation = (conversation: Conversation): Conversation => {
        if (conversation.id !== stream.conversationId) {
          return conversation;
        }

        let protocolCards = conversation.protocolCards.map((card) => {
          if (terminalEvent && currentProtocolStream?.taskId === taskId && card.id === currentProtocolStream.cardId) {
            return { ...card, streaming: false };
          }
          return card;
        });

        if (payload.type === 'model_streaming' && currentProtocolStream?.taskId === taskId) {
          protocolCards = protocolCards.map((card) => {
            if (card.id !== currentProtocolStream.cardId) {
              return card;
            }
            const previousChunk =
              card.payload.type === 'model_streaming' ? card.payload.chunk : '';
            const mergedChunk = `${previousChunk}${payload.chunk}`;
            const mergedPayload: ProtocolEventPayload = {
              type: 'model_streaming',
              chunk: mergedChunk,
            };
            return {
              ...card,
              seq,
              payload: mergedPayload,
              summary: summarizeEventPayload(mergedPayload),
              streaming: true,
            };
          });
          cardId = currentProtocolStream.cardId;
          return {
            ...conversation,
            protocolCards,
            updatedAt: now,
          };
        }

        const retryInput = payload.type === 'error' ? stream.inputCard : undefined;
        const card: ProtocolCard = {
          id: uuidv4(),
          taskId,
          seq,
          direction: 'event',
          type: payload.type,
          payload,
          level: levelFromEventPayload(payload),
          summary: summarizeEventPayload(payload),
          timestamp: now,
          streaming: payload.type === 'model_streaming',
          retryable: payload.type === 'error',
          retryInput,
        };
        cardId = card.id;

        if (payload.type === 'model_streaming') {
          nextActiveProtocolStream = {
            taskId,
            conversationId: stream.conversationId,
            cardId: card.id,
          };
        } else if (terminalEvent) {
          nextActiveProtocolStream = null;
        }

        return {
          ...conversation,
          protocolCards: [...protocolCards, card],
          updatedAt: now,
        };
      };

      return {
        conversations: state.conversations.map(updateConversation),
        activeProtocolStreamingCard: nextActiveProtocolStream,
      };
    });
    return cardId;
  },
  appendStreamingEventChunk: (taskId, seq, chunk) => {
    get().addProtocolEventCard(taskId, seq, {
      type: 'model_streaming',
      chunk,
    });
  },
  retryFromCard: (conversationId, cardId) => {
    const conversation = get().conversations.find((item) => item.id === conversationId);
    if (!conversation) {
      return null;
    }
    const card = conversation.protocolCards.find((item) => item.id === cardId);
    return card?.retryInput ?? null;
  },

  activeStream: null,
  activeProtocolStreamingCard: null,
  streamingTaskId: null,
  lastError: null,
  beginStream: (conversationId, taskId, inputCard) => {
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
            inputCard,
          }
        : null,
      activeProtocolStreamingCard: null,
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
        activeProtocolStreamingCard: null,
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
                content: `请求失败: ${message}`,
                isStreaming: false,
              };
            }),
            updatedAt: Date.now(),
          };
        }),
        activeStream: null,
        activeProtocolStreamingCard: null,
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
        activeProtocolStreamingCard: null,
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
      const [storedConfig, storedSchemaVersion] = await Promise.all([
        TauriAPI.getStore<Partial<AppConfig>>('config', {}),
        TauriAPI.getStore<number>('storage_schema_version', 0),
      ]);

      const normalizedStoredConfig = normalizeStoredConfig(storedConfig);
      const hydratedConfig: AppConfig = {
        ...createDefaultConfig(normalizedStoredConfig.provider ?? defaultConfig.provider),
        ...normalizedStoredConfig,
      };

      if (storedSchemaVersion !== STORAGE_SCHEMA_VERSION) {
        await Promise.all([
          TauriAPI.deleteStore('conversations'),
          TauriAPI.deleteStore('currentConversationId'),
          TauriAPI.setStore('storage_schema_version', STORAGE_SCHEMA_VERSION),
        ]);

        set({
          config: hydratedConfig,
          conversations: [],
          currentConversationId: null,
          activeStream: null,
          activeProtocolStreamingCard: null,
          streamingTaskId: null,
          isHydrated: true,
        });
        return;
      }

      const [storedConversations, storedCurrentConversationId] = await Promise.all([
        TauriAPI.getStore<Conversation[]>('conversations', []),
        TauriAPI.getStore<string | null>('currentConversationId', null),
      ]);

      const conversations = Array.isArray(storedConversations)
        ? storedConversations.map(normalizeConversation)
        : [];

      const hasCurrentConversation = conversations.some((conversation) => conversation.id === storedCurrentConversationId);

      set({
        config: hydratedConfig,
        conversations,
        currentConversationId: hasCurrentConversation ? storedCurrentConversationId : (conversations[0]?.id ?? null),
        activeStream: null,
        activeProtocolStreamingCard: null,
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

export const saveToTauriStore = (config: AppConfig, conversations: Conversation[], currentConversationId: string | null) => {
  void TauriAPI.setStore('storage_schema_version', STORAGE_SCHEMA_VERSION);
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
