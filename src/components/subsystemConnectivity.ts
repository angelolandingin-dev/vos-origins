import type { ConnectivityState } from '../api/types';

export const CONNECTIVITY_DOT: Record<ConnectivityState, { dot: string; label: string }> = {
  connected: { dot: 'bg-status-applied', label: 'Connected' },
  degraded: { dot: 'bg-status-unknown', label: 'Degraded' },
  unknown: { dot: 'bg-status-superseded', label: 'Unknown' },
};
