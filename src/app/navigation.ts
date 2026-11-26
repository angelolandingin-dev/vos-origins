/** Primary navigation routes. */
export type Route = 'promote' | 'distribute' | 'reconcile' | 'integration-history' | 'settings';

export interface NavItem {
  id: Route;
  label: string;
  description: string;
}

export const NAV_ITEMS: NavItem[] = [
  {
    id: 'promote',
    label: 'Promote',
    description: 'Browse commits or file drift and promote subsystem changes to main',
  },
  {
    id: 'distribute',
    label: 'Distribute',
    description: 'Send a trunk commit down to a subsystem pull request',
  },
  {
    id: 'reconcile',
    label: 'Reconcile',
    description: 'Mirror selected folders between a subsystem and main',
  },
  {
    id: 'integration-history',
    label: 'History',
    description: 'Merged, pending, reverted and restored per branch',
  },
  {
    id: 'settings',
    label: 'Settings',
    description: 'Registry, CLI status, cache, updates, and diagnostics',
  },
];

export const ROUTE_META: Record<Route, { title: string; subtitle: string }> = {
  promote: {
    title: 'Promote',
    subtitle: 'Browse commits or file drift and promote subsystem changes to main',
  },
  distribute: {
    title: 'Distribute',
    subtitle: 'Send a trunk commit down to a subsystem pull request',
  },
  reconcile: {
    title: 'Reconcile',
    subtitle: 'Mirror selected folders between a subsystem and main',
  },
  'integration-history': {
    title: 'History',
    subtitle: 'Merged, pending, reverted and restored per branch',
  },
  settings: {
    title: 'Settings',
    subtitle: 'Registry, CLI status, cache, updates, and diagnostics',
  },
};

export function routeMeta(route: Route) {
  return ROUTE_META[route];
}
