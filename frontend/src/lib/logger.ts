import { TauriAPI, type FrontendLogLevel } from './tauri';

type JsonPrimitive = string | number | boolean | null;
type JsonLike = JsonPrimitive | JsonLike[] | { [key: string]: JsonLike };

interface ErrorDetails {
  name?: string;
  message: string;
  stack?: string;
}

const originalConsole = {
  debug: console.debug.bind(console),
  info: console.info.bind(console),
  warn: console.warn.bind(console),
  error: console.error.bind(console),
};

let consoleHookInstalled = false;
let errorHookInstalled = false;

const toErrorDetails = (value: unknown): ErrorDetails => {
  if (value instanceof Error) {
    const details: ErrorDetails = {
      message: value.message,
    };
    if (value.name) {
      details.name = value.name;
    }
    if (value.stack) {
      details.stack = value.stack;
    }
    return details;
  }

  if (typeof value === 'string') {
    return { message: value };
  }

  if (value && typeof value === 'object') {
    try {
      return { message: JSON.stringify(value) };
    } catch {
      return { message: String(value) };
    }
  }

  return { message: String(value) };
};

const normalizeForJson = (value: unknown): JsonLike => {
  if (value === null || value === undefined) {
    return null;
  }

  if (typeof value === 'string' || typeof value === 'number' || typeof value === 'boolean') {
    return value;
  }

  if (value instanceof Error) {
    return normalizeForJson(toErrorDetails(value));
  }

  if (Array.isArray(value)) {
    return value.map((item) => normalizeForJson(item));
  }

  if (typeof value === 'object') {
    const record: { [key: string]: JsonLike } = {};
    for (const [key, item] of Object.entries(value as Record<string, unknown>)) {
      record[key] = normalizeForJson(item);
    }
    return record;
  }

  return String(value);
};

const summarizeConsoleArgs = (args: unknown[]): string => {
  if (args.length === 0) {
    return 'Console call without arguments';
  }

  return args
    .map((arg) => {
      if (typeof arg === 'string') {
        return arg;
      }
      if (arg instanceof Error) {
        return arg.message;
      }
      try {
        return JSON.stringify(arg);
      } catch {
        return String(arg);
      }
    })
    .join(' ')
    .slice(0, 2000);
};

const sendToBackend = (level: FrontendLogLevel, message: string, context?: unknown) => {
  void TauriAPI.frontendLog(level, message, context).catch((error) => {
    originalConsole.warn('[frontend][warn] Failed to forward log to backend', error);
  });
};

const emit = (level: FrontendLogLevel, message: string, context?: Record<string, unknown>) => {
  originalConsole[level](`[frontend][${level}] ${message}`, context ?? '');
  sendToBackend(level, message, context ? normalizeForJson(context) : null);
};

const mirrorConsoleCall = (level: FrontendLogLevel, args: unknown[]) => {
  sendToBackend(level, summarizeConsoleArgs(args), {
    consoleArgs: normalizeForJson(args),
  });
};

export const logger = {
  debug: (message: string, context?: Record<string, unknown>) => {
    emit('debug', message, context);
  },
  info: (message: string, context?: Record<string, unknown>) => {
    emit('info', message, context);
  },
  warn: (message: string, context?: Record<string, unknown>) => {
    emit('warn', message, context);
  },
  error: (message: string, context?: Record<string, unknown>) => {
    emit('error', message, context);
  },
};

export const reportError = (message: string, error: unknown, context?: Record<string, unknown>) => {
  const details = toErrorDetails(error);
  logger.error(message, {
    ...context,
    error: details,
  });
};

export const installFrontendDebugHooks = () => {
  if (!consoleHookInstalled) {
    consoleHookInstalled = true;

    console.error = (...args: unknown[]) => {
      originalConsole.error(...args);
      mirrorConsoleCall('error', args);
    };

    console.warn = (...args: unknown[]) => {
      originalConsole.warn(...args);
      mirrorConsoleCall('warn', args);
    };
  }

  if (errorHookInstalled || typeof window === 'undefined') {
    return;
  }

  errorHookInstalled = true;

  window.addEventListener('error', (event) => {
    logger.error('window.error', {
      message: event.message,
      filename: event.filename,
      lineno: event.lineno,
      colno: event.colno,
      error: normalizeForJson(event.error),
    });
  });

  window.addEventListener('unhandledrejection', (event) => {
    logger.error('window.unhandledrejection', {
      reason: normalizeForJson(event.reason),
    });
  });

  logger.info('Frontend debug hooks installed');
};
