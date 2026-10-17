import {
  Ban,
  CheckCheck,
  CircleCheck,
  CircleHelp,
  CircleX,
  Eye,
  GitMerge,
  GitPullRequestArrow,
  History,
  SquareCheck,
  TriangleAlert,
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

/**
 * Every status resolves to a bare dot colour: a 6px dot plus an 11px label.
 * Colour never tints a whole row and never fills a badge.
 * Labels and honest meanings are unchanged.
 */
const APPEARANCE: Record<CommitStatusKind, Appearance> = {
  eligible: {
    label: 'Eligible',
    className: 'bg-status-eligible',
    icon: CircleCheck,
    meaning:
      'On the protected source branch with no integration found for the selected target branch.',
  },
  selected: {
    label: 'Selected',
    className: 'bg-accent',
    icon: SquareCheck,
    meaning: 'Ticked locally. A selection state, not an integration status.',
  },
  pr_open: {
    label: 'PR open',
    className: 'bg-status-applied',
    icon: GitPullRequestArrow,
    meaning: 'An integration PR for this SHA and target branch exists and is not merged.',
  },
  in_review: {
    label: 'In review',
    className: 'bg-status-eligible',
    icon: Eye,
    meaning: 'The open integration PR has review activity but is not merged.',
  },
  integrated: {
    label: 'Integrated',
    className: 'bg-status-integrated',
    icon: GitMerge,
    meaning:
      'Recorded by a reachable integration commit on this target branch and not subsequently reverted.',
  },
  applied: {
    label: 'Already applied',
    className: 'bg-status-applied',
    icon: CheckCheck,
    meaning:
      'Everything this commit changes already matches the target branch byte for byte — there is nothing to promote.',
  },
  divergent: {
    label: 'Divergent',
    className: 'bg-status-divergent',
    icon: TriangleAlert,
    meaning:
      "The main system's copies of every source file this commit changes were themselves changed more recently; promoting would revert newer work. Dependency manifest and lockfile paths are exempt — they are merged or regenerated.",
  },
  superseded: {
    label: 'Superseded',
    className: 'bg-status-superseded',
    icon: History,
    meaning:
      'A newer commit on the source branch changes the same files; promoting this one would re-apply stale content.',
  },
  reverted: {
    label: 'Reverted',
    className: 'bg-status-superseded',
    icon: Undo2,
    meaning:
      'An integration record exists but a later explicit revert event on this branch reversed it.',
  },
  blocked: {
    label: 'Blocked',
    className: 'bg-status-blocked',
    icon: Ban,
    meaning:
      'Policy, patch or dependency handling stopped the promotion. The target branch was left unchanged.',
  },
  failed: {
    label: 'Failed',
    className: 'bg-status-blocked',
    icon: CircleX,
    meaning: 'The workflow or main-system CI failed for this promotion.',
  },
  unknown: {
    label: 'Unknown',
    className: 'bg-status-unknown',
    icon: CircleHelp,
    meaning:
      'Integration state could not be verified — for example offline, rate-limited, or insufficient permissions.',
  },
};

export function statusAppearance(kind: CommitStatusKind): Appearance {
  return APPEARANCE[kind] ?? APPEARANCE.unknown;
}
