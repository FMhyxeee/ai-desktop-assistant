import React from 'react';
import { Download, MessageSquare, Plus, Trash2, X } from 'lucide-react';
import type { Conversation } from '../types';

interface SidebarProps {
  conversations: Conversation[];
  currentConversationId: string | null;
  onNewConversation: () => void;
  onSelectConversation: (id: string) => void;
  onDeleteConversation: (id: string) => void;
  onExportConversation: (id: string) => void;
  onClose?: () => void;
}

const Sidebar: React.FC<SidebarProps> = ({
  conversations,
  currentConversationId,
  onNewConversation,
  onSelectConversation,
  onDeleteConversation,
  onExportConversation,
  onClose,
}) => {
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
      </div>

      <div className="sidebar-list">
        {conversations.length === 0 ? (
          <div className="sidebar-empty">
            <MessageSquare size={40} />
            <p>暂无对话</p>
          </div>
        ) : (
          conversations.map((conversation, index) => (
            <article
              key={conversation.id}
              className={`conversation-card ${
                currentConversationId === conversation.id ? 'is-active' : ''
              }`}
              style={{ animationDelay: `${index * 45}ms` }}
            >
              <button
                type="button"
                onClick={() => onSelectConversation(conversation.id)}
                className="conversation-main"
              >
                <p className="conversation-title">{conversation.title || '未命名对话'}</p>
                <p className="conversation-meta">
                  {conversation.messages.length} 条消息 · {formatDate(conversation.updatedAt)}
                </p>
              </button>

              <div className="conversation-actions">
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
