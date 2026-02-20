import React, { useEffect, useMemo, useRef, useState } from 'react';
import ReactMarkdown from 'react-markdown';
import remarkGfm from 'remark-gfm';

type PrismStyle = Record<string, React.CSSProperties>;

type PrismHighlighterProps = {
  style?: PrismStyle;
  language?: string;
  PreTag?: React.ElementType;
  className?: string;
  customStyle?: React.CSSProperties;
  children?: React.ReactNode;
};

type PrismLightComponent = React.ComponentType<PrismHighlighterProps> & {
  registerLanguage?: (name: string, syntax: unknown) => void;
};

interface MarkdownMessageProps {
  content: string;
  className?: string;
}

type MarkdownCodeProps = React.ComponentProps<'code'> & {
  inline?: boolean;
  node?: unknown;
  className?: string;
  children?: React.ReactNode;
};

type MarkdownAnchorProps = React.ComponentProps<'a'> & {
  node?: unknown;
  href?: string;
  children?: React.ReactNode;
};

type MarkdownBlockquoteProps = React.ComponentProps<'blockquote'> & {
  node?: unknown;
  children?: React.ReactNode;
};

const LANGUAGE_ALIAS: Record<string, string> = {
  js: 'javascript',
  jsx: 'jsx',
  ts: 'typescript',
  tsx: 'tsx',
  py: 'python',
  sh: 'bash',
  shell: 'bash',
  zsh: 'bash',
  csharp: 'csharp',
  cs: 'csharp',
  yml: 'yaml',
};

const LANGUAGE_LOADERS: Record<string, () => Promise<{ default: unknown }>> = {
  javascript: () => import('react-syntax-highlighter/dist/esm/languages/prism/javascript'),
  jsx: () => import('react-syntax-highlighter/dist/esm/languages/prism/jsx'),
  typescript: () => import('react-syntax-highlighter/dist/esm/languages/prism/typescript'),
  tsx: () => import('react-syntax-highlighter/dist/esm/languages/prism/tsx'),
  python: () => import('react-syntax-highlighter/dist/esm/languages/prism/python'),
  rust: () => import('react-syntax-highlighter/dist/esm/languages/prism/rust'),
  bash: () => import('react-syntax-highlighter/dist/esm/languages/prism/bash'),
  json: () => import('react-syntax-highlighter/dist/esm/languages/prism/json'),
  yaml: () => import('react-syntax-highlighter/dist/esm/languages/prism/yaml'),
  markdown: () => import('react-syntax-highlighter/dist/esm/languages/prism/markdown'),
  css: () => import('react-syntax-highlighter/dist/esm/languages/prism/css'),
  sql: () => import('react-syntax-highlighter/dist/esm/languages/prism/sql'),
  go: () => import('react-syntax-highlighter/dist/esm/languages/prism/go'),
  java: () => import('react-syntax-highlighter/dist/esm/languages/prism/java'),
  c: () => import('react-syntax-highlighter/dist/esm/languages/prism/c'),
  cpp: () => import('react-syntax-highlighter/dist/esm/languages/prism/cpp'),
};

const normalizeLanguage = (lang: string): string => {
  const lowered = lang.trim().toLowerCase();
  return LANGUAGE_ALIAS[lowered] ?? lowered;
};

const detectLanguages = (markdown: string): string[] => {
  const matches = markdown.match(/```([a-zA-Z0-9_-]+)/g) ?? [];
  const normalized = matches
    .map((match) => match.replace('```', '').trim())
    .map(normalizeLanguage)
    .filter((lang) => lang.length > 0 && lang in LANGUAGE_LOADERS);

  return Array.from(new Set(normalized));
};

const MarkdownMessage: React.FC<MarkdownMessageProps> = ({ content, className = '' }) => {
  const [highlighter, setHighlighter] = useState<PrismLightComponent | null>(null);
  const [highlighterStyle, setHighlighterStyle] = useState<PrismStyle | null>(null);
  const registeredLanguagesRef = useRef<Set<string>>(new Set());

  const codeLanguages = useMemo(() => detectLanguages(content), [content]);

  useEffect(() => {
    let disposed = false;

    const loadHighlighter = async () => {
      if (codeLanguages.length === 0) {
        return;
      }

      const [prismModule, styleModule] = await Promise.all([
        import('react-syntax-highlighter/dist/esm/prism-light'),
        import('react-syntax-highlighter/dist/esm/styles/prism'),
      ]);

      if (disposed) {
        return;
      }

      const PrismLight = prismModule.PrismLight as PrismLightComponent;
      setHighlighter(() => PrismLight);
      setHighlighterStyle(styleModule.vscDarkPlus as PrismStyle);

      await Promise.all(
        codeLanguages.map(async (language) => {
          if (registeredLanguagesRef.current.has(language)) {
            return;
          }

          const loader = LANGUAGE_LOADERS[language];
          if (!loader) {
            return;
          }

          try {
            const syntaxModule = await loader();
            PrismLight.registerLanguage?.(language, syntaxModule.default);
            registeredLanguagesRef.current.add(language);
          } catch {
            // Keep fallback rendering if a language module fails.
          }
        })
      );
    };

    void loadHighlighter();

    return () => {
      disposed = true;
    };
  }, [codeLanguages]);

  return (
    <div className={`markdown-content ${className}`}>
      <ReactMarkdown
        remarkPlugins={[remarkGfm]}
        components={{
          code({ inline, className, children, ...props }: MarkdownCodeProps) {
            const match = /language-(\w+)/.exec(className || '');
            const language = match ? normalizeLanguage(match[1]) : '';

            if (!inline && language && highlighter && highlighterStyle) {
              const Highlighter = highlighter;

              return (
                <Highlighter
                  style={highlighterStyle}
                  language={language}
                  PreTag="div"
                  className="rounded-lg"
                  customStyle={{ borderRadius: '10px', margin: '0.9em 0' }}
                >
                  {String(children).replace(/\n$/, '')}
                </Highlighter>
              );
            }

            if (!inline) {
              return (
                <pre className="markdown-pre">
                  <code className={`markdown-code-block ${className || ''}`} {...props}>
                    {children}
                  </code>
                </pre>
              );
            }

            return (
              <code className="inline-code" {...props}>
                {children}
              </code>
            );
          },
          a({ children, href, ...props }: MarkdownAnchorProps) {
            return (
              <a
                href={href}
                target="_blank"
                rel="noopener noreferrer"
                className="markdown-link"
                {...props}
              >
                {children}
              </a>
            );
          },
          blockquote({ children, ...props }: MarkdownBlockquoteProps) {
            return (
              <blockquote className="markdown-quote" {...props}>
                {children}
              </blockquote>
            );
          },
        }}
      >
        {content}
      </ReactMarkdown>
    </div>
  );
};

export default MarkdownMessage;
