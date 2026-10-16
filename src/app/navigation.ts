import {
  Boxes,
  GitBranch,
  GitPullRequestArrow,
  History,
  LayoutDashboard,
  Settings,
  Workflow,
  type LucideIcon,
} from 'lucide-react';

/** Every screen required by the spec (section 3.3). */
export type Route =
  | 'dashboard'
  | 'subsystems'
  | 'commit-browser'
  | 'promote-commits'
  | 'workflow-runs'
  | 'integration-history'
  | 'settings';

export interface NavItem {
  id: Route;
  label: string;
  icon: LucideIcon;
  /** Shown as a tooltip and used as the accessible description. */
  description: string;
}

export const NAV_ITEMS: NavItem[] = [
  {
    id: 'dashboard',
    label: 'Dashboard',
    icon: LayoutDashboard,
    description: 'Counts by status, recent promotion runs, and target-branch activity',
  },
  {
    id: 'subsystems',
    label: 'Subsystems',
    icon: Boxes,
    description: 'Registry, subsystem cards, commits, diffs and details',
  },
  {
    id: 'commit-browser',
    label: 'Commit Browser',
    icon: GitBranch,
    description: 'Browse and filter source history, select exact SHAs',
  },
  {
    id: 'promote-commits',
    label: 'Promote Commits',
    icon: GitPullRequestArrow,
    description: 'Confirm SHAs and the target main-system branch, then dispatch',
  },
  {
    id: 'workflow-runs',
    label: 'Workflow Runs',
    icon: Workflow,
    description: 'Runs started by this tool, their logs and outcomes',
  },
  {
    id: 'integration-history',
    label: 'Integration History',
    icon: History,
    description: 'Merged, pending, reverted and restored promotions per branch',
  },
  {
    id: 'settings',
    label: 'Settings',
    icon: Settings,
    description: 'Main repository, GitHub CLI status, cache and diagnostics',
  },
];

export const ROUTE_META: Record<Route, { title: string; subtitle: string }> = {
  dashboard: {
    title: 'Dashboard',
    subtitle: 'Integration state across registered subsystems and main-system target branches',
  },
  subsystems: {
    title: 'Subsystems',
    subtitle: 'Manage and integrate independent subsystems',
  },
  'commit-browser': {
    title: 'Commit Browser',
    subtitle: 'Browse a subsystem source branch and select exact commit SHAs',
  },
  'promote-commits': {
    title: 'Promote Commits',
    subtitle: 'Open an integration PR against a chosen main-system branch — never a direct push',
  },
  'workflow-runs': {
    title: 'Workflow Runs',
    subtitle: 'Promotion workflow runs started by this tool',
  },
  'integration-history': {
    title: 'Integration History',
    subtitle: 'Branch-scoped promotion events derived from reachable main-system Git history',
  },
  settings: {
    title: 'Settings',
    subtitle: 'Main repository, GitHub CLI, cache and diagnostics',
  },
};

export function routeMeta(route: Route) {
  return ROUTE_META[route];
}
