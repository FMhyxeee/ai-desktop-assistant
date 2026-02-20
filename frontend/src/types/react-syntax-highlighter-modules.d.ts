declare module 'react-syntax-highlighter/dist/esm/prism-light' {
  export const PrismLight: import('react').ComponentType<Record<string, unknown>> & {
    registerLanguage?: (name: string, syntax: unknown) => void;
  };
}

declare module 'react-syntax-highlighter/dist/esm/styles/prism' {
  export const vscDarkPlus: Record<string, import('react').CSSProperties>;
}

declare module 'react-syntax-highlighter/dist/esm/languages/prism/*' {
  const language: unknown;
  export default language;
}
