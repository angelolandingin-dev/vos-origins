import type { CommitStatusKind, IntegrationHistoryKind } from '../api/types';

/** Shared badge vocabulary for the five integration event kinds (spec §3.4). */
export const INTEGRATION_KIND_META: Record<
  IntegrationHistoryKind,
  { label: string; kind: CommitStatusKind }
> = {
  merged: { label: 'Merged', kind: 'integrated' },
  pending_pr: { label: 'Pending PR', kind: 'pr_open' },
  reverted: { label: 'Reverted', kind: 'reverted' },
  restored: { label: 'Restored', kind: 'integrated' },
  failed: { label: 'Failed', kind: 'failed' },
};
