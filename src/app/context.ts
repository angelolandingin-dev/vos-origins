import {
  createContext,
  useCallback,
  useContext,
  useEffect,
  useMemo,
  useRef,
  useState,
  type ReactNode,
} from 'react';

import { asApiError, type ApiError } from '../api/invoke';
import type {
  AuthStatus,
  CommitSummary,
  DependencyChange,
  PromotionDelivery,
  PromotionDispatchResult,
  PromotionMode,
  PromotionOnConflict,
  PromotionPreflight,
  SubsystemSummary,
  TargetBranchList,
} from '../api/types';
import type { Route } from './navigation';

/** App-level data/selection state. The backend owns caching and pagination (spec §3.5), so no query library. */
/** No components in this module so Fast Refresh can replace provider and screens independently. */

export interface AsyncResult<T> {
  data: T | null;
  loading: boolean;
  error: ApiError | null;
  reload: () => void;
}

export interface Notice {
  id: number;
  tone: 'info' | 'success' | 'warning' | 'danger';
  title: string;
  message?: ReactNode;
}

/**
 * Fetches when `deps` change. `loading` derives during render from request identity,
 * so no state is written synchronously in the effect (avoids a cascading render).
 */
export function useAsyncData<T>(loader: () => Promise<T>, deps: unknown[]): AsyncResult<T> {
  const [nonce, setNonce] = useState(0);
  const [state, setState] = useState<{
    key: string | null;
    data: T | null;
    error: ApiError | null;
  }>({ key: null, data: null, error: null });
  const loaderRef = useRef(loader);
  const key = `${JSON.stringify(deps)}#${nonce}`;

  useEffect(() => {
    loaderRef.current = loader;
  });

  useEffect(() => {
    let cancelled = false;
    loaderRef
      .current()
      .then((data) => {
        if (!cancelled) setState({ key, data, error: null });
      })
      .catch((error: unknown) => {
        if (!cancelled) setState({ key, data: null, error: asApiError(error) });
      });
    return () => {
      cancelled = true;
    };
  }, [key]);

  const reload = useCallback(() => setNonce((value) => value + 1), []);
  const loading = state.key !== key;
  return useMemo(
    () => ({ data: state.data, loading, error: loading ? null : state.error, reload }),
    [state.data, state.error, loading, reload],
  );
}

export interface PromotionState {
  open: boolean;
  shas: string[];
  commits: CommitSummary[];
  mode: PromotionMode;
  baseSha: string;
  delivery: PromotionDelivery;
  onConflict: PromotionOnConflict;
  /** The promotion commit's own message; required before a dispatch. */
  message: string;
  /** Override the non-contiguous-selection refusal: a skipped commit's changes are included. */
  allowGaps: boolean;
  /** Paths to leave out of the promotion. */
  excludedPaths: string[];
  dispatching: boolean;
  error: ApiError | null;
  result: PromotionDispatchResult | null;
  dependencies: DependencyChange[];
  dependenciesLoading: boolean;
  preflight: PromotionPreflight | null;
  preflightLoading: boolean;
}

export interface AppContextValue {
  route: Route;
  navigate: (route: Route) => void;

  auth: AsyncResult<AuthStatus>;
  refreshAuth: () => void;
  subsystems: AsyncResult<SubsystemSummary[]>;
  refreshSubsystems: () => void;
  targetBranches: AsyncResult<TargetBranchList>;

  /** Changing this counter forces every page-level query to refetch. */
  dataVersion: number;
  bumpDataVersion: () => void;

  selectedSubsystemId: string | null;
  selectSubsystem: (id: string) => void;
  selectedSubsystem: SubsystemSummary | null;

  targetBranch: string;
  setTargetBranch: (branch: string) => void;

  selectedShas: string[];
  toggleSha: (sha: string) => void;
  setSelectedShas: (shas: string[], commits?: CommitSummary[]) => void;
  clearSelection: () => void;
  /** Accumulated summaries for selected SHAs, in `selectedShas` order. Survives paging and filtering. */
  selectedCommits: CommitSummary[];
  /** Merge the listing's summaries for currently-selected SHAs into the accumulated map. */
  rememberSelectedCommits: (listing: CommitSummary[]) => void;

  focusedSha: string | null;
  setFocusedSha: (sha: string | null) => void;

  notice: Notice | null;
  pushNotice: (notice: Omit<Notice, 'id'>) => void;
  dismissNotice: () => void;

  openExternal: (url: string) => void;

  promotion: PromotionState;
  openPromotionDialog: (
    shas: string[],
    commits: CommitSummary[],
    options?: { excludedPaths?: string[] },
  ) => void;
  closePromotionDialog: () => void;
  removePromotionSha: (sha: string) => void;
  setPromotionMode: (mode: PromotionMode) => void;
  setPromotionBaseSha: (sha: string) => void;
  setPromotionDelivery: (delivery: PromotionDelivery) => void;
  setPromotionOnConflict: (onConflict: PromotionOnConflict) => void;
  setPromotionMessage: (message: string) => void;
  setPromotionAllowGaps: (allowGaps: boolean) => void;
  setPromotionExcludedPaths: (paths: string[]) => void;
  addPromotionExcludedPath: (path: string) => void;
  removePromotionExcludedPath: (path: string) => void;
  dispatchPromotion: () => void;
}

export const AppContext = createContext<AppContextValue | null>(null);

export function useApp(): AppContextValue {
  const context = useContext(AppContext);
  if (!context) throw new Error('useApp must be used inside AppDataProvider');
  return context;
}
