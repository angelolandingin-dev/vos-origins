/** Every screen required by the spec (section 3.3). */
export type Route =
  | 'commit-browser'
  | 'subsystem-diff'
  | 'promote-commits'
  | 'migrate'
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
    id: 'migrate',
    label: 'Migrate',
    description: 'Push main-branch changes into subsystems as pull requests',
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
  migrate: {
    title: 'Migrate',
    subtitle: 'Push main-branch changes into subsystems as pull requests',
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
