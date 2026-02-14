import React, { useMemo, useState } from 'react';
import { Check, Download, MessageSquare, Pencil, Pin, PinOff, Plus, Search, Trash2, X } from 'lucide-react';
import type { Conversation } from '../types';

interface SidebarProps {
  conversations: Conversation[];
  currentConversationId: string | null;
  onNewConversation: () => void;
  onSelectConversation: (id: string) => void;
  onRenameConversation: (id: string, title: string) => void;
  onToggleConversationPinned: (id: string) => void;
  onDeleteConversation: (id: string) => void;
  onExportConversation: (id: string) => void;
  onClose?: () => void;
}

const Sidebar: React.FC<SidebarProps> = ({
  conversations,
  currentConversationId,
  onNewConversation,
  onSelectConversation,
  onRenameConversation,
  onToggleConversationPinned,
  onDeleteConversation,
  onExportConversation,
  onClose,
}) => {
  const [query, setQuery] = useState('');
  const [editingConversationId, setEditingConversationId] = useState<string | null>(null);
  const [editingTitle, setEditingTitle] = useState('');

  const formatDate = (timestamp: number) => {
    const date = new Date(timestamp);
    const now = new Date();
    const diff = now.getTime() - date.getTime();
    const days = Math.floor(diff / (1000 * 60 * 60 * 24));

    if (days <= 0) {
      return '今天';
    }
    if (days === 1) {
      return '昨天';
    }
    if (days < 7) {
      return `${days}天前`;
    }
    return date.toLocaleDateString('zh-CN');
  };

  const filteredConversations = useMemo(() => {
    const normalizedQuery = query.trim().toLowerCase();
    return [...conversations]
      .sort((a, b) => {
        if (Boolean(a.pinned) !== Boolean(b.pinned)) {
          return a.pinned ? -1 : 1;
        }
        return b.updatedAt - a.updatedAt;
      })
      .filter((conversation) => {
        if (!normalizedQuery) {
          return true;
        }
        return (conversation.title || '').toLowerCase().includes(normalizedQuery);
      });
  }, [conversations, query]);

  const startRenaming = (conversation: Conversation) => {
    setEditingConversationId(conversation.id);
    setEditingTitle(conversation.title || '');
  };

  const saveRename = () => {
    if (!editingConversationId) {
      return;
    }
    onRenameConversation(editingConversationId, editingTitle);
    setEditingConversationId(null);
    setEditingTitle('');
  };

  const cancelRename = () => {
    setEditingConversationId(null);
    setEditingTitle('');
  };

  return (
    <div className="sidebar">
      <div className="sidebar-head">
        <div className="sidebar-brand">
          <div className="sidebar-brand-mark">AH</div>
          <div>
            <p className="sidebar-brand-name">AI 助手</p>
            <p className="sidebar-brand-sub">桌面工作台</p>
          </div>
        </div>

        <button type="button" onClick={onNewConversation} className="sidebar-new-btn">
          <Plus size={17} />
          新建对话
        </button>

        <label className="sidebar-search-wrap">
          <Search size={14} className="sidebar-search-icon" />
          <input
            type="text"
            value={query}
            onChange={(event) => setQuery(event.target.value)}
            placeholder="搜索对话标题..."
            className="sidebar-search-input"
            aria-label="搜索对话"
          />
        </label>
      </div>

      <div className="sidebar-list">
        {filteredConversations.length === 0 ? (
          <div className="sidebar-empty">
            <MessageSquare size={40} />
            <p>{query.trim() ? '没有匹配的对话' : '暂无对话'}</p>
          </div>
        ) : (
          filteredConversations.map((conversation, index) => (
            <article
              key={conversation.id}
              className={`conversation-card ${
                currentConversationId === conversation.id ? 'is-active' : ''
              }`}
              style={{ animationDelay: `${index * 45}ms` }}
            >
              {editingConversationId === conversation.id ? (
                <div className="conversation-rename-wrap">
                  <input
                    value={editingTitle}
                    onChange={(event) => setEditingTitle(event.target.value)}
                    onKeyDown={(event) => {
                      if (event.key === 'Enter') {
                        event.preventDefault();
                        saveRename();
                      } else if (event.key === 'Escape') {
                        event.preventDefault();
                        cancelRename();
                      }
                    }}
                    autoFocus
                    className="conversation-rename-input"
                    aria-label="重命名对话"
                  />
                  <div className="conversation-rename-actions">
                    <button type="button" className="icon-action" title="保存" onClick={saveRename}>
                      <Check size={14} />
                    </button>
                    <button type="button" className="icon-action danger" title="取消" onClick={cancelRename}>
                      <X size={14} />
                    </button>
                  </div>
                </div>
              ) : (
                <button
                  type="button"
                  onClick={() => onSelectConversation(conversation.id)}
                  className="conversation-main"
                >
                  <p className="conversation-title">
                    {conversation.pinned ? '置顶 · ' : ''}
                    {conversation.title || '未命名对话'}
                  </p>
                  <p className="conversation-meta">
                    {conversation.messages.length} 条消息 · {formatDate(conversation.updatedAt)}
                  </p>
                </button>
              )}

              <div className="conversation-actions">
                <button
                  type="button"
                  className={`icon-action ${conversation.pinned ? 'active' : ''}`}
                  title={conversation.pinned ? '取消置顶' : '置顶'}
                  onClick={(event) => {
                    event.stopPropagation();
                    onToggleConversationPinned(conversation.id);
                  }}
                >
                  {conversation.pinned ? <PinOff size={14} /> : <Pin size={14} />}
                </button>

                <button
                  type="button"
                  className="icon-action"
                  title="重命名"
                  onClick={(event) => {
                    event.stopPropagation();
                    startRenaming(conversation);
                  }}
                >
                  <Pencil size={14} />
                </button>

                <button
                  type="button"
                  className="icon-action"
                  title="导出"
                  onClick={(event) => {
                    event.stopPropagation();
                    onExportConversation(conversation.id);
                  }}
                >
                  <Download size={14} />
                </button>

                <button
                  type="button"
                  className="icon-action danger"
                  title="删除"
                  onClick={(event) => {
                    event.stopPropagation();
                    if (window.confirm('确定删除这个对话吗？')) {
                      onDeleteConversation(conversation.id);
                    }
                  }}
                >
                  <Trash2 size={14} />
                </button>
              </div>
            </article>
          ))
        )}
      </div>

      {onClose && (
        <div className="sidebar-footer">
          <button type="button" onClick={onClose} className="sidebar-close-btn">
            <X size={16} />
            关闭侧边栏
          </button>
        </div>
      )}
    </div>
  );
};

export default Sidebar;
