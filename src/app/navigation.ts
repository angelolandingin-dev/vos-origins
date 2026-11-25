/** Every screen required by the spec (section 3.3). */
export type Route =
  | 'commit-browser'
  | 'subsystem-diff'
  | 'promote-commits'
  | 'distribute'
  | 'reconcile'
  | 'integration-history'
  | 'settings';

export interface NavItem {
  id: Route;
  label: string;
  description: string;
}

export const NAV_ITEMS: NavItem[] = [
  {
    id: 'commit-browser',
    label: 'Commit Browser',
    description: 'Browse source history and inspect commits',
  },
  {
    id: 'subsystem-diff',
    label: 'Subsystem Diff',
    description: 'Files missing or diverged versus the target branch',
  },
  {
    id: 'promote-commits',
    label: 'Promote Commits',
    description: 'Confirm SHAs and target branch, then dispatch',
  },
  {
    id: 'distribute',
    label: 'Distribute',
    description: 'Send selected trunk commits down to subsystem pull requests',
  },
  {
    id: 'reconcile',
    label: 'Reconcile',
    description: 'Mirror selected folders between a subsystem and main',
  },
  {
    id: 'integration-history',
    label: 'Integration History',
    description: 'Merged, pending, reverted and restored per branch',
  },
  {
    id: 'settings',
    label: 'Settings',
    description: 'Repository, CLI status, cache and diagnostics',
  },
];

export const ROUTE_META: Record<Route, { title: string; subtitle: string }> = {
  'commit-browser': {
    title: 'Commit Browser',
    subtitle: 'Browse source history and inspect commits',
  },
  'subsystem-diff': {
    title: 'Subsystem Diff',
    subtitle: 'Files missing or diverged versus the target branch',
  },
  'promote-commits': {
    title: 'Promote Commits',
    subtitle: 'Confirm SHAs and target branch, then dispatch',
  },
  distribute: {
    title: 'Distribute',
    subtitle: 'Send selected trunk commits down to subsystem pull requests',
  },
  reconcile: {
    title: 'Reconcile',
    subtitle: 'Mirror selected folders between a subsystem and main',
  },
  'integration-history': {
    title: 'Integration History',
    subtitle: 'Merged, pending, reverted and restored per branch',
  },
  settings: {
    title: 'Settings',
    subtitle: 'Repository, CLI status, cache and diagnostics',
  },
};

export function routeMeta(route: Route) {
  return ROUTE_META[route];
}
