# Tauri Realtime Debug Guide

## 启动方式

在 `ai-desktop-assistant/` 根目录运行：

```bash
pnpm dev
```

这会同时启动：

- Vite dev server（前端 HMR）
- Tauri 桌面进程（Rust + WebView）

## 实时调试入口

1. 前端代码调试

- 打开 WebView DevTools（`Ctrl+Shift+I` 或 `F12`）
- 在 `Console` 看浏览器日志
- 在 `Sources` 里打断点，保存后热更新

2. Rust/Tauri 调试

- 直接看运行 `pnpm dev` 的终端输出
- 前端异常会通过 `frontend_log` 命令同步打印到终端

3. 日志级别

- 当前开发模式下 Tauri log 插件已设为 `Debug`
- 前端 logger 支持 `debug/info/warn/error`

## 前端错误打印链路

已接入以下自动上报：

- `window.onerror`
- `window.unhandledrejection`
- React `ErrorBoundary`
- 业务代码里的 `logger.error(...)`

所有错误会同时输出到：

- WebView Console
- Tauri 终端日志（方便 Codex 直接定位）

## 推荐排查流程

1. 复现问题并保留 `pnpm dev` 终端窗口
2. 在 DevTools Console 确认报错时间点
3. 对照终端里同一时间的 `frontend:` 日志定位上下文
4. 按 `taskId` 追踪流式会话问题（事件、取消、完成）
