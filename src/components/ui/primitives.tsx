import type {
  ButtonHTMLAttributes,
  CSSProperties,
  InputHTMLAttributes,
  ReactNode,
  Ref,
} from 'react';
import { useEffect, useId, useRef } from 'react';
import {
  CircleAlert,
  Info,
  LoaderCircle,
  RefreshCw,
  TriangleAlert,
  X,
  CircleCheck,
  type LucideIcon,
} from 'lucide-react';

import type { ApiError } from '../../api/invoke';
import { cx } from './cx';
import { Combobox } from './combobox';
import { ScrollArea } from './scroll-area';
import { Select } from './select';
import { Tooltip } from './tooltip';

export { Combobox, ScrollArea, Select, Tooltip };

export function GitHubMark({ className = 'h-4 w-4' }: { className?: string }) {
  return (
    <svg viewBox="0 0 16 16" fill="currentColor" aria-hidden="true" className={className}>
      <path d="M8 0C3.58 0 0 3.58 0 8c0 3.54 2.29 6.53 5.47 7.59.4.07.55-.17.55-.38 0-.19-.01-.82-.01-1.49-2.01.37-2.53-.49-2.69-.94-.09-.23-.48-.94-.82-1.13-.28-.15-.68-.52-.01-.53.63-.01 1.08.58 1.23.82.72 1.21 1.87.87 2.33.66.07-.52.28-.87.51-1.07-1.78-.2-3.64-.89-3.64-3.95 0-.87.31-1.59.82-2.15-.08-.2-.36-1.02.08-2.12 0 0 .67-.21 2.2.82a7.42 7.42 0 0 1 2-.27c.68 0 1.36.09 2 .27 1.53-1.04 2.2-.82 2.2-.82.44 1.1.16 1.92.08 2.12.51.56.82 1.27.82 2.15 0 3.07-1.87 3.75-3.65 3.95.29.25.54.73.54 1.48 0 1.07-.01 1.93-.01 2.2 0 .21.15.46.55.38A8.01 8.01 0 0 0 16 8c0-4.42-3.58-8-8-8Z" />
    </svg>
  );
}

export interface ButtonProps extends ButtonHTMLAttributes<HTMLButtonElement> {
  variant?: 'primary' | 'secondary' | 'ghost' | 'danger';
  size?: 'sm' | 'md';
  icon?: LucideIcon;
  trailingIcon?: LucideIcon;
  loading?: boolean;
}

export function Button({
  variant = 'secondary',
  size = 'md',
  icon: Icon,
  trailingIcon: Trailing,
  loading = false,
  className,
  children,
  disabled,
  ...rest
}: ButtonProps) {
  const variantClass =
    variant === 'primary'
      ? 'btn-primary'
      : variant === 'ghost'
        ? 'btn-ghost'
        : variant === 'danger'
          ? 'btn-danger'
          : 'btn-secondary';

  return (
    <button
      type="button"
      {...rest}
      disabled={disabled || loading}
      className={cx(variantClass, size === 'sm' && 'h-[21px] px-2 text-[11px]', className)}
    >
      {loading ? (
        <LoaderCircle className="h-3.5 w-3.5 animate-spin" aria-hidden="true" />
      ) : Icon ? (
        <Icon className="h-3.5 w-3.5" aria-hidden="true" />
      ) : null}
      {children}
      {Trailing ? <Trailing className="h-3.5 w-3.5 opacity-70" aria-hidden="true" /> : null}
    </button>
  );
}

export function IconButton({
  icon: Icon,
  label,
  className,
  tone = 'default',
  ...rest
}: ButtonHTMLAttributes<HTMLButtonElement> & {
  icon: LucideIcon;
  label: string;
  tone?: 'default' | 'danger';
}) {
  return (
    <Tooltip content={label} side="top">
      <button
        type="button"
        {...rest}
        aria-label={label}
        className={cx(
          'inline-flex h-[25px] w-[25px] shrink-0 items-center justify-center rounded-md text-muted transition-colors',
          tone === 'danger'
            ? 'hover:bg-status-blocked-soft hover:text-status-blocked-ink'
            : 'hover:bg-hover hover:text-fg',
          'disabled:cursor-not-allowed disabled:opacity-60',
          className,
        )}
      >
        <Icon className="h-4 w-4" aria-hidden="true" />
      </button>
    </Tooltip>
  );
}

export function Spinner({ className = 'h-4 w-4' }: { className?: string }) {
  return <LoaderCircle className={cx('animate-spin text-accent', className)} aria-hidden="true" />;
}

export function Checkbox({
  label,
  className,
  inputRef,
  ...rest
}: InputHTMLAttributes<HTMLInputElement> & { label: string; inputRef?: Ref<HTMLInputElement> }) {
  return (
    <input
      ref={inputRef}
      type="checkbox"
      aria-label={label}
      title={label}
      {...rest}
      className={cx('h-3.5 w-3.5 shrink-0 cursor-pointer accent-accent', className)}
    />
  );
}

export function Field({
  label,
  hint,
  htmlFor,
  children,
  required,
}: {
  label: string;
  hint?: ReactNode;
  htmlFor?: string;
  children: ReactNode;
  required?: boolean;
}) {
  return (
    <div className="flex flex-col gap-1">
      <label
        htmlFor={htmlFor}
        className="text-[11px] font-semibold tracking-wide text-muted uppercase"
      >
        {label}
        {required ? <span className="ml-1 text-status-blocked-ink">*</span> : null}
      </label>
      {children}
      {hint ? <p className="text-[11px] leading-relaxed text-muted">{hint}</p> : null}
    </div>
  );
}

export function Panel({
  title,
  subtitle,
  actions,
  children,
  className,
  bodyClassName,
  bodyStyle,
  headerClassName,
  as: Tag = 'section',
}: {
  title?: ReactNode;
  subtitle?: ReactNode;
  actions?: ReactNode;
  children: ReactNode;
  className?: string;
  bodyClassName?: string;
  bodyStyle?: CSSProperties;
  headerClassName?: string;
  as?: 'section' | 'div' | 'aside';
}) {
  return (
    <Tag className={cx('panel flex min-h-0 flex-col', className)}>
      {title || actions ? (
        <header className={cx('panel-header', headerClassName)}>
          <div className="min-w-0">
            {title ? <h2 className="panel-title">{title}</h2> : null}
            {subtitle ? <p className="muted mt-px truncate">{subtitle}</p> : null}
          </div>
          {actions ? <div className="flex shrink-0 items-center gap-1.5">{actions}</div> : null}
        </header>
      ) : null}
      <div style={bodyStyle} className={cx('min-h-0 flex-1', bodyClassName)}>
        {children}
      </div>
    </Tag>
  );
}

export interface TabItem {
  id: string;
  label: string;
  count?: number;
  disabled?: boolean;
  title?: string;
}

export function Tabs({
  items,
  value,
  onChange,
  ariaLabel,
  className,
}: {
  items: TabItem[];
  value: string;
  onChange: (id: string) => void;
  ariaLabel: string;
  className?: string;
}) {
  return (
    <div
      role="tablist"
      aria-label={ariaLabel}
      aria-orientation="horizontal"
      className={cx(
        'flex h-[29px] items-center gap-0.5 overflow-x-auto border-b border-line px-2',
        className,
      )}
    >
      {items.map((item) => (
        <button
          key={item.id}
          type="button"
          role="tab"
          id={`tab-${item.id}`}
          aria-selected={value === item.id}
          disabled={item.disabled}
          title={item.title ?? item.label}
          onClick={() => onChange(item.id)}
          className={cx('tab', value === item.id && 'tab-active')}
        >
          <span className="truncate">{item.label}</span>
          {typeof item.count === 'number' ? (
            <span className="rounded-[20px] bg-raised px-[5px] py-[2px] text-badge leading-none font-semibold text-muted tabular-nums">
              {item.count}
            </span>
          ) : null}
        </button>
      ))}
    </div>
  );
}

export function LoadingState({ label = 'Loading…' }: { label?: string }) {
  return (
    <div
      role="status"
      aria-live="polite"
      className="flex items-center justify-center gap-2 p-8 text-xs text-muted"
    >
      <Spinner />
      {label}
    </div>
  );
}

export function EmptyState({
  icon: Icon,
  title,
  message,
  action,
  className,
}: {
  icon?: LucideIcon;
  title: string;
  message?: ReactNode;
  action?: ReactNode;
  className?: string;
}) {
  return (
    <div
      className={cx('flex flex-col items-center justify-center gap-2 p-8 text-center', className)}
    >
      {Icon ? <Icon className="h-8 w-8 text-disabled" aria-hidden="true" /> : null}
      <p className="text-xs font-semibold text-fg">{title}</p>
      {message ? (
        <p className="max-w-md text-[11px] leading-relaxed text-muted">{message}</p>
      ) : null}
      {action ? <div className="mt-2">{action}</div> : null}
    </div>
  );
}

export function ErrorState({
  error,
  onRetry,
  className,
  compact = false,
}: {
  error: ApiError;
  onRetry?: () => void;
  className?: string;
  compact?: boolean;
}) {
  return (
    <div
      className={cx(
        'flex flex-col items-center justify-center gap-2.5 text-center',
        compact ? 'p-4' : 'p-8',
        className,
      )}
      role="alert"
    >
      <CircleAlert className="h-6 w-6 text-status-blocked" aria-hidden="true" />
      <div className="space-y-1">
        <p className="text-xs font-semibold text-status-blocked-ink">{error.message}</p>
        <p className="mono text-disabled">code: {error.code}</p>
      </div>
      {error.remedy ? (
        <p className="max-w-lg text-[11px] leading-relaxed text-muted">{error.remedy}</p>
      ) : null}
      {error.details ? (
        <details className="w-full max-w-lg text-left">
          <summary className="cursor-pointer text-[11px] text-muted">Technical detail</summary>
          <pre className="mono mt-2 max-h-40 overflow-auto rounded-md border border-line bg-raised p-2 whitespace-pre-wrap text-muted">
            {error.details}
          </pre>
        </details>
      ) : null}
      {onRetry ? (
        <Button size="sm" onClick={onRetry} icon={RefreshCw}>
          Retry
        </Button>
      ) : null}
    </div>
  );
}

export function InlineAlert({
  tone = 'info',
  title,
  children,
  actions,
  className,
}: {
  tone?: 'info' | 'success' | 'warning' | 'danger';
  title: string;
  children?: ReactNode;
  actions?: ReactNode;
  className?: string;
}) {
  const tones = {
    info: {
      wrap: 'border-status-eligible/40 bg-status-eligible-soft text-status-eligible-ink',
      icon: Info,
      iconClass: 'text-status-eligible',
    },
    success: {
      wrap: 'border-status-applied/40 bg-status-applied-soft text-status-applied-ink',
      icon: CircleCheck,
      iconClass: 'text-status-applied',
    },
    warning: {
      wrap: 'border-status-unknown/40 bg-status-unknown-soft text-status-unknown-ink',
      icon: TriangleAlert,
      iconClass: 'text-status-unknown',
    },
    danger: {
      wrap: 'border-status-blocked/40 bg-status-blocked-soft text-status-blocked-ink',
      icon: CircleAlert,
      iconClass: 'text-status-blocked',
    },
  } as const;
  const tone_ = tones[tone];
  const Icon = tone_.icon;

  return (
    <div
      role={tone === 'danger' ? 'alert' : 'status'}
      className={cx('flex gap-2.5 rounded-md border px-2.5 py-2 text-xs', tone_.wrap, className)}
    >
      <Icon className={cx('mt-px h-4 w-4 shrink-0', tone_.iconClass)} aria-hidden="true" />
      <div className="min-w-0 flex-1 space-y-1">
        <p className="font-semibold">{title}</p>
        {children ? <div className="leading-relaxed">{children}</div> : null}
      </div>
      {actions ? <div className="flex shrink-0 items-start gap-1.5">{actions}</div> : null}
    </div>
  );
}

export function KeyValue({ label, children }: { label: string; children: ReactNode }) {
  return (
    <div className="flex items-start justify-between gap-2.5 py-1">
      <span className="text-xs text-muted">{label}</span>
      <span className="min-w-0 max-w-[65%] text-right text-xs text-fg">{children}</span>
    </div>
  );
}

export function Modal({
  open,
  onClose,
  title,
  description,
  children,
  footer,
  size = 'md',
}: {
  open: boolean;
  onClose: () => void;
  title: string;
  description?: ReactNode;
  children: ReactNode;
  footer?: ReactNode;
  size?: 'sm' | 'md' | 'lg';
}) {
  const titleId = useId();
  const descriptionId = useId();
  const containerRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    if (!open) return;
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key === 'Escape') {
        event.stopPropagation();
        onClose();
      }
    };
    document.addEventListener('keydown', onKeyDown);
    const timer = window.setTimeout(() => {
      const target = containerRef.current?.querySelector<HTMLElement>(
        '[data-autofocus], button:not([disabled]), [href], input:not([disabled]), select:not([disabled]), textarea:not([disabled])',
      );
      target?.focus();
    }, 30);
    return () => {
      document.removeEventListener('keydown', onKeyDown);
      window.clearTimeout(timer);
    };
  }, [open, onClose]);

  if (!open) return null;

  const widths = {
    sm: 'sm:max-w-[400px]',
    md: 'sm:max-w-[520px]',
    lg: 'sm:max-w-[600px]',
  };

  return (
    <div
      className="fixed inset-0 z-50 flex items-end justify-center bg-scrim sm:items-center sm:p-6"
      onMouseDown={(event) => {
        if (event.target === event.currentTarget) onClose();
      }}
      role="presentation"
    >
      <div
        ref={containerRef}
        role="dialog"
        aria-modal="true"
        aria-labelledby={titleId}
        aria-describedby={description ? descriptionId : undefined}
        className={cx(
          'flex max-h-[92vh] w-full flex-col overflow-hidden rounded-t-md border border-line bg-overlay shadow-dialog animate-fade-in sm:rounded-md',
          widths[size],
        )}
      >
        <header className="flex h-[50px] shrink-0 items-center justify-between gap-3 border-b border-line px-5">
          <div className="min-w-0">
            <h2 id={titleId} className="truncate text-sm font-semibold text-fg">
              {title}
            </h2>
            {description ? (
              <div
                id={descriptionId}
                className="mt-0.5 truncate text-[11px] leading-relaxed text-muted"
              >
                {description}
              </div>
            ) : null}
          </div>
          <IconButton icon={X} label="Close dialog" onClick={onClose} />
        </header>
        <div className="min-h-0 flex-1 overflow-y-auto px-5 py-4">{children}</div>
        {footer ? (
          <footer className="flex flex-wrap items-center justify-end gap-1.5 border-t border-line px-5 py-3">
            {footer}
          </footer>
        ) : null}
      </div>
    </div>
  );
}
