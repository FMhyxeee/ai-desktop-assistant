// MotherDuck 风格 React 组件库
// 使用 Tailwind CSS + Headless UI

import React from 'react';
import { motion } from 'framer-motion';

// ==================== Button 组件 ====================
export const Button = ({
  variant = 'primary',
  size = 'md',
  children,
  className = '',
  ...props
}) => {
  const baseStyles = 'inline-flex items-center justify-center font-semibold rounded-xl transition-all duration-300 cursor-pointer';

  const variants = {
    primary: 'bg-gradient-to-r from-orange-500 to-orange-400 text-white shadow-lg shadow-orange-500/30 hover:shadow-xl hover:shadow-orange-500/40 hover:-translate-y-0.5 active:translate-y-0',
    secondary: 'bg-white text-orange-500 border-2 border-orange-500 hover:bg-orange-500 hover:text-white',
    text: 'bg-transparent text-orange-500 hover:bg-orange-500/10 rounded-lg',
  };

  const sizes = {
    sm: 'px-4 py-2 text-sm',
    md: 'px-7 py-3.5 text-base',
    lg: 'px-8 py-4 text-lg',
  };

  return (
    <motion.button
      whileHover={{ scale: 1.02 }}
      whileTap={{ scale: 0.98 }}
      className={`${baseStyles} ${variants[variant]} ${sizes[size]} ${className}`}
      {...props}
    >
      {children}
    </motion.button>
  );
};

// ==================== Card 组件 ====================
export const Card = ({
  children,
  interactive = false,
  className = '',
  ...props
}) => {
  return (
    <motion.div
      whileHover={interactive ? { y: -8 } : {}}
      className={`bg-white rounded-2xl p-6 shadow-md border border-gray-200 transition-all duration-300 ${interactive ? 'cursor-pointer hover:shadow-xl hover:border-orange-500' : ''} ${className}`}
      {...props}
    >
      {children}
    </motion.div>
  );
};

// ==================== Badge 组件 ====================
export const Badge = ({
  variant = 'primary',
  children,
  className = '',
}) => {
  const variants = {
    primary: 'bg-orange-500/10 text-orange-500',
    success: 'bg-green-100 text-green-600',
    warning: 'bg-yellow-100 text-yellow-600',
    error: 'bg-red-100 text-red-600',
    info: 'bg-blue-100 text-blue-600',
  };

  return (
    <span className={`inline-flex items-center px-3 py-1.5 rounded-full text-xs font-semibold tracking-wide ${variants[variant]} ${className}`}>
      {children}
    </span>
  );
};

// ==================== Input 组件 ====================
export const Input = ({
  label,
  error,
  className = '',
  ...props
}) => {
  return (
    <div className="w-full">
      {label && (
        <label className="block text-sm font-medium text-gray-700 mb-2">
          {label}
        </label>
      )}
      <input
        className={`w-full px-4 py-3.5 text-base border-2 border-gray-200 rounded-xl focus:outline-none focus:border-orange-500 focus:ring-4 focus:ring-orange-500/10 transition-all duration-200 ${error ? 'border-red-500' : ''} ${className}`}
        {...props}
      />
      {error && (
        <p className="mt-2 text-sm text-red-600">{error}</p>
      )}
    </div>
  );
};

// ==================== Hero Section 组件 ====================
export const HeroSection = ({
  badge,
  title,
  description,
  primaryAction,
  secondaryAction,
  illustration,
}) => {
  return (
    <section className="relative overflow-hidden bg-gradient-to-b from-white to-gray-50 py-24">
      {/* 装饰性背景 */}
      <div className="absolute top-0 right-0 w-96 h-96 bg-orange-500/10 rounded-full blur-3xl -translate-y-1/2 translate-x-1/2" />
      <div className="absolute bottom-0 left-0 w-96 h-96 bg-blue-500/10 rounded-full blur-3xl translate-y-1/2 -translate-x-1/2" />

      <div className="container mx-auto px-4 relative z-10">
        <motion.div
          initial={{ opacity: 0, y: 20 }}
          animate={{ opacity: 1, y: 0 }}
          transition={{ duration: 0.6 }}
          className="max-w-4xl mx-auto text-center"
        >
          {badge && <Badge variant="primary" className="mb-6">{badge}</Badge>}

          <h1 className="text-6xl md:text-7xl font-bold mb-8 bg-gradient-to-r from-orange-500 to-slate-700 bg-clip-text text-transparent leading-tight">
            {title}
          </h1>

          <p className="text-xl text-gray-600 mb-12 leading-relaxed max-w-2xl mx-auto">
            {description}
          </p>

          <div className="flex flex-col sm:flex-row gap-4 justify-center">
            {primaryAction}
            {secondaryAction}
          </div>
        </motion.div>

        {illustration && (
          <motion.div
            initial={{ opacity: 0, scale: 0.95 }}
            animate={{ opacity: 1, scale: 1 }}
            transition={{ duration: 0.8, delay: 0.2 }}
            className="mt-16"
          >
            {illustration}
          </motion.div>
        )}
      </div>
    </section>
  );
};

// ==================== FeatureCard 组件 ====================
export const FeatureCard = ({
  icon,
  title,
  description,
  link,
  delay = 0,
}) => {
  return (
    <motion.div
      initial={{ opacity: 0, y: 20 }}
      whileInView={{ opacity: 1, y: 0 }}
      viewport={{ once: true }}
      transition={{ duration: 0.5, delay }}
    >
      <Card interactive>
        {icon && (
          <div className="w-16 h-16 bg-gradient-to-br from-orange-500 to-orange-400 rounded-2xl flex items-center justify-center text-white text-2xl mb-6 shadow-lg shadow-orange-500/30">
            {icon}
          </div>
        )}
        <h3 className="text-2xl font-semibold mb-4 text-gray-900">{title}</h3>
        <p className="text-gray-600 mb-6 leading-relaxed">{description}</p>
        {link && (
          <a href={link.href} className="text-orange-500 font-semibold hover:text-orange-600 inline-flex items-center gap-2 group">
            {link.text}
            <span className="group-hover:translate-x-1 transition-transform">→</span>
          </a>
        )}
      </Card>
    </motion.div>
  );
};

// ==================== Section 组件 ====================
export const Section = ({
  children,
  className = '',
  container = true,
  ...props
}) => {
  return (
    <section className={`py-16 md:py-24 ${className}`} {...props}>
      {container ? (
        <div className="container mx-auto px-4">
          {children}
        </div>
      ) : children}
    </section>
  );
};

// ==================== Grid 组件 ====================
export const Grid = ({
  children,
  cols = 3,
  gap = 6,
  className = '',
}) => {
  const gridCols = {
    2: 'grid-cols-1 md:grid-cols-2',
    3: 'grid-cols-1 md:grid-cols-2 lg:grid-cols-3',
    4: 'grid-cols-1 md:grid-cols-2 lg:grid-cols-4',
  };

  return (
    <div className={`grid ${gridCols[cols]} gap-${gap} ${className}`}>
      {children}
    </div>
  );
};

// ==================== Tag 组件 ====================
export const Tag = ({
  children,
  onRemove,
  className = '',
}) => {
  return (
    <span className={`inline-flex items-center gap-2 px-3 py-1.5 bg-orange-500/10 text-orange-500 rounded-lg text-sm font-medium ${className}`}>
      {children}
      {onRemove && (
        <button
          onClick={onRemove}
          className="hover:bg-orange-500/20 rounded-full p-0.5 transition-colors"
        >
          <svg className="w-4 h-4" fill="none" viewBox="0 0 24 24" stroke="currentColor">
            <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M6 18L18 6M6 6l12 12" />
          </svg>
        </button>
      )}
    </span>
  );
};

// ==================== Select 组件 ====================
export const Select = ({
  label,
  options,
  className = '',
  ...props
}) => {
  return (
    <div className="w-full">
      {label && (
        <label className="block text-sm font-medium text-gray-700 mb-2">
          {label}
        </label>
      )}
      <select
        className={`w-full px-4 py-3.5 text-base border-2 border-gray-200 rounded-xl focus:outline-none focus:border-orange-500 focus:ring-4 focus:ring-orange-500/10 transition-all duration-200 bg-white ${className}`}
        {...props}
      >
        {options.map((option) => (
          <option key={option.value} value={option.value}>
            {option.label}
          </option>
        ))}
      </select>
    </div>
  );
};

// ==================== TextArea 组件 ====================
export const TextArea = ({
  label,
  error,
  rows = 4,
  className = '',
  ...props
}) => {
  return (
    <div className="w-full">
      {label && (
        <label className="block text-sm font-medium text-gray-700 mb-2">
          {label}
        </label>
      )}
      <textarea
        rows={rows}
        className={`w-full px-4 py-3.5 text-base border-2 border-gray-200 rounded-xl focus:outline-none focus:border-orange-500 focus:ring-4 focus:ring-orange-500/10 transition-all duration-200 resize-none ${error ? 'border-red-500' : ''} ${className}`}
        {...props}
      />
      {error && (
        <p className="mt-2 text-sm text-red-600">{error}</p>
      )}
    </div>
  );
};

// ==================== Checkbox 组件 ====================
export const Checkbox = ({
  label,
  checked,
  onChange,
  className = '',
}) => {
  return (
    <label className={`flex items-center gap-3 cursor-pointer ${className}`}>
      <input
        type="checkbox"
        checked={checked}
        onChange={onChange}
        className="w-5 h-5 text-orange-500 border-2 border-gray-300 rounded focus:ring-4 focus:ring-orange-500/10 focus:border-orange-500 transition-all"
      />
      <span className="text-gray-700">{label}</span>
    </label>
  );
};

// ==================== RadioGroup 组件 ====================
export const RadioGroup = ({
  label,
  options,
  value,
  onChange,
  className = '',
}) => {
  return (
    <div className={className}>
      {label && (
        <label className="block text-sm font-medium text-gray-700 mb-3">
          {label}
        </label>
      )}
      <div className="space-y-3">
        {options.map((option) => (
          <label key={option.value} className="flex items-center gap-3 cursor-pointer">
            <input
              type="radio"
              name={label}
              value={option.value}
              checked={value === option.value}
              onChange={(e) => onChange(e.target.value)}
              className="w-5 h-5 text-orange-500 border-2 border-gray-300 focus:ring-4 focus:ring-orange-500/10 focus:border-orange-500 transition-all"
            />
            <span className="text-gray-700">{option.label}</span>
          </label>
        ))}
      </div>
    </div>
  );
};

// ==================== Progress 组件 ====================
export const Progress = ({
  value,
  max = 100,
  className = '',
}) => {
  const percentage = Math.min(Math.max((value / max) * 100, 0), 100);

  return (
    <div className={`w-full bg-gray-200 rounded-full h-2 overflow-hidden ${className}`}>
      <motion.div
        initial={{ width: 0 }}
        animate={{ width: `${percentage}%` }}
        transition={{ duration: 0.5, ease: 'easeOut' }}
        className="h-full bg-gradient-to-r from-orange-500 to-orange-400 rounded-full"
      />
    </div>
  );
};

// ==================== Tooltip 组件 ====================
export const Tooltip = ({
  content,
  children,
  position = 'top',
}) => {
  return (
    <div className="relative inline-block group">
      {children}
      <div className={`absolute ${position === 'top' ? 'bottom-full mb-2' : 'top-full mt-2'} left-1/2 -translate-x-1/2 px-3 py-2 bg-gray-900 text-white text-sm rounded-lg whitespace-nowrap opacity-0 group-hover:opacity-100 transition-opacity duration-200 pointer-events-none z-50`}>
        {content}
        <div className={`absolute ${position === 'top' ? 'top-full' : 'bottom-full'} left-1/2 -translate-x-1/2 border-8 border-transparent ${position === 'top' ? 'border-t-gray-900' : 'border-b-gray-900'}`} />
      </div>
    </div>
  );
};

// ==================== Avatar 组件 ====================
export const Avatar = ({
  src,
  alt,
  size = 'md',
  className = '',
}) => {
  const sizes = {
    sm: 'w-8 h-8',
    md: 'w-12 h-12',
    lg: 'w-16 h-16',
    xl: 'w-24 h-24',
  };

  return (
    <div className={`${sizes[size]} rounded-full overflow-hidden bg-gradient-to-br from-orange-500 to-orange-400 flex items-center justify-center ${className}`}>
      {src ? (
        <img src={src} alt={alt} className="w-full h-full object-cover" />
      ) : (
        <span className="text-white font-semibold text-lg">
          {alt?.charAt(0)?.toUpperCase()}
        </span>
      )}
    </div>
  );
};

// ==================== Divider 组件 ====================
export const Divider = ({
  label,
  className = '',
}) => {
  return (
    <div className={`flex items-center gap-4 ${className}`}>
      <div className="flex-1 h-px bg-gray-200" />
      {label && (
        <span className="text-sm text-gray-500 font-medium">{label}</span>
      )}
      <div className="flex-1 h-px bg-gray-200" />
    </div>
  );
};

// ==================== Alert 组件 ====================
export const Alert = ({
  variant = 'info',
  title,
  children,
  className = '',
}) => {
  const variants = {
    info: 'bg-blue-50 border-blue-200 text-blue-800',
    success: 'bg-green-50 border-green-200 text-green-800',
    warning: 'bg-yellow-50 border-yellow-200 text-yellow-800',
    error: 'bg-red-50 border-red-200 text-red-800',
  };

  const icons = {
    info: 'ℹ️',
    success: '✅',
    warning: '⚠️',
    error: '❌',
  };

  return (
    <div className={`p-4 rounded-xl border-2 ${variants[variant]} ${className}`}>
      <div className="flex gap-3">
        <span className="text-xl">{icons[variant]}</span>
        <div className="flex-1">
          {title && <h4 className="font-semibold mb-1">{title}</h4>}
          <div className="text-sm">{children}</div>
        </div>
      </div>
    </div>
  );
};

// ==================== Tabs 组件 ====================
export const Tabs = ({
  tabs,
  activeTab,
  onChange,
  className = '',
}) => {
  return (
    <div className={className}>
      <div className="flex gap-2 border-b-2 border-gray-200">
        {tabs.map((tab) => (
          <button
            key={tab.value}
            onClick={() => onChange(tab.value)}
            className={`px-6 py-3 font-semibold transition-all relative ${activeTab === tab.value
              ? 'text-orange-500'
              : 'text-gray-500 hover:text-gray-700'
            }`}
          >
            {tab.label}
            {activeTab === tab.value && (
              <motion.div
                layoutId="activeTab"
                className="absolute bottom-0 left-0 right-0 h-0.5 bg-orange-500"
              />
            )}
          </button>
        ))}
      </div>
    </div>
  );
};

// ==================== Skeleton 组件 ====================
export const Skeleton = ({
  className = '',
  ...props
}) => {
  return (
    <div
      className={`animate-pulse bg-gray-200 rounded-lg ${className}`}
      {...props}
    />
  );
};

// ==================== 使用示例 ====================
/*
import {
  HeroSection,
  Button,
  Card,
  FeatureCard,
  Section,
  Grid,
  Input,
  Badge
} from './components';

function App() {
  return (
    <>
      <HeroSection
        badge="🎨 New Design System"
        title="让设计变得简单有趣"
        description="一套现代、友好、专业的数据平台设计系统,充满活力又不失稳重"
        primaryAction={<Button>立即开始</Button>}
        secondaryAction={<Button variant="secondary">了解更多</Button>}
      />

      <Section>
        <Grid cols={3}>
          <FeatureCard
            icon="🚀"
            title="快速开发"
            description="基于 React + Tailwind CSS 的组件库,开箱即用"
            link={{ href: '/docs', text: '查看文档' }}
          />
          <FeatureCard
            icon="🎨"
            title="精美设计"
            description="参考 MotherDuck 设计风格,现代化的视觉体验"
            link={{ href: '/design', text: '探索设计' }}
          />
          <FeatureCard
            icon="⚡"
            title="高性能"
            description="优化的动画和交互,流畅的用户体验"
            link={{ href: '/performance', text: '了解更多' }}
          />
        </Grid>
      </Section>
    </>
  );
}
*/

export default {
  Button,
  Card,
  Badge,
  Input,
  HeroSection,
  FeatureCard,
  Section,
  Grid,
  Tag,
  Select,
  TextArea,
  Checkbox,
  RadioGroup,
  Progress,
  Tooltip,
  Avatar,
  Divider,
  Alert,
  Tabs,
  Skeleton,
};
