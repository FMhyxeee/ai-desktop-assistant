import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import type { AgentEvent } from '../types';

export type FrontendLogLevel = 'debug' | 'info' | 'warn' | 'error';

export class TauriAPI {
  static async askAgent(input: string): Promise<string> {
    try {
      return await invoke<string>('ask_agent', { input });
    } catch (error) {
      console.error('ask_agent error:', error);
      throw new Error(typeof error === 'string' ? error : 'Failed to ask agent');
    }
  }

  static async startAgentStream(input: string, taskId?: string): Promise<string> {
    try {
      return await invoke<string>('start_agent_stream', {
        input,
        taskId,
      });
    } catch (error) {
      console.error('start_agent_stream error:', error);
      throw new Error(typeof error === 'string' ? error : 'Failed to start stream');
    }
  }

  static async cancelAgentTask(taskId: string): Promise<void> {
    try {
      await invoke('cancel_agent_task', { taskId });
    } catch (error) {
      console.error('cancel_agent_task error:', error);
      throw new Error(typeof error === 'string' ? error : 'Failed to cancel task');
    }
  }

  static async listenAgentEvents(callback: (event: AgentEvent) => void): Promise<() => void> {
    try {
      const unlisten = await listen<AgentEvent>('agent://event', (event) => {
        callback(event.payload);
      });
      return unlisten;
    } catch (error) {
      console.error('listenAgentEvents error:', error);
      throw new Error('Failed to listen to agent events');
    }
  }

  static async frontendLog(
    level: FrontendLogLevel,
    message: string,
    context?: unknown
  ): Promise<void> {
    try {
      await invoke('frontend_log', {
        level,
        message,
        context: context ?? null,
      });
    } catch {
      // Keep console path as the fallback when Tauri command is unavailable.
    }
  }

  static async setStore(key: string, value: unknown): Promise<void> {
    try {
      localStorage.setItem(key, JSON.stringify(value));
    } catch (error) {
      console.error('setStore error:', error);
      throw new Error('Failed to save to store');
    }
  }

  static async getStore<T>(key: string, defaultValue: T): Promise<T> {
    try {
      const item = localStorage.getItem(key);
      return item ? (JSON.parse(item) as T) : defaultValue;
    } catch (error) {
      console.error('getStore error:', error);
      return defaultValue;
    }
  }

  static async deleteStore(key: string): Promise<void> {
    try {
      localStorage.removeItem(key);
    } catch (error) {
      console.error('deleteStore error:', error);
      throw new Error('Failed to delete from store');
    }
  }
}
