import type { CommitStatus, CommitStatusKind } from '../api/types';
import { statusAppearance } from './statusAppearance';

/**
 * Status indicator for the vocabulary (spec §3.4). `Selected` is local-only and visually
 * distinct from `Integrated`. There is deliberately no `Ready` badge: subsystem approval
 * is not main-system rollout approval.
 *
 * Colour lives on a 6px dot plus the 11px label — never a tinted whole row,
 * never a filled pill. While a promotion runs, an animated glyph in the status
 * ink colour takes the dot's place.
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
  // The appearance class is the bare dot colour (`bg-status-*` / `bg-accent`);
  // the matching ink token colours the animated glyph while a promotion runs.
  const inkClassName = `${appearance.className.replace(/^bg-/, 'text-')}-ink`;

  return (
    <span
      className={`inline-flex shrink-0 items-center gap-1 whitespace-nowrap ${size === 'sm' ? 'text-[11px]' : 'text-xs'} ${className}`}
      title={tooltip}
      aria-label={`Status: ${label ?? appearance.label}. ${appearance.meaning}`}
    >
      {appearance.spin ? (
        <Icon
          className={`${size === 'sm' ? 'h-2.5 w-2.5' : 'h-3 w-3'} ${inkClassName} animate-spin`}
          aria-hidden="true"
        />
      ) : (
        <span
          aria-hidden="true"
          className={`${size === 'sm' ? 'h-1.5 w-1.5' : 'h-2 w-2'} shrink-0 rounded-full ${appearance.className}`}
        />
      )}
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
      {showTarget ? <span className="mono text-muted">{status.targetBranch}</span> : null}
    </span>
  );
}
