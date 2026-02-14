import React from 'react';
import { logger } from '../lib/logger';

interface AppErrorBoundaryProps {
  children: React.ReactNode;
}

interface AppErrorBoundaryState {
  hasError: boolean;
  errorMessage: string;
}

class AppErrorBoundary extends React.Component<AppErrorBoundaryProps, AppErrorBoundaryState> {
  state: AppErrorBoundaryState = {
    hasError: false,
    errorMessage: '',
  };

  static getDerivedStateFromError(error: Error): AppErrorBoundaryState {
    return {
      hasError: true,
      errorMessage: error.message,
    };
  }

  componentDidCatch(error: Error, info: React.ErrorInfo): void {
    logger.error('React error boundary caught an exception', {
      error,
      componentStack: info.componentStack,
    });
  }

  render(): React.ReactNode {
    if (this.state.hasError) {
      return (
        <div className="error-shell">
          <div className="error-card">
            <h1 className="error-title">前端发生异常</h1>
            <p className="error-text">
              异常信息已同步记录到浏览器控制台与 Tauri 终端日志。
            </p>
            <pre className="error-detail">{this.state.errorMessage}</pre>
            <button
              type="button"
              className="btn-primary error-button"
              onClick={() => {
                window.location.reload();
              }}
            >
              重新加载应用
            </button>
          </div>
        </div>
      );
    }

    return this.props.children;
  }
}

export default AppErrorBoundary;
