/** Every screen required by the spec (section 3.3). */
export type Route =
  | 'dashboard'
  | 'commit-browser'
  | 'promote-commits'
  | 'workflow-runs'
  | 'integration-history'
  | 'settings';

export interface NavItem {
  id: Route;
  label: string;
  /** Shown as a tooltip and used as the accessible description. */
  description: string;
}

export const NAV_ITEMS: NavItem[] = [
  {
    id: 'dashboard',
    label: 'Dashboard',
    description: 'Status by branch and recent runs',
  },
  {
    id: 'commit-browser',
    label: 'Commit Browser',
    description: 'Browse source history and inspect commits',
  },
  {
    id: 'promote-commits',
    label: 'Promote Commits',
    description: 'Confirm SHAs and target branch, then dispatch',
  },
  {
    id: 'workflow-runs',
    label: 'Workflow Runs',
    description: 'Runs, logs and outcomes',
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
  dashboard: {
    title: 'Dashboard',
    subtitle: 'Status by branch and recent runs',
  },
  'commit-browser': {
    title: 'Commit Browser',
    subtitle: 'Browse source history and inspect commits',
  },
  'promote-commits': {
    title: 'Promote Commits',
    subtitle: 'Confirm SHAs and target branch, then dispatch',
  },
  'workflow-runs': {
    title: 'Workflow Runs',
    subtitle: 'Runs, logs and outcomes',
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
