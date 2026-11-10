import { useCallback, useMemo, useState, type ReactNode } from 'react';

import { listen } from '@tauri-apps/api/event';

import { api, asApiError } from '../api/invoke';
import type {
  CommitSummary,
  DependencyChange,
  PromotionDelivery,
  PromotionMode,
  PromotionPreflight,
  PromotionProgress,
} from '../api/types';
import {
  AppContext,
  useAsyncData,
  type AppContextValue,
  type Notice,
  type PromotionState,
} from './context';
import type { Route } from './navigation';

const DEFAULT_TARGET_BRANCH = 'staging';

const TARGET_BRANCH_KEY = 'vos-origins.target-branch';

function readTargetBranchSetting(): string | null {
  try {
    const value = window.localStorage.getItem(TARGET_BRANCH_KEY);
    return value && value.length > 0 ? value : null;
  } catch {
    return null;
  }
}

type PromotionInternal = Omit<
  PromotionState,
  'dependencies' | 'dependenciesLoading' | 'preflight' | 'preflightLoading'
>;

export function AppDataProvider({ children }: { children: ReactNode }) {
  const [route, setRoute] = useState<Route>('commit-browser');
  const [dataVersion, setDataVersion] = useState(0);
  const [selectedSubsystemIdSetting, setSelectedSubsystemId] = useState<string | null>(null);
  const [targetBranchSetting, setTargetBranchState] = useState<string | null>(
    readTargetBranchSetting,
  );
  const setTargetBranch = useCallback((branch: string) => {
    setTargetBranchState(branch);
    try {
      window.localStorage.setItem(TARGET_BRANCH_KEY, branch);
    } catch {
      /* Storage unavailable - the selection simply does not survive a restart. */
    }
  }, []);
  const [selectedShas, setSelectedShasState] = useState<string[]>([]);
  const [focusedSha, setFocusedSha] = useState<string | null>(null);
  const [selectedSummaries, setSelectedSummaries] = useState<Map<string, CommitSummary>>(
    () => new Map(),
  );
  const [notice, setNotice] = useState<Notice | null>(null);
  const [promotion, setPromotion] = useState<PromotionInternal>({
    open: false,
    shas: [],
    commits: [],
    mode: 'individual',
    baseSha: '',
    delivery: 'local_direct',
    message: '',
    excludedPaths: [],
    dispatching: false,
    progress: [],
    error: null,
    result: null,
  });

  const auth = useAsyncData(() => api.getAuthStatus(), [dataVersion]);
  const subsystems = useAsyncData(() => api.listSubsystems(), [dataVersion]);
  const targetBranches = useAsyncData(() => api.listTargetBranches(), [dataVersion]);

  const subsystemList = subsystems.data;

  // Defaults to the first entry (preferring hr-management); derived during render so a user selection always wins.
  const selectedSubsystemId = useMemo(() => {
    if (selectedSubsystemIdSetting) return selectedSubsystemIdSetting;
    if (!subsystemList || subsystemList.length === 0) return null;
    const preferred =
      subsystemList.find((subsystem) => subsystem.id === 'hr-management') ?? subsystemList[0];
    return preferred?.id ?? null;
  }, [selectedSubsystemIdSetting, subsystemList]);

  const allowedTargetBranches = useMemo(() => {
    if (!targetBranches.data) return null;
    return targetBranches.data.branches
      .filter((branch) => branch.allowed)
      .map((branch) => branch.name);
  }, [targetBranches.data]);

  const targetBranch = useMemo(() => {
    if (!allowedTargetBranches || allowedTargetBranches.length === 0) {
      return targetBranchSetting ?? DEFAULT_TARGET_BRANCH;
    }
    if (targetBranchSetting && allowedTargetBranches.includes(targetBranchSetting)) {
      return targetBranchSetting;
    }
    return allowedTargetBranches[0] ?? targetBranchSetting ?? DEFAULT_TARGET_BRANCH;
  }, [allowedTargetBranches, targetBranchSetting]);

  const selectedSubsystem =
    subsystemList?.find((subsystem) => subsystem.id === selectedSubsystemId) ?? null;

  const promotionDependencies = useAsyncData<DependencyChange[]>(async () => {
    if (!promotion.open || promotion.shas.length === 0 || !selectedSubsystemId) return [];
    const details = await Promise.all(
      promotion.shas.map((sha) => api.getCommitDetails(selectedSubsystemId, sha, targetBranch)),
    );
    return details.flatMap((detail) => detail.dependencies);
  }, [promotion.open, promotion.shas, selectedSubsystemId, targetBranch]);

  const promotionPreflight = useAsyncData<PromotionPreflight | null>(async () => {
    if (!promotion.open || promotion.shas.length === 0 || !selectedSubsystemId) return null;
    return api.preflightPromotion({
      subsystemId: selectedSubsystemId,
      sourceCommitShas: promotion.shas,
      mode: promotion.mode,
      baseSha: promotion.mode === 'cumulative' ? promotion.baseSha : null,
      targetBranch,
      confirmationAcknowledged: false,
      delivery: promotion.delivery,
      // Promotion is a copy: the subsystem's copy always wins. The dialog no
      // longer offers a choice; the wire field is retained for compatibility.
      onConflict: 'take_subsystem',
      message: promotion.message,
      excludedPaths: promotion.excludedPaths,
    });
  }, [
    promotion.open,
    promotion.shas,
    selectedSubsystemId,
    targetBranch,
    promotion.mode,
    promotion.baseSha,
    promotion.delivery,
    promotion.message,
    promotion.excludedPaths,
  ]);

  const pushNotice = useCallback((next: Omit<Notice, 'id'>) => {
    setNotice({ ...next, id: Date.now() });
  }, []);

  const dismissNotice = useCallback(() => setNotice(null), []);

  const bumpDataVersion = useCallback(() => setDataVersion((value) => value + 1), []);

  const selectSubsystem = useCallback((id: string) => {
    setSelectedSubsystemId(id);
    setSelectedShasState([]);
    setSelectedSummaries(new Map());
    setFocusedSha(null);
  }, []);

  const toggleSha = useCallback(
    (sha: string) => {
      const removing = selectedShas.includes(sha);
      setSelectedShasState((current) =>
        current.includes(sha) ? current.filter((value) => value !== sha) : [...current, sha],
      );
      if (removing) {
        setSelectedSummaries((prev) => {
          if (!prev.has(sha)) return prev;
          const next = new Map(prev);
          next.delete(sha);
          return next;
        });
      }
    },
    [selectedShas],
  );

  const setSelectedShas = useCallback((shas: string[], commits: CommitSummary[] = []) => {
    setSelectedShasState(shas);
    setSelectedSummaries((prev) => {
      const next = new Map(prev);
      let changed = false;
      for (const commit of commits) {
        if (shas.includes(commit.sha) && next.get(commit.sha) !== commit) {
          next.set(commit.sha, commit);
          changed = true;
        }
      }
      for (const sha of Array.from(next.keys())) {
        if (!shas.includes(sha)) {
          next.delete(sha);
          changed = true;
        }
      }
      return changed ? next : prev;
    });
  }, []);

  const clearSelection = useCallback(() => {
    setSelectedShasState([]);
    setSelectedSummaries(new Map());
  }, []);

  const selectedCommits = useMemo(() => {
    const commits: CommitSummary[] = [];
    for (const sha of selectedShas) {
      const summary = selectedSummaries.get(sha);
      if (summary) commits.push(summary);
    }
    return commits;
  }, [selectedShas, selectedSummaries]);

  const rememberSelectedCommits = useCallback(
    (listing: CommitSummary[]) => {
      if (listing.length === 0) return;
      setSelectedSummaries((prev) => {
        let changed = false;
        const next = new Map(prev);
        for (const commit of listing) {
          if (selectedShas.includes(commit.sha) && next.get(commit.sha) !== commit) {
            next.set(commit.sha, commit);
            changed = true;
          }
        }
        return changed ? next : prev;
      });
    },
    [selectedShas],
  );

  const openExternal = useCallback(
    (url: string) => {
      void api.openExternalUrl(url).catch((error: unknown) => {
        const apiError = asApiError(error);
        pushNotice({
          tone: 'danger',
          title: apiError.message,
          message: apiError.remedy ?? undefined,
        });
      });
    },
    [pushNotice],
  );

  const openPromotionDialog = useCallback(
    (shas: string[], commits: CommitSummary[], options?: { excludedPaths?: string[] }) => {
      setPromotion({
        open: true,
        shas,
        commits,
        mode: 'individual',
        baseSha: '',
        delivery: 'local_direct',
        message: '',
        excludedPaths: options?.excludedPaths ?? [],
        dispatching: false,
        progress: [],
        error: null,
        result: null,
      });
    },
    [],
  );

  const closePromotionDialog = useCallback(
    () => setPromotion((current) => ({ ...current, open: false })),
    [],
  );

  const removePromotionSha = useCallback((sha: string) => {
    setPromotion((current) => ({
      ...current,
      shas: current.shas.filter((value) => value !== sha),
      commits: current.commits.filter((commit) => commit.sha !== sha),
    }));
  }, []);

  const setPromotionMode = useCallback((mode: PromotionMode) => {
    setPromotion((current) => ({ ...current, mode }));
  }, []);

  const setPromotionBaseSha = useCallback((baseSha: string) => {
    setPromotion((current) => ({ ...current, baseSha }));
  }, []);

  const setPromotionDelivery = useCallback((delivery: PromotionDelivery) => {
    setPromotion((current) => ({ ...current, delivery }));
  }, []);

  const setPromotionMessage = useCallback((message: string) => {
    setPromotion((current) => ({ ...current, message }));
  }, []);

  const setPromotionExcludedPaths = useCallback((paths: string[]) => {
    setPromotion((current) => ({ ...current, excludedPaths: paths }));
  }, []);

  const addPromotionExcludedPath = useCallback((path: string) => {
    const trimmed = path.trim();
    if (!trimmed) return;
    setPromotion((current) =>
      current.excludedPaths.includes(trimmed)
        ? current
        : { ...current, excludedPaths: [...current.excludedPaths, trimmed] },
    );
  }, []);

  const dispatchPromotion = useCallback(() => {
    if (!selectedSubsystemId) return;
    setPromotion((current) => ({ ...current, dispatching: true, progress: [], error: null }));
    // Progress is best-effort: a failed listen must never block the dispatch.
    let stopListening: (() => void) | null = null;
    let done = false;
    const finishListening = () => {
      done = true;
      stopListening?.();
      stopListening = null;
    };
    void listen<PromotionProgress>('promotion-progress', (event) => {
      if (done) return;
      const step = event.payload;
      setPromotion((current) => ({ ...current, progress: [...current.progress, step] }));
    })
      .then((stop) => {
        if (done) stop();
        else stopListening = stop;
      })
      .catch(() => {});
    void api
      .dispatchPromotion({
        subsystemId: selectedSubsystemId,
        sourceCommitShas: promotion.shas,
        mode: promotion.mode,
        baseSha: promotion.mode === 'cumulative' ? promotion.baseSha : null,
        targetBranch,
        confirmationAcknowledged: true,
        delivery: promotion.delivery,
        onConflict: 'take_subsystem',
        message: promotion.message,
        excludedPaths: promotion.excludedPaths,
      })
      .then((result) => {
        finishListening();
        setPromotion((current) => ({ ...current, dispatching: false, result }));
        if (result.delivery === 'local_direct') {
          pushNotice({
            tone: 'success',
            title: `Committed to ${targetBranch} locally — push when ready`,
            message: `${result.branch} · commit ${result.commitSha ?? '?'}. No push, no PR — run the listed push command to push it yourself.`,
          });
        } else if (result.delivery === 'local') {
          pushNotice({
            tone: 'success',
            title: 'Local branch + commit created',
            message: `${result.branch} · target ${targetBranch}. No push, no PR — run the listed next commands to push it yourself.`,
          });
        } else if (result.delivery === 'worktree') {
          pushNotice({
            tone: 'success',
            title: `Staged into working tree on ${targetBranch} — no commit, no push`,
            message:
              'Nothing was committed or pushed, and integration status is unchanged. The commit is yours to make.',
          });
        } else {
          pushNotice({
            tone: 'success',
            title: result.prUrl
              ? `Integration PR #${result.prNumber ?? '?'} opened`
              : 'Promotion request accepted',
            message: result.prUrl
              ? `${result.branch} · ${result.prUrl} against ${targetBranch}.`
              : `${result.branch} · target ${targetBranch}. ${result.message}`,
          });
        }
        bumpDataVersion();
      })
      .catch((error: unknown) => {
        finishListening();
        setPromotion((current) => ({ ...current, dispatching: false, error: asApiError(error) }));
      });
  }, [
    promotion.shas,
    promotion.mode,
    promotion.baseSha,
    promotion.delivery,
    promotion.message,
    promotion.excludedPaths,
    selectedSubsystemId,
    targetBranch,
    pushNotice,
    bumpDataVersion,
  ]);

  const value = useMemo<AppContextValue>(
    () => ({
      route,
      navigate: setRoute,
      auth,
      refreshAuth: auth.reload,
      subsystems,
      refreshSubsystems: subsystems.reload,
      targetBranches,
      dataVersion,
      bumpDataVersion,
      selectedSubsystemId,
      selectSubsystem,
      selectedSubsystem,
      targetBranch,
      setTargetBranch,
      selectedShas,
      toggleSha,
      setSelectedShas,
      clearSelection,
      selectedCommits,
      rememberSelectedCommits,
      focusedSha,
      setFocusedSha,
      notice,
      pushNotice,
      dismissNotice,
      openExternal,
      promotion: {
        ...promotion,
        error: promotion.error ?? promotionDependencies.error,
        dependencies: promotionDependencies.data ?? [],
        dependenciesLoading: promotionDependencies.loading,
        preflight: promotionPreflight.data ?? null,
        preflightLoading: promotionPreflight.loading,
      },
      openPromotionDialog,
      closePromotionDialog,
      removePromotionSha,
      setPromotionMode,
      setPromotionBaseSha,
      setPromotionDelivery,
      setPromotionMessage,
      setPromotionExcludedPaths,
      addPromotionExcludedPath,
      dispatchPromotion,
    }),
    [
      route,
      auth,
      subsystems,
      targetBranches,
      setTargetBranch,
      dataVersion,
      bumpDataVersion,
      selectedSubsystemId,
      selectSubsystem,
      selectedSubsystem,
      targetBranch,
      selectedShas,
      toggleSha,
      setSelectedShas,
      clearSelection,
      selectedCommits,
      rememberSelectedCommits,
      focusedSha,
      notice,
      pushNotice,
      dismissNotice,
      openExternal,
      promotion,
      promotionDependencies,
      promotionPreflight,
      openPromotionDialog,
      closePromotionDialog,
      removePromotionSha,
      setPromotionMode,
      setPromotionBaseSha,
      setPromotionDelivery,
      setPromotionMessage,
      setPromotionExcludedPaths,
      addPromotionExcludedPath,
      dispatchPromotion,
    ],
  );

  return <AppContext.Provider value={value}>{children}</AppContext.Provider>;
}
