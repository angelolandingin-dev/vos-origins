import type {
  ButtonHTMLAttributes,
  InputHTMLAttributes,
  ReactNode,
  Ref,
  SelectHTMLAttributes,
} from 'react';
import { useEffect, useId, useRef } from 'react';
import {
  ChevronDown,
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

/** The GitHub mark is not part of Lucide 1.x, so it is provided inline. */
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
      className={cx(variantClass, size === 'sm' && 'px-2.5 py-1.5 text-xs', className)}
    >
      {loading ? (
        <LoaderCircle className="h-4 w-4 animate-spin" aria-hidden="true" />
      ) : Icon ? (
        <Icon className="h-4 w-4" aria-hidden="true" />
      ) : null}
      {children}
      {Trailing ? <Trailing className="h-4 w-4 opacity-70" aria-hidden="true" /> : null}
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
    <button
      type="button"
      {...rest}
      title={label}
      aria-label={label}
      className={cx(
        'inline-flex h-8 w-8 items-center justify-center rounded-md text-slate-300 transition-colors',
        tone === 'danger'
          ? 'hover:bg-rose-900/50 hover:text-rose-200'
          : 'hover:bg-ink-700 hover:text-white',
        'disabled:cursor-not-allowed disabled:opacity-40',
        className,
      )}
    >
      <Icon className="h-4 w-4" aria-hidden="true" />
    </button>
  );
}

export function Spinner({ className = 'h-4 w-4' }: { className?: string }) {
  return (
    <LoaderCircle className={cx('animate-spin text-brand-400', className)} aria-hidden="true" />
  );
}

export function Select({
  options,
  className,
  ...rest
}: SelectHTMLAttributes<HTMLSelectElement> & {
  options: Array<{ value: string; label: string; disabled?: boolean; title?: string }>;
}) {
  return (
    <span className={cx('relative inline-flex w-full', className)}>
      <select {...rest} className="select">
        {options.map((option) => (
          <option
            key={option.value}
            value={option.value}
            disabled={option.disabled}
            title={option.title}
          >
            {option.label}
          </option>
        ))}
      </select>
      <ChevronDown
        className="pointer-events-none absolute right-3 top-1/2 h-4 w-4 -translate-y-1/2 text-slate-400"
        aria-hidden="true"
      />
    </span>
  );
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
      className={cx(
        'h-4 w-4 shrink-0 cursor-pointer rounded-sm border-ink-500 bg-ink-900 text-brand-600',
        'focus:ring-2 focus:ring-brand-500 focus:ring-offset-1 focus:ring-offset-ink-870',
        className,
      )}
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
    <div className="flex flex-col gap-1.5">
      <label
        htmlFor={htmlFor}
        className="text-xs font-semibold uppercase tracking-wide text-slate-400"
      >
        {label}
        {required ? <span className="ml-1 text-rose-400">*</span> : null}
      </label>
      {children}
      {hint ? <p className="text-2xs leading-relaxed text-slate-500">{hint}</p> : null}
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
  headerClassName,
  as: Tag = 'section',
}: {
  title?: ReactNode;
  subtitle?: ReactNode;
  actions?: ReactNode;
  children: ReactNode;
  className?: string;
  bodyClassName?: string;
  headerClassName?: string;
  as?: 'section' | 'div' | 'aside';
}) {
  return (
    <Tag className={cx('panel flex min-h-0 flex-col', className)}>
      {title || actions ? (
        <header className={cx('panel-header', headerClassName)}>
          <div className="min-w-0">
            {title ? <h2 className="panel-title truncate">{title}</h2> : null}
            {subtitle ? <p className="muted mt-0.5 truncate">{subtitle}</p> : null}
          </div>
          {actions ? <div className="flex shrink-0 items-center gap-2">{actions}</div> : null}
        </header>
      ) : null}
      <div className={cx('min-h-0 flex-1', bodyClassName)}>{children}</div>
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
        'flex items-center gap-1 overflow-x-auto border-b border-ink-700/60 px-2',
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
          className={cx(
            'tab',
            value === item.id && 'tab-active',
            item.disabled && 'cursor-not-allowed opacity-40',
          )}
        >
          {item.label}
          {typeof item.count === 'number' ? (
            <span className="ml-1.5 rounded-full bg-ink-700/80 px-1.5 py-0.5 text-2xs text-slate-300">
              {item.count}
            </span>
          ) : null}
          {value === item.id ? (
            <span
              aria-hidden="true"
              className="absolute inset-x-2 -bottom-px h-0.5 rounded-full bg-brand-500"
            />
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
      className="flex items-center justify-center gap-3 p-8 text-sm text-slate-400"
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
      {Icon ? <Icon className="h-6 w-6 text-slate-500" aria-hidden="true" /> : null}
      <p className="text-sm font-medium text-slate-200">{title}</p>
      {message ? (
        <p className="max-w-md text-xs leading-relaxed text-slate-400">{message}</p>
      ) : null}
      {action ? <div className="mt-2">{action}</div> : null}
    </div>
  );
}

/** Error state with code, message and remedy. Never renders a silent empty success. */
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
        'flex flex-col items-center justify-center gap-3 text-center',
        compact ? 'p-4' : 'p-8',
        className,
      )}
      role="alert"
    >
      <CircleAlert className="h-6 w-6 text-rose-400" aria-hidden="true" />
      <div className="space-y-1">
        <p className="text-sm font-semibold text-rose-200">{error.message}</p>
        <p className="mono text-slate-500">code: {error.code}</p>
      </div>
      {error.remedy ? (
        <p className="max-w-lg text-xs leading-relaxed text-slate-300">{error.remedy}</p>
      ) : null}
      {error.details ? (
        <details className="w-full max-w-lg text-left">
          <summary className="cursor-pointer text-2xs text-slate-400">Technical detail</summary>
          <pre className="mono mt-2 max-h-40 overflow-auto whitespace-pre-wrap rounded-md border border-ink-700 bg-ink-900 p-2 text-2xs text-slate-400">
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
      wrap: 'border-brand-700/60 bg-brand-950/40 text-brand-100',
      icon: Info,
      iconClass: 'text-brand-300',
    },
    success: {
      wrap: 'border-emerald-800/70 bg-emerald-950/40 text-emerald-100',
      icon: CircleCheck,
      iconClass: 'text-emerald-300',
    },
    warning: {
      wrap: 'border-amber-800/70 bg-amber-950/40 text-amber-100',
      icon: TriangleAlert,
      iconClass: 'text-amber-300',
    },
    danger: {
      wrap: 'border-rose-900/70 bg-rose-950/40 text-rose-100',
      icon: CircleAlert,
      iconClass: 'text-rose-300',
    },
  } as const;
  const tone_ = tones[tone];
  const Icon = tone_.icon;

  return (
    <div
      role={tone === 'danger' ? 'alert' : 'status'}
      className={cx('flex gap-3 rounded-lg border px-3 py-2.5 text-xs', tone_.wrap, className)}
    >
      <Icon className={cx('mt-0.5 h-4 w-4 shrink-0', tone_.iconClass)} aria-hidden="true" />
      <div className="min-w-0 flex-1 space-y-1">
        <p className="font-semibold">{title}</p>
        {children ? <div className="leading-relaxed text-slate-300">{children}</div> : null}
      </div>
      {actions ? <div className="flex shrink-0 items-start gap-2">{actions}</div> : null}
    </div>
  );
}

export function KeyValue({ label, children }: { label: string; children: ReactNode }) {
  return (
    <div className="flex items-start justify-between gap-3 py-1.5">
      <span className="text-xs text-slate-400">{label}</span>
      <span className="min-w-0 max-w-[65%] text-right text-xs text-slate-200">{children}</span>
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

  const widths = { sm: 'sm:max-w-md', md: 'sm:max-w-2xl', lg: 'sm:max-w-4xl' };

  return (
    <div
      className="fixed inset-0 z-50 flex items-end justify-center bg-ink-950/80 backdrop-blur-xs sm:items-center sm:p-6"
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
          'flex max-h-[92vh] w-full flex-col overflow-hidden rounded-t-2xl border border-ink-700 bg-ink-870 shadow-popover animate-fade-in sm:rounded-xl',
          widths[size],
        )}
      >
        <header className="flex items-start justify-between gap-4 border-b border-ink-700/70 px-5 py-4">
          <div className="min-w-0">
            <h2 id={titleId} className="text-base font-semibold text-white">
              {title}
            </h2>
            {description ? (
              <div id={descriptionId} className="mt-1 text-xs leading-relaxed text-slate-400">
                {description}
              </div>
            ) : null}
          </div>
          <IconButton icon={X} label="Close dialog" onClick={onClose} />
        </header>
        <div className="min-h-0 flex-1 overflow-y-auto px-5 py-4">{children}</div>
        {footer ? (
          <footer className="flex flex-wrap items-center justify-end gap-2 border-t border-ink-700/70 px-5 py-4">
            {footer}
          </footer>
        ) : null}
      </div>
    </div>
  );
}
