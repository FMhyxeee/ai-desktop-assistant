import React, { useState } from 'react';
import { AlertTriangle, CheckCircle, XCircle, Eye, EyeOff } from 'lucide-react';
import type { ToolApprovalRequest } from '../types';
import './ApprovalModal.css';

interface ApprovalModalProps {
  request: ToolApprovalRequest | null;
  onResolve: (approved: boolean, rememberChoice: boolean) => void;
  onClose: () => void;
}

const formatArgs = (args: unknown): string => {
  try {
    return JSON.stringify(args, null, 2);
  } catch {
    return String(args);
  }
};

const getRiskLevelColor = (level: string): string => {
  switch (level) {
    case 'low':
      return 'text-green-600';
    case 'medium':
      return 'text-yellow-600';
    case 'high':
      return 'text-red-600';
    default:
      return 'text-gray-600';
  }
};

const getRiskLevelIcon = (level: string) => {
  switch (level) {
    case 'low':
      return <CheckCircle size={20} className="text-green-600" />;
    case 'medium':
      return <AlertTriangle size={20} className="text-yellow-600" />;
    case 'high':
      return <XCircle size={20} className="text-red-600" />;
    default:
      return <AlertTriangle size={20} className="text-gray-600" />;
  }
};

const getRiskLevelText = (level: string): string => {
  switch (level) {
    case 'low':
      return '低风险';
    case 'medium':
      return '中等风险';
    case 'high':
      return '高风险';
    default:
      return '未知';
  }
};

const ApprovalModal: React.FC<ApprovalModalProps> = ({ request, onResolve, onClose }) => {
  const [rememberChoice, setRememberChoice] = useState(false);
  const [showDetails, setShowDetails] = useState(false);

  if (!request) {
    return null;
  }

  const isExpired = Date.now() > request.expiresAtUnixMs;

  const handleApprove = () => {
    onResolve(true, rememberChoice);
  };

  const handleReject = () => {
    onResolve(false, rememberChoice);
  };

  return (
    <div className="modal-overlay" onClick={onClose}>
      <div className="modal-content" onClick={(e) => e.stopPropagation()}>
        {/* Header */}
        <div className="modal-header">
          <div className="modal-title-row">
            <AlertTriangle size={24} className="text-yellow-600" />
            <h2 className="modal-title">权限申请</h2>
          </div>
          <button type="button" className="modal-close-btn" onClick={onClose}>
            ✕
          </button>
        </div>

        {/* Body */}
        <div className="modal-body">
          {/* Risk Level */}
          <div className={`risk-badge ${request.riskLevel}`}>
            {getRiskLevelIcon(request.riskLevel)}
            <span className={`risk-text ${getRiskLevelColor(request.riskLevel)}`}>
              {getRiskLevelText(request.riskLevel)}
            </span>
          </div>

          {/* Tool Name */}
          <div className="approval-section">
            <label className="approval-label">工具名称</label>
            <div className="approval-value">{request.tool}</div>
          </div>

          {/* Reason */}
          <div className="approval-section">
            <label className="approval-label">申请原因</label>
            <div className="approval-value">{request.reason}</div>
          </div>

          {/* Args Preview */}
          <div className="approval-section">
            <div className="approval-header">
              <label className="approval-label">参数详情</label>
              <button
                type="button"
                className="approval-toggle-btn"
                onClick={() => setShowDetails(!showDetails)}
              >
                {showDetails ? <EyeOff size={16} /> : <Eye size={16} />}
                {showDetails ? '隐藏' : '显示'}
              </button>
            </div>
            {showDetails ? (
              <pre className="approval-args-preview">{formatArgs(request.args)}</pre>
            ) : (
              <div className="approval-args-summary">
                {typeof request.args === 'object' && request.args !== null
                  ? `${Object.keys(request.args).length} 个参数`
                  : '参数已隐藏'}
              </div>
            )}
          </div>

          {/* Warning for high risk */}
          {request.riskLevel === 'high' && (
            <div className="approval-warning">
              <AlertTriangle size={16} className="text-yellow-600" />
              <span>警告：此操作可能会对系统造成重大影响，请仔细审查后再决定。</span>
            </div>
          )}

          {/* Expired Warning */}
          {isExpired && (
            <div className="approval-warning error">
              <AlertTriangle size={16} className="text-red-600" />
              <span>此权限请求已过期，请重新发起请求。</span>
            </div>
          )}
        </div>

        {/* Options */}
        <div className="approval-options">
          <label className="approval-checkbox">
            <input
              type="checkbox"
              checked={rememberChoice}
              onChange={(e) => setRememberChoice(e.target.checked)}
              disabled={isExpired}
            />
            <span>记住此选择（后续类似操作自动批准）</span>
          </label>
        </div>

        {/* Footer */}
        <div className="modal-footer">
          <button
            type="button"
            className="btn-secondary"
            onClick={handleReject}
            disabled={isExpired}
          >
            拒绝
          </button>
          <button
            type="button"
            className="btn-primary"
            onClick={handleApprove}
            disabled={isExpired}
          >
            批准
          </button>
        </div>
      </div>
    </div>
  );
};

export default ApprovalModal;
