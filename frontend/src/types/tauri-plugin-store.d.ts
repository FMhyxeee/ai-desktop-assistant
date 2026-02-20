declare module '@tauri-apps/plugin-store' {
  export class Store {
    constructor(path: string);
    set(key: string, value: unknown): Promise<void>;
    get<T>(key: string): Promise<T | null>;
    delete(key: string): Promise<void>;
    save(): Promise<void>;
  }
}
