import { v4 as uuidv4 } from 'uuid';
import { create } from 'zustand';
import { createDefaultConfig, normalizeStoredConfig } from '../config/providers';
import { logger } from '../lib/logger';
import { TauriAPI } from '../lib/tauri';
import type {
  AppConfig,
  Conversation,
  InputCard,
  LegacySessionSnapshot,
  Message,
  ProtocolCard,
  ProtocolCardLevel,
  ProtocolEventPayload,
  ProtocolOpPayload,
  ProtocolRuntimeConfigPatch,
} from '../types';

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
  reloadWorkspaceConversations: () => Promise<void>;

  exportConversation: (conversationId: string) => void;
}

const defaultConfig: AppConfig = createDefaultConfig();
const CONFIG_STORE_KEY = 'config';
const LEGACY_CONVERSATIONS_STORE_KEY = 'conversations';
const LEGACY_CURRENT_CONVERSATION_STORE_KEY = 'currentConversationId';

const DEFAULT_CONVERSATION_TITLE = '新对话';
const MAX_TITLE_LENGTH = 50;

const findConversation = (conversations: Conversation[], id: string | null) =>
  id ? conversations.find((conversation) => conversation.id === id) : undefined;

const toInputCardFromOpPayload = (payload: ProtocolOpPayload): InputCard | undefined => {
  if (payload.type === 'user_turn') {
    return {
      content: payload.text,
    };
  }
  if (payload.type === 'run_user_shell_command') {
    return {
      content: `/${payload.command}`,
    };
  }
  return undefined;
};

const summarizeOpPayload = (payload: ProtocolOpPayload): string => {
  switch (payload.type) {
    case 'user_turn':
      return `UserTurn | ${payload.text.slice(0, 80)}`;
    case 'run_user_shell_command':
      return `RunUserShellCommand | ${payload.command}`;
    case 'interrupt':
      return 'Interrupt';
  }
  return 'UnknownOp';
};

const summarizeEventPayload = (payload: ProtocolEventPayload): string => {
  switch (payload.type) {
    case 'turn_started':
      return `TurnStarted | ${payload.turn_id}`;
    case 'model_streaming':
      return `ModelStreaming | ${payload.chunk.slice(0, 80)}`;
    case 'model_complete':
      return `ModelComplete | ${payload.usage.total_tokens} tokens`;
    case 'tool_call_requested': {
      const rewrittenCount = payload.normalization?.rewrittenCount ?? 0;
      const rejectReason = payload.normalization?.rejectPreview?.reason;
      if (rejectReason) {
        return `ToolCallRequested | ${payload.tool} | reject: ${rejectReason}`;
      }
      if (rewrittenCount > 0) {
        return `ToolCallRequested | ${payload.tool} | rewrites:${rewrittenCount}`;
      }
      return `ToolCallRequested | ${payload.tool}`;
    }
    case 'tool_call_result':
      return `ToolCallResult | ${payload.tool}`;
    case 'run_user_shell_command':
      return `RunUserShellCommand | ${payload.command}`;
    case 'warning':
      return `Warning | ${payload.message}`;
    case 'error':
      return `Error | ${payload.message}`;
    case 'turn_aborted':
      return `TurnAborted | ${payload.reason}`;
    case 'turn_complete':
      return 'TurnComplete';
    case 'mcp_list_tools_response':
      return `McpListToolsResponse | ${payload.tools.length} tools`;
    case 'mcp_list_resources_response':
      return `McpListResourcesResponse | ${payload.resources.length} resources`;
    case 'mcp_resource_content':
      return `McpResourceContent | ${payload.uri}`;
    case 'mcp_list_prompts_response':
      return `McpListPromptsResponse | ${payload.prompts.length} prompts`;
    case 'mcp_prompt_result':
      return `McpPromptResult | ${payload.messages.length} messages`;
    case 'list_skills_response':
      return `ListSkillsResponse | ${payload.skills.length} skills`;
    case 'skill_content':
      return `SkillContent | ${payload.name}`;
    case 'skill_applied':
      return `SkillApplied | ${payload.name}`;
    case 'skill_file_content':
      return `SkillFileContent | ${payload.skill_name}/${payload.file_path}`;
    case 'governance_report':
      return `GovernanceReport | B:${payload.report.blockerCount} W:${payload.report.warningCount} I:${payload.report.infoCount}`;
    case 'guidance_context':
      return `GuidanceContext | contracts:${payload.input.mcpContractsTotal} servers:${payload.input.mcpServersTotal} skills:${payload.input.skillsTotal}`;
    case 'conversation_title_suggestion':
      return `ConversationTitleSuggestion | ${payload.title}`;
    case 'control_decision':
      return `ControlDecision | ${payload.source} | ${payload.summary}`;
    case 'config_change_request':
      return `ConfigChangeRequest | ${payload.summary}`;
    case 'config_change_result':
      return `ConfigChangeResult | ${payload.approved ? 'approved' : 'rejected'} | ${payload.reason}`;
  }
  return 'UnknownEvent';
};

const levelFromEventPayload = (payload: ProtocolEventPayload): ProtocolCardLevel => {
  switch (payload.type) {
    case 'error':
    case 'turn_aborted':
      return 'error';
    case 'tool_call_requested':
      return payload.normalization?.rejectPreview ? 'warning' : 'info';
    case 'warning':
      return 'warning';
    case 'model_complete':
    case 'turn_complete':
    case 'tool_call_result':
    case 'mcp_list_tools_response':
    case 'mcp_list_resources_response':
    case 'mcp_resource_content':
    case 'mcp_list_prompts_response':
    case 'mcp_prompt_result':
    case 'list_skills_response':
    case 'skill_content':
    case 'skill_applied':
    case 'skill_file_content':
    case 'conversation_title_suggestion':
      return 'success';
    case 'governance_report':
      if (payload.report.blockerCount > 0) {
        return 'error';
      }
      if (payload.report.warningCount > 0) {
        return 'warning';
      }
      return 'success';
    case 'guidance_context':
      return 'info';
    case 'config_change_request':
      return 'warning';
    case 'config_change_result':
      return payload.approved ? 'success' : 'warning';
    default:
      return 'info';
  }
};

const mergePersistedPatchIntoConfig = (
  config: AppConfig,
  patch: ProtocolRuntimeConfigPatch
): AppConfig => {
  const next: AppConfig = { ...config };

  if (typeof patch.systemPrompt === 'string' && patch.systemPrompt.trim().length > 0) {
    next.systemPrompt = patch.systemPrompt;
  }
  if (patch.mcp && typeof patch.mcp === 'object') {
    next.mcp = {
      ...next.mcp,
      ...(patch.mcp as unknown as AppConfig['mcp']),
    };
  }
  if (patch.skills && typeof patch.skills === 'object') {
    next.skills = {
      ...next.skills,
      ...(patch.skills as unknown as AppConfig['skills']),
    };
  }

  return next;
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

const readLegacySessionSnapshot = async (): Promise<LegacySessionSnapshot | null> => {
  const [storedConversations, storedCurrentConversationId] = await Promise.all([
    TauriAPI.getStore<Conversation[]>(LEGACY_CONVERSATIONS_STORE_KEY, []),
    TauriAPI.getStore<string | null>(LEGACY_CURRENT_CONVERSATION_STORE_KEY, null),
  ]);
  const conversations = Array.isArray(storedConversations)
    ? storedConversations.map(normalizeConversation)
    : [];
  if (conversations.length === 0 && !storedCurrentConversationId) {
    return null;
  }
  return {
    conversations,
    currentConversationId: storedCurrentConversationId,
  };
};

const clearLegacyConversationStore = async (): Promise<void> => {
  await Promise.all([
    TauriAPI.deleteStore(LEGACY_CONVERSATIONS_STORE_KEY),
    TauriAPI.deleteStore(LEGACY_CURRENT_CONVERSATION_STORE_KEY),
  ]);
};

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
        return {
          ...conversation,
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
        config:
          payload.type === 'config_change_result' &&
          payload.approved &&
          payload.persisted &&
          payload.applied &&
          payload.patch
            ? mergePersistedPatchIntoConfig(state.config, payload.patch)
            : state.config,
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
      const storedConfig = await TauriAPI.getStore<Partial<AppConfig>>(CONFIG_STORE_KEY, {});

      const normalizedStoredConfig = normalizeStoredConfig(storedConfig);
      const hydratedConfig: AppConfig = {
        ...createDefaultConfig(normalizedStoredConfig.provider ?? defaultConfig.provider),
        ...normalizedStoredConfig,
      };

      try {
        await TauriAPI.updateRuntimeConfig(hydratedConfig);
      } catch (error) {
        logger.error('Failed to sync runtime config before storage bootstrap', { error });
      }

      const legacySnapshot = await readLegacySessionSnapshot();
      const bootstrapResponse = await TauriAPI.storageBootstrap(
        legacySnapshot
          ? {
              legacy: legacySnapshot,
            }
          : undefined
      );

      const conversations = Array.isArray(bootstrapResponse.conversations)
        ? bootstrapResponse.conversations.map(normalizeConversation)
        : [];
      const hasCurrentConversation = conversations.some(
        (conversation) => conversation.id === bootstrapResponse.currentConversationId
      );
      const resolvedCurrentConversationId = hasCurrentConversation
        ? (bootstrapResponse.currentConversationId ?? null)
        : (conversations[0]?.id ?? null);

      if (legacySnapshot) {
        await clearLegacyConversationStore();
      }

      lastPersistedConversationUpdatedAt = new Map(
        conversations.map((conversation) => [conversation.id, conversation.updatedAt])
      );
      lastPersistedCurrentConversationId = resolvedCurrentConversationId;

      set({
        config: hydratedConfig,
        conversations,
        currentConversationId: resolvedCurrentConversationId,
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
  reloadWorkspaceConversations: async () => {
    try {
      const bootstrapResponse = await TauriAPI.storageBootstrap();
      const conversations = Array.isArray(bootstrapResponse.conversations)
        ? bootstrapResponse.conversations.map(normalizeConversation)
        : [];
      const hasCurrentConversation = conversations.some(
        (conversation) => conversation.id === bootstrapResponse.currentConversationId
      );
      const resolvedCurrentConversationId = hasCurrentConversation
        ? (bootstrapResponse.currentConversationId ?? null)
        : (conversations[0]?.id ?? null);
      lastPersistedConversationUpdatedAt = new Map(
        conversations.map((conversation) => [conversation.id, conversation.updatedAt])
      );
      lastPersistedCurrentConversationId = resolvedCurrentConversationId;
      set({
        conversations,
        currentConversationId: resolvedCurrentConversationId,
        activeStream: null,
        activeProtocolStreamingCard: null,
        streamingTaskId: null,
        lastError: null,
      });
    } catch (error) {
      logger.error('Failed to reload workspace conversations', { error });
      throw error;
    }
  },

  exportConversation: (conversationId) => {
    void (async () => {
      try {
        const conversation = await TauriAPI.storageExportConversation(conversationId);
        const data = JSON.stringify(conversation, null, 2);
        const blob = new Blob([data], { type: 'application/json' });
        const url = URL.createObjectURL(blob);
        const link = document.createElement('a');
        link.href = url;
        link.download = `conversation-${conversationId}.json`;
        link.click();
        URL.revokeObjectURL(url);
      } catch (error) {
        logger.error('Failed to export conversation from storage', {
          error,
          conversationId,
        });
      }
    })();
  },
}));

const PERSIST_DEBOUNCE_MS = 250;
const PERSIST_STREAMING_DEBOUNCE_MS = 1200;

interface PersistSnapshot {
  config: AppConfig;
  conversations: Conversation[];
  currentConversationId: string | null;
}

let persistTimer: ReturnType<typeof setTimeout> | null = null;
let pendingPersistSnapshot: PersistSnapshot | null = null;
let pendingStorageSnapshot: PersistSnapshot | null = null;
let storageSyncInFlight = false;
let lastPersistedConversationUpdatedAt = new Map<string, number>();
let lastPersistedCurrentConversationId: string | null = null;

const syncSnapshotToStorage = async (snapshot: PersistSnapshot) => {
  try {
    await TauriAPI.setStore(CONFIG_STORE_KEY, snapshot.config);
  } catch (error) {
    logger.error('Failed to persist config', { error });
  }

  const nextConversationIds = new Set(snapshot.conversations.map((conversation) => conversation.id));

  for (const conversationId of lastPersistedConversationUpdatedAt.keys()) {
    if (nextConversationIds.has(conversationId)) {
      continue;
    }
    try {
      await TauriAPI.storageDeleteConversation(conversationId);
      lastPersistedConversationUpdatedAt.delete(conversationId);
    } catch (error) {
      logger.error('Failed to delete conversation from storage', {
        error,
        conversationId,
      });
    }
  }

  for (const conversation of snapshot.conversations) {
    const previousUpdatedAt = lastPersistedConversationUpdatedAt.get(conversation.id);
    if (previousUpdatedAt === conversation.updatedAt) {
      continue;
    }
    try {
      await TauriAPI.storageUpsertConversation(conversation);
      lastPersistedConversationUpdatedAt.set(conversation.id, conversation.updatedAt);
    } catch (error) {
      logger.error('Failed to upsert conversation into storage', {
        error,
        conversationId: conversation.id,
      });
    }
  }

  if (lastPersistedCurrentConversationId !== snapshot.currentConversationId) {
    try {
      await TauriAPI.storageSetCurrentConversation(snapshot.currentConversationId);
      lastPersistedCurrentConversationId = snapshot.currentConversationId;
    } catch (error) {
      logger.error('Failed to persist current conversation id', { error });
    }
  }
};

const flushPendingStorageSnapshot = () => {
  if (storageSyncInFlight) {
    return;
  }
  storageSyncInFlight = true;

  void (async () => {
    while (pendingStorageSnapshot) {
      const nextSnapshot = pendingStorageSnapshot;
      pendingStorageSnapshot = null;
      await syncSnapshotToStorage(nextSnapshot);
    }
    storageSyncInFlight = false;
    if (pendingStorageSnapshot) {
      flushPendingStorageSnapshot();
    }
  })();
};

const schedulePersistToTauriStore = (snapshot: PersistSnapshot, delayMs: number) => {
  pendingPersistSnapshot = snapshot;

  if (persistTimer !== null) {
    clearTimeout(persistTimer);
  }

  persistTimer = setTimeout(() => {
    persistTimer = null;
    const nextSnapshot = pendingPersistSnapshot;
    if (!nextSnapshot) {
      return;
    }
    pendingPersistSnapshot = null;
    pendingStorageSnapshot = nextSnapshot;
    flushPendingStorageSnapshot();
  }, delayMs);
};

useAppStore.subscribe((state) => {
  if (!state.isHydrated) {
    return;
  }

  const delayMs = state.streamingTaskId ? PERSIST_STREAMING_DEBOUNCE_MS : PERSIST_DEBOUNCE_MS;
  schedulePersistToTauriStore(
    {
      config: state.config,
      conversations: state.conversations,
      currentConversationId: state.currentConversationId,
    },
    delayMs
  );
});
