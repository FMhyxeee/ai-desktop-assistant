import { StrictMode } from 'react';
import { createRoot } from 'react-dom/client';
import AppErrorBoundary from './components/AppErrorBoundary.tsx';
import App from './App.tsx';
import './index.css';
import { installFrontendDebugHooks } from './lib/logger';

installFrontendDebugHooks();

createRoot(document.getElementById('root')!).render(
  <StrictMode>
    <AppErrorBoundary>
      <App />
    </AppErrorBoundary>
  </StrictMode>
);
