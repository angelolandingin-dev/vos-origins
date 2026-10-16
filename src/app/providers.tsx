import { useCallback, useMemo, useState, type ReactNode } from 'react';

import { api, asApiError } from '../api/invoke';
import type {
  CommitSummary,
  DependencyChange,
  PromotionDelivery,
  PromotionMode,
  PromotionOnConflict,
  PromotionPreflight,
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

type PromotionInternal = Omit<
  PromotionState,
  'dependencies' | 'dependenciesLoading' | 'preflight' | 'preflightLoading'
>;

export function AppDataProvider({ children }: { children: ReactNode }) {
  const [route, setRoute] = useState<Route>('subsystems');
  const [dataVersion, setDataVersion] = useState(0);
  const [selectedSubsystemIdSetting, setSelectedSubsystemId] = useState<string | null>(null);
  const [targetBranchSetting, setTargetBranch] = useState(DEFAULT_TARGET_BRANCH);
  const [selectedShas, setSelectedShasState] = useState<string[]>([]);
  const [focusedSha, setFocusedSha] = useState<string | null>(null);
  const [notice, setNotice] = useState<Notice | null>(null);
  const [promotion, setPromotion] = useState<PromotionInternal>({
    open: false,
    shas: [],
    commits: [],
    mode: 'individual',
    baseSha: '',
    delivery: 'local_direct',
    onConflict: 'fail',
    message: '',
    dispatching: false,
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

  // Stays aligned with the policy list once it loads, derived instead of written from an effect.
  const targetBranch = useMemo(() => {
    if (!allowedTargetBranches || allowedTargetBranches.length === 0) return targetBranchSetting;
    if (allowedTargetBranches.includes(targetBranchSetting)) return targetBranchSetting;
    return allowedTargetBranches[0] ?? targetBranchSetting;
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
      onConflict: promotion.onConflict,
      message: promotion.message,
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
    promotion.onConflict,
  ]);

  const pushNotice = useCallback((next: Omit<Notice, 'id'>) => {
    setNotice({ ...next, id: Date.now() });
  }, []);

  const dismissNotice = useCallback(() => setNotice(null), []);

  const bumpDataVersion = useCallback(() => setDataVersion((value) => value + 1), []);

  const selectSubsystem = useCallback((id: string) => {
    setSelectedSubsystemId(id);
    setSelectedShasState([]);
    setFocusedSha(null);
  }, []);

  const toggleSha = useCallback((sha: string) => {
    setSelectedShasState((current) =>
      current.includes(sha) ? current.filter((value) => value !== sha) : [...current, sha],
    );
  }, []);

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

  const openPromotionDialog = useCallback((shas: string[], commits: CommitSummary[]) => {
    setPromotion({
      open: true,
      shas,
      commits,
      mode: 'individual',
      baseSha: '',
      delivery: 'local_direct',
      onConflict: 'fail',
      message: '',
      dispatching: false,
      error: null,
      result: null,
    });
  }, []);

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

  const setPromotionOnConflict = useCallback((onConflict: PromotionOnConflict) => {
    setPromotion((current) => ({ ...current, onConflict }));
  }, []);

  const setPromotionMessage = useCallback((message: string) => {
    setPromotion((current) => ({ ...current, message }));
  }, []);

  const dispatchPromotion = useCallback(() => {
    if (!selectedSubsystemId) return;
    setPromotion((current) => ({ ...current, dispatching: true, error: null }));
    void api
      .dispatchPromotion({
        subsystemId: selectedSubsystemId,
        sourceCommitShas: promotion.shas,
        mode: promotion.mode,
        baseSha: promotion.mode === 'cumulative' ? promotion.baseSha : null,
        targetBranch,
        confirmationAcknowledged: true,
        delivery: promotion.delivery,
        onConflict: promotion.onConflict,
        message: promotion.message,
      })
      .then((result) => {
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
        setPromotion((current) => ({ ...current, dispatching: false, error: asApiError(error) }));
      });
  }, [
    promotion.shas,
    promotion.mode,
    promotion.baseSha,
    promotion.delivery,
    promotion.message,
    promotion.onConflict,
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
      setSelectedShas: setSelectedShasState,
      clearSelection: () => setSelectedShasState([]),
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
      setPromotionOnConflict,
      setPromotionMessage,
      dispatchPromotion,
    }),
    [
      route,
      auth,
      subsystems,
      targetBranches,
      dataVersion,
      bumpDataVersion,
      selectedSubsystemId,
      selectSubsystem,
      selectedSubsystem,
      targetBranch,
      selectedShas,
      toggleSha,
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
      setPromotionOnConflict,
      setPromotionMessage,
      dispatchPromotion,
    ],
  );

  return <AppContext.Provider value={value}>{children}</AppContext.Provider>;
}
