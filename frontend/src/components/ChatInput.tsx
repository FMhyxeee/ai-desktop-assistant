import React, { useEffect, useRef, useState } from 'react';
import { ImagePlus, Send, Square, X } from 'lucide-react';
import { parseSlashInput } from '../lib/input';
import type { InputCard, InputImageAttachment } from '../types';

interface ChatInputProps {
  onSend: (input: InputCard) => void;
  disabled?: boolean;
  isStreaming?: boolean;
  onCancel?: () => void;
  draftInput?: InputCard | null;
  onDraftConsumed?: () => void;
}

const ChatInput: React.FC<ChatInputProps> = ({
  onSend,
  disabled = false,
  isStreaming = false,
  onCancel,
  draftInput,
  onDraftConsumed,
}) => {
  const fileInputRef = useRef<HTMLInputElement>(null);
  const textareaRef = useRef<HTMLTextAreaElement>(null);

  const [input, setInput] = useState('');
  const [images, setImages] = useState<InputImageAttachment[]>([]);
  const [isReadingImages, setIsReadingImages] = useState(false);

  useEffect(() => {
    if (!textareaRef.current) {
      return;
    }
    textareaRef.current.style.height = 'auto';
    textareaRef.current.style.height = `${Math.min(textareaRef.current.scrollHeight, 220)}px`;
  }, [input]);

  useEffect(() => {
    if (!draftInput) {
      return;
    }

    setInput(draftInput.content);
    setImages(draftInput.images ?? []);
    onDraftConsumed?.();

    requestAnimationFrame(() => {
      textareaRef.current?.focus();
      const textarea = textareaRef.current;
      if (!textarea) {
        return;
      }
      const length = textarea.value.length;
      textarea.setSelectionRange(length, length);
    });
  }, [draftInput, onDraftConsumed]);

  const handlePickImage = () => {
    fileInputRef.current?.click();
  };

  const handleImagesSelected = async (event: React.ChangeEvent<HTMLInputElement>) => {
    const files = Array.from(event.target.files ?? []);
    event.target.value = '';
    if (files.length === 0) {
      return;
    }

    setIsReadingImages(true);
    const nextAttachments: InputImageAttachment[] = [];

    for (const file of files) {
      if (!file.type.startsWith('image/')) {
        continue;
      }

      try {
        const dataUrl = await new Promise<string>((resolve, reject) => {
          const reader = new FileReader();
          reader.onload = () => {
            if (typeof reader.result === 'string') {
              resolve(reader.result);
              return;
            }
            reject(new Error('Failed to read image file'));
          };
          reader.onerror = () => reject(reader.error ?? new Error('Failed to read image file'));
          reader.readAsDataURL(file);
        });

        const attachmentId =
          typeof crypto !== 'undefined' && typeof crypto.randomUUID === 'function'
            ? crypto.randomUUID()
            : `img-${Date.now()}-${Math.random().toString(36).slice(2)}`;

        nextAttachments.push({
          id: attachmentId,
          name: file.name,
          mimeType: file.type || 'application/octet-stream',
          dataUrl,
          sizeBytes: file.size,
        });
      } catch {
        // Skip unreadable image files and keep processing the rest.
      }
    }

    if (nextAttachments.length > 0) {
      setImages((current) => [...current, ...nextAttachments]);
    }

    setIsReadingImages(false);
  };

  const removeImage = (imageId: string) => {
    setImages((current) => current.filter((item) => item.id !== imageId));
  };

  const handleSend = () => {
    const parsedInput = parseSlashInput(input);
    const normalizedContent = parsedInput.normalizedContent;
    const hasText = normalizedContent.length > 0;
    const hasImages = images.length > 0;

    if ((!hasText && !hasImages) || disabled || isReadingImages) {
      return;
    }

    if (parsedInput.isCommand && hasImages) {
      window.alert('Command input cannot include image attachments.');
      return;
    }

    if (parsedInput.isCommand) {
      const confirmed = window.confirm(
        `Confirm shell command execution?\n\n${parsedInput.command ?? ''}`
      );
      if (!confirmed) {
        return;
      }
    }

    onSend({
      content: normalizedContent,
      images: parsedInput.isCommand ? undefined : images,
    });

    setInput('');
    setImages([]);

    if (textareaRef.current) {
      textareaRef.current.style.height = 'auto';
    }
  };

  const handleKeyDown = (event: React.KeyboardEvent<HTMLTextAreaElement>) => {
    if (event.key === 'Enter' && !event.shiftKey) {
      event.preventDefault();
      handleSend();
    }
  };

  const parsedInput = parseSlashInput(input);
  const hasSendableInput = parsedInput.normalizedContent.length > 0 || images.length > 0;

  return (
    <div className="composer-wrap">
      <div className="composer-panel">
        <div className="composer-media-row">
          <button
            type="button"
            className="composer-image-btn"
            onClick={handlePickImage}
            disabled={disabled || isStreaming}
          >
            <ImagePlus size={14} />
            Add image
          </button>
          <span className="composer-media-note">
            {isReadingImages ? 'Reading images...' : `${images.length} image(s) attached`}
          </span>
        </div>

        <input
          ref={fileInputRef}
          type="file"
          accept="image/*"
          multiple
          onChange={handleImagesSelected}
          className="composer-file-input"
          tabIndex={-1}
        />

        {images.length > 0 && (
          <div className="composer-attachments">
            {images.map((image) => (
              <div key={image.id} className="composer-attachment">
                <img src={image.dataUrl} alt={image.name} className="composer-attachment-preview" />
                <div className="composer-attachment-meta">
                  <span className="composer-attachment-name">{image.name}</span>
                  <span className="composer-attachment-size">{(image.sizeBytes / 1024).toFixed(1)} KB</span>
                </div>
                <button
                  type="button"
                  className="composer-attachment-remove"
                  onClick={() => removeImage(image.id)}
                  aria-label={`Remove ${image.name}`}
                >
                  <X size={14} />
                </button>
              </div>
            ))}
          </div>
        )}

        <textarea
          ref={textareaRef}
          value={input}
          onChange={(event) => setInput(event.target.value)}
          onKeyDown={handleKeyDown}
          placeholder="Type a message... (/command to run shell, // to send text starting with /)"
          disabled={disabled}
          rows={1}
          className="composer-input"
        />

        <div className="composer-controls">
          <div className="composer-hint">
            <span>
              {isStreaming
                ? 'Streaming response in progress...'
                : parsedInput.isCommand
                  ? 'Command mode: will trigger run_user_shell_command'
                  : 'Message mode: will trigger user_turn'}
            </span>
            <span className="composer-count">{input.length}</span>
          </div>

          {isStreaming ? (
            <button
              type="button"
              onClick={onCancel}
              className="composer-button stop"
              disabled={!onCancel}
            >
              <Square size={16} />
              Stop
            </button>
          ) : (
            <button
              type="button"
              onClick={handleSend}
              className="composer-button primary"
              disabled={disabled || !hasSendableInput || isReadingImages}
            >
              <Send size={16} />
              Send
            </button>
          )}
        </div>
      </div>
    </div>
  );
};

export default ChatInput;

