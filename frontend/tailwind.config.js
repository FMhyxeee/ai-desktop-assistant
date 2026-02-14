/** @type {import('tailwindcss').Config} */
export default {
  content: [
    "./index.html",
    "./src/**/*.{js,ts,jsx,tsx}",
  ],
  theme: {
    extend: {
      colors: {
        // 主色板 (Primary Palette) - MotherDuck 风格橙红色
        primary: {
          DEFAULT: '#FF6B35',
          dark: '#E85A2E',
          light: '#FF8B62',
          50: '#FFF7ED',
          100: '#FFEDD5',
          200: '#FED7AA',
          300: '#FDBA74',
          400: '#FB923C',
          500: '#FF6B35',
          600: '#E85A2E',
          700: '#C2410C',
          800: '#9A3412',
          900: '#7C2D12',
          950: '#431407',
        },
        // 辅助色 (Secondary Palette) - 深海蓝
        secondary: {
          DEFAULT: '#2C3E50',
          dark: '#1A252F',
          light: '#34495E',
        },
        // 强调色 (Accent Palette) - 明亮黄
        accent: {
          DEFAULT: '#F7B731',
          light: '#FFD93D',
        },
        // 中性色板 (Neutral Palette)
        neutral: {
          50: '#FAFAFA',
          100: '#F9FAFB',
          200: '#F3F4F6',
          300: '#E5E7EB',
          400: '#D1D5DB',
          500: '#9CA3AF',
          600: '#6B7280',
          700: '#4B5563',
          800: '#1F2937',
          900: '#111827',
          950: '#030712',
        },
        // 文本颜色
        text: {
          primary: '#1F2937',
          secondary: '#6B7280',
          tertiary: '#9CA3AF',
          inverse: '#FFFFFF',
        },
        // 背景颜色
        bg: {
          primary: '#FFFFFF',
          secondary: '#F9FAFB',
          tertiary: '#F3F4F6',
          inverse: '#1F2937',
        },
        // 边框颜色
        border: {
          primary: '#E5E7EB',
          secondary: '#D1D5DB',
          focus: '#FF6B35',
        },
        // 语义色板 (Semantic Palette)
        success: {
          DEFAULT: '#10B981',
          bg: '#D1FAE5',
        },
        warning: {
          DEFAULT: '#F59E0B',
          bg: '#FEF3C7',
        },
        error: {
          DEFAULT: '#EF4444',
          bg: '#FEE2E2',
        },
        info: {
          DEFAULT: '#3B82F6',
          bg: '#DBEAFE',
        },
      },
      fontFamily: {
        // 标题字体 - 几何无衬线字体
        heading: [
          '"Space Grotesk"',
          '"Plus Jakarta Sans"',
          '"Segoe UI"',
          'system-ui',
          'sans-serif',
        ],
        // 正文字体 - 现代无衬线字体
        body: [
          '"Inter"',
          '"SF Pro Display"',
          '-apple-system',
          'sans-serif',
        ],
        // 等宽字体 - 代码和数据
        mono: [
          '"Fira Code"',
          '"JetBrains Mono"',
          'monospace',
        ],
        sans: [
          '"Inter"',
          '"Segoe UI"',
          '"Microsoft YaHei UI"',
          '-apple-system',
          'BlinkMacSystemFont',
          'sans-serif',
        ],
      },
      fontSize: {
        // Display - 超大标题
        'display-xl': ['72px', { lineHeight: '1', fontWeight: '700', letterSpacing: '-0.02em' }],
        'display-lg': ['56px', { lineHeight: '1.1', fontWeight: '700', letterSpacing: '-0.015em' }],
        'display-md': ['44px', { lineHeight: '1.2', fontWeight: '700', letterSpacing: '-0.01em' }],
        // Heading - 页面标题
        'h1': ['36px', { lineHeight: '1.3', fontWeight: '700', letterSpacing: '-0.01em' }],
        'h2': ['30px', { lineHeight: '1.4', fontWeight: '600', letterSpacing: '-0.005em' }],
        'h3': ['24px', { lineHeight: '1.5', fontWeight: '600' }],
        'h4': ['20px', { lineHeight: '1.5', fontWeight: '600' }],
        // Body - 正文
        'body-xl': ['20px', { lineHeight: '1.6', fontWeight: '400' }],
        'body-lg': ['18px', { lineHeight: '1.6', fontWeight: '400' }],
        'body-md': ['16px', { lineHeight: '1.7', fontWeight: '400' }],
        'body-sm': ['14px', { lineHeight: '1.7', fontWeight: '400' }],
        // Caption - 辅助文本
        'caption': ['12px', { lineHeight: '1.5', fontWeight: '500' }],
        'overline': ['11px', { lineHeight: '1.5', fontWeight: '600', letterSpacing: '0.06em', textTransform: 'uppercase' }],
      },
      spacing: {
        '18': '72px',
        '22': '88px',
      },
      borderRadius: {
        '4xl': '24px',
      },
      boxShadow: {
        'sm': '0 1px 2px 0 rgba(0, 0, 0, 0.05)',
        'base': '0 4px 6px -1px rgba(0, 0, 0, 0.1), 0 2px 4px -1px rgba(0, 0, 0, 0.06)',
        'md': '0 10px 15px -3px rgba(0, 0, 0, 0.1), 0 4px 6px -2px rgba(0, 0, 0, 0.05)',
        'lg': '0 20px 25px -5px rgba(0, 0, 0, 0.1), 0 10px 10px -5px rgba(0, 0, 0, 0.04)',
        'xl': '0 25px 50px -12px rgba(0, 0, 0, 0.25)',
        'glow': '0 0 20px rgba(255, 107, 53, 0.3)',
        'soft': '0 4px 6px rgba(0, 0, 0, 0.05), 0 10px 20px rgba(0, 0, 0, 0.03)',
      },
      transitionTimingFunction: {
        'out-cubic': 'cubic-bezier(0.33, 1, 0.68, 1)',
        'in-out-cubic': 'cubic-bezier(0.65, 0, 0.35, 1)',
        'out-quart': 'cubic-bezier(0.25, 1, 0.5, 1)',
      },
      transitionDuration: {
        'fast': '150ms',
        'base': '250ms',
        'slow': '350ms',
        'slower': '500ms',
      },
      animation: {
        'fade-in': 'fadeIn 0.2s ease-in-out',
        'slide-up': 'slideUp 0.3s ease-out',
        'slide-up-fade': 'slideUpFade 0.3s cubic-bezier(0.4, 0, 0.2, 1)',
        'scale-fade': 'scaleFade 0.3s cubic-bezier(0.4, 0, 0.2, 1)',
        'pulse-slow': 'pulse 3s cubic-bezier(0.4, 0, 0.6, 1) infinite',
      },
      keyframes: {
        fadeIn: {
          '0%': { opacity: '0' },
          '100%': { opacity: '1' },
        },
        slideUp: {
          '0%': { transform: 'translateY(10px)', opacity: '0' },
          '100%': { transform: 'translateY(0)', opacity: '1' },
        },
        slideUpFade: {
          '0%': { opacity: '0', transform: 'translateY(20px)' },
          '100%': { opacity: '1', transform: 'translateY(0)' },
        },
        scaleFade: {
          '0%': { opacity: '0', transform: 'scale(0.95)' },
          '100%': { opacity: '1', transform: 'scale(1)' },
        },
      },
    },
  },
  plugins: [],
}
