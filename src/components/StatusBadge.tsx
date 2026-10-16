import type { CommitStatus, CommitStatusKind } from '../api/types';
import { statusAppearance } from './statusAppearance';

/**
 * Status badge for the vocabulary (spec §3.4). `Selected` is local-only and visually
 * distinct from `Integrated`. There is deliberately no `Ready` badge: subsystem approval
 * is not main-system rollout approval.
 */

export interface StatusBadgeProps {
  kind: CommitStatusKind;
  label?: string;
  size?: 'sm' | 'md';
  detail?: string | null;
  className?: string;
}

export function StatusBadge({
  kind,
  label,
  size = 'sm',
  detail,
  className = '',
}: StatusBadgeProps) {
  const appearance = statusAppearance(kind);
  const Icon = appearance.icon;
  const tooltip = detail ? `${appearance.meaning}\n\n${detail}` : appearance.meaning;

  return (
    <span
      className={`inline-flex shrink-0 items-center gap-1.5 whitespace-nowrap rounded-full font-medium ${
        size === 'sm' ? 'px-2 py-0.5 text-2xs' : 'px-2.5 py-1 text-xs'
      } ${appearance.className} ${className}`}
      title={tooltip}
      aria-label={`Status: ${label ?? appearance.label}. ${appearance.meaning}`}
    >
      <Icon className={`h-3.5 w-3.5 ${appearance.spin ? 'animate-spin' : ''}`} aria-hidden="true" />
      {label ?? appearance.label}
    </span>
  );
}

export function CommitStatusBadge({
  status,
  size = 'sm',
  showTarget = false,
}: {
  status: CommitStatus;
  size?: 'sm' | 'md';
  showTarget?: boolean;
}) {
  return (
    <span className="inline-flex items-center gap-1.5">
      <StatusBadge kind={status.kind} label={status.label} size={size} detail={status.detail} />
      {showTarget ? <span className="mono text-slate-500">{status.targetBranch}</span> : null}
    </span>
  );
}
