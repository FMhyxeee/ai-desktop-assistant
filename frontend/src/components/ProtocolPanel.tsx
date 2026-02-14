import React, { useMemo, useState } from 'react';
import { AlertCircle, Copy, RotateCcw, Rows3 } from 'lucide-react';
import type { ProtocolCard } from '../types';

interface ProtocolPanelProps {
  cards: ProtocolCard[];
  onRetry: (cardId: string) => void;
}

const formatTime = (timestamp: number): string =>
  new Date(timestamp).toLocaleTimeString('zh-CN', {
    hour: '2-digit',
    minute: '2-digit',
    second: '2-digit',
  });

const payloadToText = (payload: unknown): string => {
  try {
    return JSON.stringify(payload, null, 2);
  } catch {
    return String(payload);
  }
};

const ProtocolPanel: React.FC<ProtocolPanelProps> = ({ cards, onRetry }) => {
  const [expanded, setExpanded] = useState<Record<string, boolean>>({});

  const sortedCards = useMemo(
    () => [...cards].sort((a, b) => (a.timestamp === b.timestamp ? a.seq - b.seq : a.timestamp - b.timestamp)),
    [cards]
  );

  const toggleExpanded = (cardId: string) => {
    setExpanded((state) => ({
      ...state,
      [cardId]: !state[cardId],
    }));
  };

  return (
    <aside className="protocol-panel">
      <header className="protocol-header">
        <div className="protocol-title-wrap">
          <Rows3 size={16} />
          <h2 className="protocol-title">协议时间线</h2>
        </div>
        <span className="protocol-count">{sortedCards.length}</span>
      </header>

      <div className="protocol-list">
        {sortedCards.length === 0 && (
          <div className="protocol-empty">
            <AlertCircle size={16} />
            <span>等待 Op/Event 卡片...</span>
          </div>
        )}

        {sortedCards.map((card) => {
          const isExpanded = expanded[card.id] ?? false;
          return (
            <article
              key={card.id}
              className={`protocol-card level-${card.level} ${isExpanded ? 'is-expanded' : ''}`}
              role="button"
              tabIndex={0}
              aria-expanded={isExpanded}
              onClick={() => toggleExpanded(card.id)}
              onKeyDown={(event) => {
                if (event.target !== event.currentTarget) {
                  return;
                }
                if (event.key === 'Enter' || event.key === ' ') {
                  event.preventDefault();
                  toggleExpanded(card.id);
                }
              }}
            >
              <div className="protocol-card-head">
                <div className="protocol-chip-group">
                  <span className={`protocol-chip ${card.direction}`}>{card.direction}</span>
                  <span className="protocol-chip type">{card.type}</span>
                  {card.streaming && <span className="protocol-chip streaming">streaming</span>}
                </div>
                <span className="protocol-time">{formatTime(card.timestamp)}</span>
              </div>

              <p className="protocol-summary">{card.summary}</p>

              <div className="protocol-actions">
                <button
                  type="button"
                  className="protocol-action"
                  onClick={(event) => {
                    event.stopPropagation();
                    void navigator.clipboard.writeText(payloadToText(card.payload));
                  }}
                >
                  <Copy size={14} />
                  复制
                </button>

                <button
                  type="button"
                  className="protocol-action"
                  onClick={(event) => {
                    event.stopPropagation();
                    toggleExpanded(card.id);
                  }}
                >
                  {isExpanded ? '收起' : '展开'}
                </button>

                {card.retryable && (
                  <button
                    type="button"
                    className="protocol-action retry"
                    onClick={(event) => {
                      event.stopPropagation();
                      onRetry(card.id);
                    }}
                  >
                    <RotateCcw size={14} />
                    重试
                  </button>
                )}
              </div>

              {isExpanded && (
                <pre className="protocol-payload">
                  <code>{payloadToText(card.payload)}</code>
                </pre>
              )}
            </article>
          );
        })}
      </div>
    </aside>
  );
};

export default ProtocolPanel;
