import {
  Ban,
  CircleCheck,
  CircleHelp,
  CircleX,
  Clock,
  Eye,
  GitMerge,
  GitPullRequestArrow,
  LoaderCircle,
  SquareCheck,
  Undo2,
  type LucideIcon,
} from 'lucide-react';

import type { CommitStatusKind } from '../api/types';

/** Badge presentation for the status vocabulary (spec §3.4), kept separate so `StatusBadge.tsx` exports only components. */

export interface Appearance {
  label: string;
  className: string;
  icon: LucideIcon;
  meaning: string;
  spin?: boolean;
}

const APPEARANCE: Record<CommitStatusKind, Appearance> = {
  eligible: {
    label: 'Eligible',
    className: 'bg-emerald-950/70 text-emerald-300 ring-1 ring-inset ring-emerald-800/70',
    icon: CircleCheck,
    meaning:
      'On the protected source branch with no integration found for the selected target branch.',
  },
  selected: {
    label: 'Selected',
    className: 'bg-brand-900/40 text-brand-200 ring-1 ring-inset ring-brand-600/70',
    icon: SquareCheck,
    meaning: 'Ticked locally. A selection state, not an integration status.',
  },
  promotion_queued: {
    label: 'Promotion queued',
    className: 'bg-amber-950/70 text-amber-300 ring-1 ring-inset ring-amber-800/70',
    icon: Clock,
    meaning:
      'A promotion workflow has been dispatched and has not started yet. No integration PR exists yet.',
  },
  promotion_running: {
    label: 'Promotion running',
    className: 'bg-orange-950/70 text-orange-300 ring-1 ring-inset ring-orange-800/70',
    icon: LoaderCircle,
    meaning: 'A promotion workflow is running against the integration branch.',
    spin: true,
  },
  pr_open: {
    label: 'PR open',
    className: 'bg-violet-950/70 text-violet-300 ring-1 ring-inset ring-violet-800/70',
    icon: GitPullRequestArrow,
    meaning: 'An integration PR for this SHA and target branch exists and is not merged.',
  },
  in_review: {
    label: 'In review',
    className: 'bg-fuchsia-950/60 text-fuchsia-300 ring-1 ring-inset ring-fuchsia-800/60',
    icon: Eye,
    meaning: 'The open integration PR has review activity but is not merged.',
  },
  integrated: {
    label: 'Integrated',
    className: 'bg-emerald-600/90 text-emerald-50 ring-1 ring-inset ring-emerald-400/60',
    icon: GitMerge,
    meaning:
      'Recorded by a reachable integration commit on this target branch and not subsequently reverted.',
  },
  reverted: {
    label: 'Reverted',
    className: 'bg-slate-800 text-slate-300 ring-1 ring-inset ring-slate-600',
    icon: Undo2,
    meaning:
      'An integration record exists but a later explicit revert event on this branch reversed it.',
  },
  blocked: {
    label: 'Blocked',
    className: 'bg-rose-950/70 text-rose-300 ring-1 ring-inset ring-rose-800/70',
    icon: Ban,
    meaning:
      'Policy, patch or dependency handling stopped the promotion. The target branch was left unchanged.',
  },
  failed: {
    label: 'Failed',
    className: 'bg-red-600/90 text-red-50 ring-1 ring-inset ring-red-400/60',
    icon: CircleX,
    meaning: 'The workflow or main-system CI failed for this promotion.',
  },
  unknown: {
    label: 'Unknown',
    className: 'border border-dashed border-ink-500 bg-ink-850 text-slate-400',
    icon: CircleHelp,
    meaning:
      'Integration state could not be verified — for example offline, rate-limited, or insufficient permissions.',
  },
};

export function statusAppearance(kind: CommitStatusKind): Appearance {
  return APPEARANCE[kind] ?? APPEARANCE.unknown;
}
