export interface ParsedSlashInput {
  isCommand: boolean;
  command: string | null;
  normalizedContent: string;
}

export const parseSlashInput = (rawContent: string): ParsedSlashInput => {
  const trimmed = rawContent.trim();
  if (trimmed.length === 0) {
    return {
      isCommand: false,
      command: null,
      normalizedContent: trimmed,
    };
  }

  if (!trimmed.startsWith('/')) {
    return {
      isCommand: false,
      command: null,
      normalizedContent: trimmed,
    };
  }

  if (trimmed.startsWith('//')) {
    return {
      isCommand: false,
      command: null,
      normalizedContent: trimmed.slice(1),
    };
  }

  const command = trimmed.slice(1).trim();
  if (command.length === 0) {
    return {
      isCommand: false,
      command: null,
      normalizedContent: trimmed,
    };
  }

  return {
    isCommand: true,
    command,
    normalizedContent: `/${command}`,
  };
};
