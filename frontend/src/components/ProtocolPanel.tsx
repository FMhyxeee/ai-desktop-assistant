import React, { useMemo, useState } from 'react';
import { AlertCircle, Copy, Download, Filter, RotateCcw, Rows3 } from 'lucide-react';
import type { ProtocolCard, ProtocolCardDirection, ProtocolCardLevel } from '../types';

interface ProtocolPanelProps {
  cards: ProtocolCard[];
  onRetry: (cardId: string) => void;
  onResolveConfigChangeRequest: (
    taskId: string,
    requestId: string,
    approved: boolean,
    persist: boolean
  ) => Promise<void> | void;
}

type DirectionFilter = 'all' | ProtocolCardDirection;
type LevelFilter = 'all' | ProtocolCardLevel;
type DetailMode = 'tree' | 'raw' | 'diff';

type DiffLine = {
  type: 'context' | 'added' | 'removed';
  text: string;
};

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

const downloadJson = (filename: string, data: unknown) => {
  const blob = new Blob([JSON.stringify(data, null, 2)], { type: 'application/json' });
  const url = URL.createObjectURL(blob);
  const anchor = document.createElement('a');
  anchor.href = url;
  anchor.download = filename;
  anchor.click();
  URL.revokeObjectURL(url);
};

const buildLineDiff = (beforeText: string, afterText: string): DiffLine[] => {
  const before = beforeText.split('\n');
  const after = afterText.split('\n');
  const m = before.length;
  const n = after.length;
  const dp: number[][] = Array.from({ length: m + 1 }, () => Array<number>(n + 1).fill(0));

  for (let i = m - 1; i >= 0; i -= 1) {
    for (let j = n - 1; j >= 0; j -= 1) {
      if (before[i] === after[j]) {
        dp[i][j] = dp[i + 1][j + 1] + 1;
      } else {
        dp[i][j] = Math.max(dp[i + 1][j], dp[i][j + 1]);
      }
    }
  }

  const lines: DiffLine[] = [];
  let i = 0;
  let j = 0;

  while (i < m && j < n) {
    if (before[i] === after[j]) {
      lines.push({ type: 'context', text: before[i] });
      i += 1;
      j += 1;
    } else if (dp[i + 1][j] >= dp[i][j + 1]) {
      lines.push({ type: 'removed', text: before[i] });
      i += 1;
    } else {
      lines.push({ type: 'added', text: after[j] });
      j += 1;
    }
  }

  while (i < m) {
    lines.push({ type: 'removed', text: before[i] });
    i += 1;
  }

  while (j < n) {
    lines.push({ type: 'added', text: after[j] });
    j += 1;
  }

  return lines;
};

const formatPrimitive = (value: unknown): string => {
  if (value === null) {
    return 'null';
  }
  if (typeof value === 'string') {
    return `"${value}"`;
  }
  if (typeof value === 'number' || typeof value === 'boolean') {
    return String(value);
  }
  if (typeof value === 'undefined') {
    return 'undefined';
  }
  return JSON.stringify(value);
};

const JsonTree: React.FC<{ value: unknown }> = ({ value }) => {
  const [collapsed, setCollapsed] = useState<Record<string, boolean>>({});

  const toggle = (path: string) => {
    setCollapsed((state) => ({
      ...state,
      [path]: !state[path],
    }));
  };

  const renderNode = (node: unknown, path: string, label?: string): React.ReactNode => {
    if (node === null || typeof node !== 'object') {
      return (
        <div className="json-tree-row" key={path}>
          {typeof label === 'string' && <span className="json-tree-key">{label}:</span>}
          <span className="json-tree-value">{formatPrimitive(node)}</span>
        </div>
      );
    }

    const isArray = Array.isArray(node);
    const entries = isArray
      ? (node as unknown[]).map((item, index) => [String(index), item] as const)
      : Object.entries(node as Record<string, unknown>);
    const isCollapsed = collapsed[path] ?? path !== 'root';

    return (
      <div className="json-tree-group" key={path}>
        <button
          type="button"
          className="json-tree-toggle"
          onClick={() => toggle(path)}
          aria-expanded={!isCollapsed}
        >
          <span className={`json-tree-caret ${isCollapsed ? 'collapsed' : ''}`}>▾</span>
          {typeof label === 'string' && <span className="json-tree-key">{label}</span>}
          <span className="json-tree-summary">{isArray ? `Array(${entries.length})` : `Object(${entries.length})`}</span>
        </button>

        {!isCollapsed && (
          <div className="json-tree-children">
            {entries.map(([childKey, childValue]) => renderNode(childValue, `${path}.${childKey}`, childKey))}
          </div>
        )}
      </div>
    );
  };

  return <div className="json-tree">{renderNode(value, 'root')}</div>;
};

const ProtocolPanel: React.FC<ProtocolPanelProps> = ({
  cards,
  onRetry,
  onResolveConfigChangeRequest,
}) => {
  const [expanded, setExpanded] = useState<Record<string, boolean>>({});
  const [detailMode, setDetailMode] = useState<Record<string, DetailMode>>({});
  const [directionFilter, setDirectionFilter] = useState<DirectionFilter>('all');
  const [levelFilter, setLevelFilter] = useState<LevelFilter>('all');
  const [typeFilter, setTypeFilter] = useState<string>('all');
  const [submittingRequests, setSubmittingRequests] = useState<Record<string, boolean>>({});

  const resolvedRequestIds = useMemo(() => {
    const resolved = new Set<string>();
    cards.forEach((card) => {
      if (card.payload.type === 'config_change_result') {
        resolved.add(card.payload.request_id);
      }
    });
    return resolved;
  }, [cards]);

  const chronologicalCards = useMemo(
    () => [...cards].sort((a, b) => (a.timestamp === b.timestamp ? a.seq - b.seq : a.timestamp - b.timestamp)),
    [cards]
  );

  const previousCardById = useMemo(() => {
    const mapping: Record<string, ProtocolCard | null> = {};
    chronologicalCards.forEach((card, index) => {
      mapping[card.id] = index === 0 ? null : chronologicalCards[index - 1];
    });
    return mapping;
  }, [chronologicalCards]);

  const typeOptions = useMemo(() => {
    const uniqueTypes = new Set<string>();
    chronologicalCards.forEach((card) => uniqueTypes.add(card.type));
    return Array.from(uniqueTypes).sort((a, b) => a.localeCompare(b));
  }, [chronologicalCards]);

  const filteredCards = useMemo(
    () =>
      chronologicalCards.filter((card) => {
        if (directionFilter !== 'all' && card.direction !== directionFilter) {
          return false;
        }
        if (levelFilter !== 'all' && card.level !== levelFilter) {
          return false;
        }
        if (typeFilter !== 'all' && card.type !== typeFilter) {
          return false;
        }
        return true;
      }),
    [chronologicalCards, directionFilter, levelFilter, typeFilter]
  );

  const displayCards = useMemo(() => [...filteredCards].reverse(), [filteredCards]);

  const toggleExpanded = (cardId: string) => {
    setExpanded((state) => ({
      ...state,
      [cardId]: !state[cardId],
    }));
  };

  const submitConfigRequest = async (
    taskId: string,
    requestId: string,
    approved: boolean,
    persist: boolean
  ) => {
    if (resolvedRequestIds.has(requestId) || submittingRequests[requestId]) {
      return;
    }
    setSubmittingRequests((state) => ({
      ...state,
      [requestId]: true,
    }));
    try {
      await onResolveConfigChangeRequest(taskId, requestId, approved, persist);
    } finally {
      setSubmittingRequests((state) => ({
        ...state,
        [requestId]: false,
      }));
    }
  };

  return (
    <aside className="protocol-panel">
      <header className="protocol-header">
        <div className="protocol-title-wrap">
          <Rows3 size={16} />
          <h2 className="protocol-title">协议时间线</h2>
        </div>

        <div className="protocol-header-actions">
          <button
            type="button"
            className="protocol-action"
            onClick={() => downloadJson(`protocol-timeline-${Date.now()}.json`, displayCards)}
          >
            <Download size={14} />
            导出全部
          </button>
          <span className="protocol-count">{displayCards.length}/{chronologicalCards.length}</span>
        </div>
      </header>

      <div className="protocol-filter-row">
        <div className="protocol-filter-item">
          <Filter size={13} />
          <span>方向</span>
          <select
            value={directionFilter}
            onChange={(event) => setDirectionFilter(event.target.value as DirectionFilter)}
            className="protocol-filter-select"
          >
            <option value="all">全部</option>
            <option value="op">Op</option>
            <option value="event">Event</option>
          </select>
        </div>

        <div className="protocol-filter-item">
          <span>级别</span>
          <select
            value={levelFilter}
            onChange={(event) => setLevelFilter(event.target.value as LevelFilter)}
            className="protocol-filter-select"
          >
            <option value="all">全部</option>
            <option value="info">Info</option>
            <option value="success">Success</option>
            <option value="warning">Warning</option>
            <option value="error">Error</option>
          </select>
        </div>

        <div className="protocol-filter-item">
          <span>类型</span>
          <select
            value={typeFilter}
            onChange={(event) => setTypeFilter(event.target.value)}
            className="protocol-filter-select"
          >
            <option value="all">全部</option>
            {typeOptions.map((type) => (
              <option key={type} value={type}>
                {type}
              </option>
            ))}
          </select>
        </div>
      </div>

      <div className="protocol-list">
        {displayCards.length === 0 && (
          <div className="protocol-empty">
            <AlertCircle size={16} />
            <span>{chronologicalCards.length === 0 ? '等待 Op/Event 卡片...' : '当前筛选条件下没有卡片'}</span>
          </div>
        )}

        {displayCards.map((card) => {
          const isExpanded = expanded[card.id] ?? false;
          const mode = detailMode[card.id] ?? 'tree';
          const previousCard = previousCardById[card.id];
          const configChangeRequestPayload =
            card.payload.type === 'config_change_request' ? card.payload : null;
          const diffLines =
            mode === 'diff' && previousCard
              ? buildLineDiff(payloadToText(previousCard.payload), payloadToText(card.payload))
              : [];

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
                    downloadJson(`protocol-card-${card.seq}-${card.id}.json`, card);
                  }}
                >
                  <Download size={14} />
                  导出
                </button>

                <button
                  type="button"
                  className={`protocol-action mode ${mode === 'tree' ? 'active' : ''}`}
                  onClick={(event) => {
                    event.stopPropagation();
                    setDetailMode((state) => ({ ...state, [card.id]: 'tree' }));
                  }}
                >
                  树
                </button>

                <button
                  type="button"
                  className={`protocol-action mode ${mode === 'raw' ? 'active' : ''}`}
                  onClick={(event) => {
                    event.stopPropagation();
                    setDetailMode((state) => ({ ...state, [card.id]: 'raw' }));
                  }}
                >
                  Raw
                </button>

                <button
                  type="button"
                  className={`protocol-action mode ${mode === 'diff' ? 'active' : ''}`}
                  onClick={(event) => {
                    event.stopPropagation();
                    setDetailMode((state) => ({ ...state, [card.id]: 'diff' }));
                  }}
                >
                  Diff
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

              {configChangeRequestPayload && (
                <div className="protocol-actions">
                  <button
                    type="button"
                    className="protocol-action"
                    disabled={
                      resolvedRequestIds.has(configChangeRequestPayload.request_id) ||
                      Boolean(submittingRequests[configChangeRequestPayload.request_id])
                    }
                    onClick={(event) => {
                      event.stopPropagation();
                      void submitConfigRequest(
                        card.taskId,
                        configChangeRequestPayload.request_id,
                        true,
                        false
                      );
                    }}
                  >
                    Approve (Session)
                  </button>
                  <button
                    type="button"
                    className="protocol-action"
                    disabled={
                      resolvedRequestIds.has(configChangeRequestPayload.request_id) ||
                      Boolean(submittingRequests[configChangeRequestPayload.request_id])
                    }
                    onClick={(event) => {
                      event.stopPropagation();
                      void submitConfigRequest(
                        card.taskId,
                        configChangeRequestPayload.request_id,
                        true,
                        true
                      );
                    }}
                  >
                    Approve + Persist
                  </button>
                  <button
                    type="button"
                    className="protocol-action"
                    disabled={
                      resolvedRequestIds.has(configChangeRequestPayload.request_id) ||
                      Boolean(submittingRequests[configChangeRequestPayload.request_id])
                    }
                    onClick={(event) => {
                      event.stopPropagation();
                      void submitConfigRequest(
                        card.taskId,
                        configChangeRequestPayload.request_id,
                        false,
                        false
                      );
                    }}
                  >
                    Reject
                  </button>
                </div>
              )}

              {isExpanded && mode === 'tree' && (
                <div className="protocol-payload json-mode">
                  <JsonTree value={card.payload} />
                </div>
              )}

              {isExpanded && mode === 'raw' && (
                <pre className="protocol-payload">
                  <code>{payloadToText(card.payload)}</code>
                </pre>
              )}

              {isExpanded && mode === 'diff' && (
                <div className="protocol-payload diff-mode">
                  {previousCard ? (
                    <div className="protocol-diff">
                      {diffLines.map((line, index) => (
                        <div key={`${card.id}-diff-${index}`} className={`protocol-diff-line ${line.type}`}>
                          <span className="protocol-diff-prefix">
                            {line.type === 'added' ? '+' : line.type === 'removed' ? '-' : ' '}
                          </span>
                          <code>{line.text}</code>
                        </div>
                      ))}
                    </div>
                  ) : (
                    <p className="protocol-diff-empty">没有上一个卡片可比较。</p>
                  )}
                </div>
              )}
            </article>
          );
        })}
      </div>
    </aside>
  );
};

export default ProtocolPanel;
