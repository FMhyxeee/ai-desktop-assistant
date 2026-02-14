# AI Desktop Assistant - 开发指南

## 🎯 项目概述

这是一个基于 Tauri 2.0 构建的桌面 AI 助手应用，采用 React + TypeScript + Vite 作为前端技术栈。

### 核心特性

- ✅ **流式对话**: 实时显示 AI 响应，类似 ChatGPT/Claude 的打字机效果
- ✅ **Markdown 支持**: 完整的 Markdown 渲染和代码语法高亮
- ✅ **多对话管理**: 支持创建、删除、导出多个对话会话
- ✅ **配置管理**: 可视化配置 AI 提供商、API Key、模型和 System Prompt
- ✅ **数据持久化**: 使用 localStorage 保存对话历史
- ✅ **Claude 风格 UI**: 现代化、简洁的用户界面设计

## 📁 项目结构

```
ai-desktop-assistant/
├── frontend/                 # React 前端项目
│   ├── src/
│   │   ├── components/    # UI 组件
│   │   │   ├── ChatView.tsx       # 主对话视图
│   │   │   ├── MessageList.tsx     # 消息列表组件
│   │   │   ├── ChatInput.tsx       # 聊天输入框
│   │   │   ├── Sidebar.tsx         # 侧边栏（对话列表）
│   │   │   ├── SettingsModal.tsx  # 设置弹窗
│   │   │   └── MarkdownMessage.tsx # Markdown 渲染组件
│   │   ├── lib/
│   │   │   └── tauri.ts          # Tauri API 封装层
│   │   ├── store/
│   │   │   └── appStore.ts        # Zustand 状态管理
│   │   ├── types/
│   │   │   └── index.ts           # TypeScript 类型定义
│   │   ├── App.tsx
│   │   ├── index.css
│   │   └── main.tsx
│   ├── package.json
│   ├── vite.config.ts
│   ├── tailwind.config.js
│   └── tsconfig.json
├── src-tauri/               # Rust 后端
│   ├── src/
│   │   ├── lib.rs
│   │   └── agent_service/      # Agent 服务实现
│   ├── Cargo.toml
│   └── tauri.conf.json
└── package.json              # 根目录脚本
```

## 🚀 快速开始

### 环境要求

- **Node.js**: >= 18.x
- **pnpm**: >= 8.x (推荐) 或 npm/yarn
- **Rust**: >= 1.77.2
- **系统依赖**:
  - Windows: 无需额外依赖
  - Linux: 需要安装 WebView2
  - macOS: 无需额外依赖

### 安装步骤

#### 1. 安装前端依赖

```bash
cd frontend
pnpm install
```

#### 2. 安装根目录依赖

```bash
cd ..
pnpm install
```

#### 3. 配置环境变量

在项目根目录创建 `.env` 文件（可选，也可在应用内配置）：

```env
# GLM 配置
GLM_CODE_PLAN_URL=https://open.bigmodel.cn/api/coding/paas/v4
GLM_API_KEY=your_api_key_here

# 或使用 OpenAI
OPENAI_API_KEY=your_openai_key_here
```

### 运行应用

#### 开发模式

```bash
pnpm tauri dev
```

这将：
1. 启动 Vite 开发服务器 (http://localhost:5173)
2. 编译 Rust 后端
3. 打开 Tauri 应用窗口

#### 生产构建

```bash
pnpm tauri build
```

构建产物位于 `src-tauri/target/release/bundle/`

## 🔧 技术栈详解

### 前端

| 技术 | 版本 | 用途 |
|------|------|------|
| React | 19.2.4 | UI 框架 |
| TypeScript | 5.9.3 | 类型安全 |
| Vite | 7.3.1 | 构建工具 |
| Tailwind CSS | 4.1.18 | 样式框架 |
| Zustand | 5.0.11 | 状态管理 |
| React Markdown | 10.1.0 | Markdown 渲染 |
| React Syntax Highlighter | 16.1.0 | 代码高亮 |
| Lucide React | 0.564.0 | 图标库 |

### 后端

| 技术 | 版本 | 用途 |
|------|------|------|
| Rust | 1.77.2 | 后端语言 |
| Tauri | 2.1.0 | 桌面应用框架 |
| agent-lib | 本地路径 | AI 核心库 |

## 📝 API 命令

前端通过 Tauri 的 `invoke` API 调用后端命令：

### 1. ask_agent

简单对话请求（非流式）

```typescript
const response = await invoke<string>('ask_agent', {
  input: '你的问题'
});
```

### 2. start_agent_stream

启动流式对话

```typescript
const taskId = await invoke<string>('start_agent_stream', {
  input: '你的问题',
  taskId: 'optional-task-id'
});
```

### 3. cancel_agent_task

取消正在进行的任务

```typescript
await invoke('cancel_agent_task', {
  taskId: 'task-id'
});
```

## 🎨 事件系统

监听 Agent 事件：

```typescript
import { listen } from '@tauri-apps/api/event';

const unlisten = await listen('agent://event', (event) => {
  switch (event.payload.type) {
    case 'started':
      console.log('任务开始');
      break;
    case 'delta':
      console.log('收到数据片段:', event.payload.chunk);
      break;
    case 'completed':
      console.log('任务完成:', event.payload.output);
      break;
    case 'error':
      console.error('任务错误:', event.payload.message);
      break;
  }
});
```

## 💾 数据存储

应用使用 localStorage 持久化以下数据：

- `conversations`: 对话列表数组
- `currentConversationId`: 当前选中的对话 ID

### 导出功能

对话可导出为 JSON 格式，包含完整的消息历史。

## 🎨 UI 组件说明

### ChatView

主视图组件，管理整个聊天界面：
- 侧边栏切换
- 消息列表渲染
- 输入框状态管理
- Agent 事件监听

### MessageList

渲染消息列表：
- 用户消息（右侧，蓝色）
- AI 消息（左侧，白色）
- 自动滚动到最新消息

### ChatInput

消息输入组件：
- 支持多行输入
- Enter 发送，Shift+Enter 换行
- 流式响应时可停止
- 字符计数显示

### Sidebar

对话管理侧边栏：
- 对话列表
- 创建新对话
- 删除对话
- 导出对话
- 显示消息数量和时间

### SettingsModal

配置管理弹窗：
- AI 提供商选择（OpenAI/GLM）
- 模型名称配置
- API Key 配置（支持显示/隐藏）
- System Prompt 编辑

## 🔧 常见问题

### 1. WebSocket 连接失败

**错误信息**: `WebSocket connection failed`

**解决方案**:
- 确保 Vite 开发服务器正在运行
- 检查端口 5173 未被占用
- 重启开发服务器

### 2. Rust 依赖下载缓慢

**现象**: `Updating crates.io index` 很慢或超时

**解决方案**:
- 配置 Rust 镜像源（使用国内镜像）
- 或耐心等待（首次运行可能需要 5-10 分钟）

### 3. 类型错误

**现象**: TypeScript 编译失败

**解决方案**:
```bash
cd frontend
pnpm build
```

查看详细错误信息并修复。

### 4. Tauri 命令未找到

**错误信息**: `command not found`

**解决方案**:
- 确保后端已正确编译
- 检查 `src-tauri/src/lib.rs` 中的命令注册
- 重启 Tauri 应用

## 🚢 构建优化建议

### 减小包体积

当前构建后主 chunk 约 1MB，可通过以下方式优化：

1. **代码分割**:
```typescript
// vite.config.ts
build: {
  rollupOptions: {
    output: {
      manualChunks: {
        'react-markdown': ['react-markdown'],
        'react-syntax-highlighter': ['react-syntax-highlighter']
      }
    }
  }
}
```

2. **Tree Shaking**:
   - 使用 ES modules
   - 避免导入整个库

3. **生产优化**:
   - 启用 Tauri 的生产模式
   - 使用 `tauri build` 而非 `dev`

## 📚 扩展建议

### 功能扩展

- [ ] 对话搜索功能
- [ ] 对话文件夹/分组
- [ ] 导出为 Markdown/PDF
- [ ] 对话分享功能
- [ ] 快捷键支持
- [ ] 主题切换（暗色模式）
- [ ] 插件系统
- [ ] 多语言支持

### 技术优化

- [ ] 使用虚拟滚动优化长对话
- [ ] 实现 IndexedDB 存储大量对话
- [ ] 添加离线模式
- [ ] PWA 支持
- [ ] 单元测试覆盖

## 📄 许可证

MIT

## 🤝 贡献

欢迎提交 Issue 和 Pull Request！

---

**注意**: 首次运行 `pnpm tauri dev` 时，Rust 会下载并编译依赖，可能需要较长时间（取决于网络速度）。请耐心等待。
