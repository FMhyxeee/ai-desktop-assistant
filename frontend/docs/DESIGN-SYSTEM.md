# MotherDuck 风格前端设计系统

> **设计理念**: 现代、友好、专业的数据平台美学 - "让数据分析变得简单有趣"

## 🎨 设计核心原则

### 1. **友好而不失专业**
- 使用鸭子插图和趣味元素增添亲和力
- 保持企业级产品的可信度和专业性
- 在趣味与功能之间找到完美平衡

### 2. **清晰的视觉层级**
- 强烈的排版对比
- 大胆的颜色对比
- 明确的信息架构

### 3. **动态交互体验**
- 流畅的动画和过渡
- 悬停状态的视觉反馈
- 引导用户操作的微交互

---

## 🎨 配色系统

### 主色板 (Primary Palette)
```css
/* 品牌主色 - 充满活力的橙红色 */
--color-primary: #FF6B35;
--color-primary-dark: #E85A2E;
--color-primary-light: #FF8B62;

/* 辅助色 - 深海蓝 */
--color-secondary: #2C3E50;
--color-secondary-dark: #1A252F;
--color-secondary-light: #34495E;

/* 强调色 - 明亮黄 */
--color-accent: #F7B731;
--color-accent-light: #FFD93D;
```

### 中性色板 (Neutral Palette)
```css
/* 文本颜色 */
--color-text-primary: #1F2937;
--color-text-secondary: #6B7280;
--color-text-tertiary: #9CA3AF;
--color-text-inverse: #FFFFFF;

/* 背景颜色 */
--color-bg-primary: #FFFFFF;
--color-bg-secondary: #F9FAFB;
--color-bg-tertiary: #F3F4F6;
--color-bg-inverse: #1F2937;

/* 边框颜色 */
--color-border-primary: #E5E7EB;
--color-border-secondary: #D1D5DB;
--color-border-focus: #FF6B35;
```

### 语义色板 (Semantic Palette)
```css
/* 成功 */
--color-success: #10B981;
--color-success-bg: #D1FAE5;

/* 警告 */
--color-warning: #F59E0B;
--color-warning-bg: #FEF3C7;

/* 错误 */
--color-error: #EF4444;
--color-error-bg: #FEE2E2;

/* 信息 */
--color-info: #3B82F6;
--color-info-bg: #DBEAFE;
```

---

## ✒️ 排版系统

### 字体家族
```css
/* 标题字体 - 使用几何无衬线字体 */
--font-heading: 'Aeonik', 'Space Grotesk', 'Plus Jakarta Sans', system-ui, sans-serif;

/* 正文字体 - 现代无衬线字体 */
--font-body: 'Inter', 'SF Pro Display', -apple-system, sans-serif;

/* 等宽字体 - 代码和数据 */
--font-mono: 'Aeonik Mono', 'Fira Code', 'JetBrains Mono', monospace;
```

### 字体层级
```css
/* Display - 超大标题 */
.text-display-xl { font-size: 72px; line-height: 1; font-weight: 700; letter-spacing: -0.02em; }
.text-display-lg { font-size: 56px; line-height: 1.1; font-weight: 700; letter-spacing: -0.015em; }
.text-display-md { font-size: 44px; line-height: 1.2; font-weight: 700; letter-spacing: -0.01em; }

/* Heading - 页面标题 */
.text-h1 { font-size: 36px; line-height: 1.3; font-weight: 700; letter-spacing: -0.01em; }
.text-h2 { font-size: 30px; line-height: 1.4; font-weight: 600; letter-spacing: -0.005em; }
.text-h3 { font-size: 24px; line-height: 1.5; font-weight: 600; }
.text-h4 { font-size: 20px; line-height: 1.5; font-weight: 600; }

/* Body - 正文 */
.text-body-xl { font-size: 20px; line-height: 1.6; font-weight: 400; }
.text-body-lg { font-size: 18px; line-height: 1.6; font-weight: 400; }
.text-body-md { font-size: 16px; line-height: 1.7; font-weight: 400; }
.text-body-sm { font-size: 14px; line-height: 1.7; font-weight: 400; }

/* Caption - 辅助文本 */
.text-caption { font-size: 12px; line-height: 1.5; font-weight: 500; }
.text-overline { font-size: 11px; line-height: 1.5; font-weight: 600; letter-spacing: 0.06em; text-transform: uppercase; }
```

---

## 🎯 组件设计规范

### 按钮 (Buttons)

#### 主要按钮 (Primary)
```css
.btn-primary {
  background: linear-gradient(135deg, #FF6B35 0%, #FF8B62 100%);
  color: white;
  padding: 14px 28px;
  border-radius: 12px;
  font-weight: 600;
  font-size: 16px;
  border: none;
  cursor: pointer;
  box-shadow: 0 4px 14px rgba(255, 107, 53, 0.3);
  transition: all 0.3s cubic-bezier(0.4, 0, 0.2, 1);
}

.btn-primary:hover {
  transform: translateY(-2px);
  box-shadow: 0 6px 20px rgba(255, 107, 53, 0.4);
}

.btn-primary:active {
  transform: translateY(0);
}
```

#### 次要按钮 (Secondary)
```css
.btn-secondary {
  background: white;
  color: #FF6B35;
  padding: 14px 28px;
  border-radius: 12px;
  font-weight: 600;
  font-size: 16px;
  border: 2px solid #FF6B35;
  cursor: pointer;
  transition: all 0.3s ease;
}

.btn-secondary:hover {
  background: #FF6B35;
  color: white;
}
```

#### 文字按钮 (Text)
```css
.btn-text {
  background: transparent;
  color: #FF6B35;
  padding: 8px 16px;
  border-radius: 8px;
  font-weight: 600;
  font-size: 14px;
  border: none;
  cursor: pointer;
  transition: background 0.2s ease;
}

.btn-text:hover {
  background: rgba(255, 107, 53, 0.1);
}
```

### 卡片 (Cards)
```css
.card {
  background: white;
  border-radius: 16px;
  padding: 24px;
  box-shadow: 0 4px 6px rgba(0, 0, 0, 0.05), 0 10px 20px rgba(0, 0, 0, 0.03);
  border: 1px solid #E5E7EB;
  transition: all 0.3s cubic-bezier(0.4, 0, 0.2, 1);
}

.card:hover {
  transform: translateY(-4px);
  box-shadow: 0 20px 40px rgba(0, 0, 0, 0.1);
  border-color: #FF6B35;
}

.card-interactive {
  cursor: pointer;
}
```

### 输入框 (Inputs)
```css
.input {
  width: 100%;
  padding: 14px 16px;
  border: 2px solid #E5E7EB;
  border-radius: 12px;
  font-size: 16px;
  background: white;
  transition: all 0.2s ease;
}

.input:focus {
  outline: none;
  border-color: #FF6B35;
  box-shadow: 0 0 0 4px rgba(255, 107, 53, 0.1);
}

.input::placeholder {
  color: #9CA3AF;
}
```

### 标签 (Tags/Badges)
```css
.badge {
  display: inline-flex;
  align-items: center;
  padding: 6px 12px;
  border-radius: 20px;
  font-size: 12px;
  font-weight: 600;
  letter-spacing: 0.02em;
}

.badge-primary {
  background: rgba(255, 107, 53, 0.1);
  color: #FF6B35;
}

.badge-success {
  background: #D1FAE5;
  color: #10B981;
}

.badge-warning {
  background: #FEF3C7;
  color: #F59E0B;
}
```

---

## 📐 间距系统

使用 8px 基础网格系统:
```css
--spacing-0: 0;
--spacing-1: 4px;
--spacing-2: 8px;
--spacing-3: 12px;
--spacing-4: 16px;
--spacing-5: 20px;
--spacing-6: 24px;
--spacing-8: 32px;
--spacing-10: 40px;
--spacing-12: 48px;
--spacing-16: 64px;
--spacing-20: 80px;
--spacing-24: 96px;
```

---

## 🌊 动画与过渡

### 标准缓动函数
```css
--ease-out-cubic: cubic-bezier(0.33, 1, 0.68, 1);
--ease-in-out-cubic: cubic-bezier(0.65, 0, 0.35, 1);
--ease-out-quart: cubic-bezier(0.25, 1, 0.5, 1);
```

### 标准持续时间
```css
--duration-fast: 150ms;
--duration-base: 250ms;
--duration-slow: 350ms;
--duration-slower: 500ms;
```

### 常用动画
```css
/* 淡入 */
@keyframes fadeIn {
  from { opacity: 0; }
  to { opacity: 1; }
}

/* 向上滑动淡入 */
@keyframes slideUpFade {
  from {
    opacity: 0;
    transform: translateY(20px);
  }
  to {
    opacity: 1;
    transform: translateY(0);
  }
}

/* 缩放淡入 */
@keyframes scaleFade {
  from {
    opacity: 0;
    transform: scale(0.95);
  }
  to {
    opacity: 1;
    transform: scale(1);
  }
}
```

---

## 🎭 特殊效果

### 渐变背景
```css
/* 主渐变 */
.gradient-primary {
  background: linear-gradient(135deg, #FF6B35 0%, #FF8B62 100%);
}

/* 深色渐变 */
.gradient-dark {
  background: linear-gradient(180deg, #2C3E50 0%, #1A252F 100%);
}

/* 柔和渐变 */
.gradient-soft {
  background: linear-gradient(135deg, #F9FAFB 0%, #F3F4F6 100%);
}
```

### 阴影系统
```css
--shadow-sm: 0 1px 2px 0 rgba(0, 0, 0, 0.05);
--shadow-base: 0 4px 6px -1px rgba(0, 0, 0, 0.1), 0 2px 4px -1px rgba(0, 0, 0, 0.06);
--shadow-md: 0 10px 15px -3px rgba(0, 0, 0, 0.1), 0 4px 6px -2px rgba(0, 0, 0, 0.05);
--shadow-lg: 0 20px 25px -5px rgba(0, 0, 0, 0.1), 0 10px 10px -5px rgba(0, 0, 0, 0.04);
--shadow-xl: 0 25px 50px -12px rgba(0, 0, 0, 0.25);
--shadow-glow: 0 0 20px rgba(255, 107, 53, 0.3);
```

### 模糊效果
```css
.backdrop-blur {
  backdrop-filter: blur(12px);
  -webkit-backdrop-filter: blur(12px);
  background: rgba(255, 255, 255, 0.8);
}
```

---

## 📱 响应式断点

```css
/* 移动设备 */
@media (max-width: 640px) { /* sm */ }

/* 平板设备 */
@media (max-width: 768px) { /* md */ }

/* 桌面设备 */
@media (max-width: 1024px) { /* lg */ }

/* 大屏设备 */
@media (max-width: 1280px) { /* xl */ }

/* 超大屏设备 */
@media (min-width: 1536px) { /* 2xl */ }
```

---

## 🎪 插图与图标

### 插图风格
- 使用友好的、扁平化的插图风格
- 鸭子主题的角色设计
- 柔和的圆角和形状
- 明亮但不刺眼的配色

### 图标系统
- 使用轮廓图标作为主要风格
- 填充图标用于激活状态
- 保持 2px 描边宽度的一致性
- 圆角端点 (round caps) 和连接点 (round joins)

---

## 📝 使用示例

### Hero Section
```html
<section class="hero">
  <div class="hero-content">
    <span class="badge badge-primary">New Feature</span>
    <h1 class="text-display-lg">Infrastructure for Answers</h1>
    <p class="text-body-lg">
      The data warehouse built for answers, in SQL or natural language.
    </p>
    <div class="hero-actions">
      <button class="btn-primary">Try Free</button>
      <button class="btn-secondary">Book a Demo</button>
    </div>
  </div>
  <div class="hero-visual">
    <!-- 插图或动画 -->
  </div>
</section>
```

### Feature Card
```html
<div class="card card-interactive">
  <div class="card-icon">
    <!-- 图标 -->
  </div>
  <h3 class="text-h4">Data Warehouse + AI</h3>
  <p class="text-body-md">
    Scale per-user compute nodes independently, serving sub-second latency.
  </p>
  <a href="#" class="btn-text">Learn more →</a>
</div>
```

---

## 🔧 Tailwind 配置

要在你的项目中使用这个设计系统,更新 `tailwind.config.js`:

```javascript
module.exports = {
  theme: {
    extend: {
      colors: {
        primary: {
          DEFAULT: '#FF6B35',
          dark: '#E85A2E',
          light: '#FF8B62',
        },
        secondary: {
          DEFAULT: '#2C3E50',
          dark: '#1A252F',
          light: '#34495E',
        },
        accent: {
          DEFAULT: '#F7B731',
          light: '#FFD93D',
        },
      },
      fontFamily: {
        heading: ['Aeonik', 'Space Grotesk', 'sans-serif'],
        body: ['Inter', 'sans-serif'],
        mono: ['Aeonik Mono', 'Fira Code', 'monospace'],
      },
      spacing: {
        '18': '72px',
        '22': '88px',
      },
      boxShadow: {
        'glow': '0 0 20px rgba(255, 107, 53, 0.3)',
      },
      transitionTimingFunction: {
        'out-cubic': 'cubic-bezier(0.33, 1, 0.68, 1)',
        'in-out-cubic': 'cubic-bezier(0.65, 0, 0.35, 1)',
      },
    },
  },
}
```

---

## ✨ 最佳实践

1. **颜色使用**: 主色用于关键操作和强调,避免过度使用导致视觉疲劳
2. **排版层级**: 严格遵守排版系统,保持视觉一致性
3. **留白**: 使用充足的留白,让内容呼吸
4. **动画**: 动画应该有意义,增强用户体验而不是分散注意力
5. **可访问性**: 确保颜色对比度符合 WCAG AA 标准 (4.5:1)
6. **性能**: 优先使用 CSS 动画而非 JavaScript,使用 transform 和 opacity 属性

---

## 📚 相关资源

- [MotherDuck 官网](https://motherduck.com/)
- [Aeonik 字体](https://aeonik.co/)
- [DuckDB 文档](https://duckdb.org/docs/)
