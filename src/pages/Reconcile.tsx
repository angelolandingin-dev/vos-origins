import { useMemo, useState } from 'react';
import { listen } from '@tauri-apps/api/event';
import { ArrowDownToLine, FolderSync, RefreshCw } from 'lucide-react';

import { api, asApiError } from '../api/invoke';
import type { ApiError } from '../api/invoke';
import type {
  PromotionDelivery,
  PromotionProgress,
  ReconcileDirection,
  ReconcileFile,
  ReconcileFileDiff,
  ReconcileOutcome,
  ReconcilePlan,
  ReconcileTreeSnapshot,
  SubsystemSummary,
} from '../api/types';
import { useApp, useAsyncData } from '../app/context';
import { SubsystemSelector } from '../components/SubsystemSelector';
import { DirectorySelector } from '../components/reconcile/DirectorySelector';
import { ReconcileReview } from '../components/reconcile/ReconcileReview';
import { addFolderWithConsolidation, scopesEqual } from '../components/reconcile/scopeUtils';
import { cx } from '../components/ui/cx';
import { Button, EmptyState, ErrorState, LoadingState, Select } from '../components/ui/primitives';

/** Reconcile: snapshot-based folder mirror between a subsystem and main. */
export function Reconcile() {
  const app = useApp();
  const [direction, setDirection] = useState<ReconcileDirection>('subsystem_to_main');

  const enabledSubsystems = useMemo(
    () => (app.subsystems.data ?? []).filter((subsystem) => subsystem.enabled),
    [app.subsystems.data],
  );
  const subsystemId = useMemo(() => {
    if (
      app.selectedSubsystem?.enabled &&
      enabledSubsystems.some((s) => s.id === app.selectedSubsystem?.id)
    ) {
      return app.selectedSubsystem.id;
    }
    return enabledSubsystems[0]?.id ?? '';
  }, [enabledSubsystems, app.selectedSubsystem]);

  const selectedSubsystem = useMemo(
    () => enabledSubsystems.find((s) => s.id === subsystemId) ?? null,
    [enabledSubsystems, subsystemId],
  );

  const targetBranches = app.targetBranches;
  const mainBranch = app.targetBranch || targetBranches.data?.targetBranch || '';

  if (enabledSubsystems.length === 0 && !app.subsystems.loading) {
    return (
      <div className="p-5">
        <EmptyState
          icon={FolderSync}
          title="No enabled subsystem"
          message="Register or enable a subsystem in Settings before reconciling folders."
          action={
            <Button variant="primary" onClick={() => app.navigate('settings')}>
              Open Settings
            </Button>
          }
        />
      </div>
    );
  }

  return (
    <ReconcileWorkspace
      key={subsystemId}
      direction={direction}
      onDirectionChange={setDirection}
      subsystemId={subsystemId}
      subsystemName={selectedSubsystem?.name ?? subsystemId}
      subsystems={enabledSubsystems}
      onSubsystemChange={(next) => app.selectSubsystem(next)}
      mainBranch={mainBranch}
      branchOptions={(targetBranches.data?.branches ?? []).map((b) => ({
        value: b.name,
        label: `${b.name}${b.isDefault ? ' (default)' : ''}`,
        disabled: !b.allowed,
        title: b.allowed ? undefined : (b.disallowedReason ?? 'Not permitted'),
      }))}
      branchesLoading={targetBranches.loading}
      branchesError={targetBranches.error}
      onReloadBranches={targetBranches.reload}
      onMainBranchChange={(next) => app.setTargetBranch(next)}
    />
  );
}

interface ReconcileWorkspaceProps {
  direction: ReconcileDirection;
  onDirectionChange: (next: ReconcileDirection) => void;
  subsystemId: string;
  subsystemName: string;
  subsystems: SubsystemSummary[];
  onSubsystemChange: (next: string) => void;
  mainBranch: string;
  branchOptions: Array<{ value: string; label: string; disabled?: boolean; title?: string }>;
  branchesLoading: boolean;
  branchesError: ApiError | null;
  onReloadBranches: () => void;
  onMainBranchChange: (next: string) => void;
}

function ReconcileWorkspace({
  direction,
  onDirectionChange,
  subsystemId,
  subsystemName,
  subsystems,
  onSubsystemChange,
  mainBranch,
  branchOptions,
  branchesLoading,
  branchesError,
  onReloadBranches,
  onMainBranchChange,
}: ReconcileWorkspaceProps) {
  const app = useApp();
  const [step, setStep] = useState<'select' | 'review'>('select');
  const [draftFolders, setDraftFolders] = useState<string[] | null>(null);
  const [savedOverride, setSavedOverride] = useState<string[] | null>(null);
  const [saving, setSaving] = useState(false);
  const [saveError, setSaveError] = useState<string | null>(null);
  const [consolidatedNotice, setConsolidatedNotice] = useState<string | null>(null);

  const [delivery, setDelivery] = useState<PromotionDelivery>('local_direct');
  const [excludedPaths, setExcludedPaths] = useState<string[]>([]);
  const [inspectedFile, setInspectedFile] = useState<ReconcileFile | null>(null);
  const [message, setMessage] = useState('');
  const [confirmed, setConfirmed] = useState(false);
  const [running, setRunning] = useState(false);
  const [progress, setProgress] = useState<PromotionProgress | null>(null);
  const [runError, setRunError] = useState<ApiError | null>(null);
  const [outcome, setOutcome] = useState<ReconcileOutcome | null>(null);

  const treeQuery = useAsyncData<ReconcileTreeSnapshot | null>(
    () =>
      subsystemId && mainBranch
        ? api.inspectReconcileTree(subsystemId, mainBranch, direction)
        : Promise.resolve(null),
    [subsystemId, mainBranch, direction, app.dataVersion],
  );

  const savedFolders = useMemo(
    () => savedOverride ?? treeQuery.data?.savedFolders ?? [],
    [savedOverride, treeQuery.data?.savedFolders],
  );
  const selectedFolders = useMemo(() => draftFolders ?? savedFolders, [draftFolders, savedFolders]);

  const effectiveDelivery: PromotionDelivery =
    direction === 'main_to_subsystem' ? 'push_pr' : delivery;
  const foldersKey = selectedFolders.join('\0');
  const excludedKey = excludedPaths.join('\0');

  const planQuery = useAsyncData<ReconcilePlan | null>(
    () =>
      step === 'review' && subsystemId && mainBranch && selectedFolders.length > 0
        ? api.previewReconcile({
            subsystemId,
            mainBranch,
            direction,
            folders: selectedFolders,
            excludedPaths,
            delivery: effectiveDelivery,
          })
        : Promise.resolve(null),
    [
      step,
      subsystemId,
      mainBranch,
      direction,
      foldersKey,
      excludedKey,
      effectiveDelivery,
      app.dataVersion,
    ],
  );

  const diffQuery = useAsyncData<ReconcileFileDiff | null>(
    () =>
      step === 'review' && inspectedFile && subsystemId && mainBranch
        ? api.getReconcileFileDiff(
            subsystemId,
            mainBranch,
            direction,
            inspectedFile.path,
            inspectedFile.action,
          )
        : Promise.resolve(null),
    [step, subsystemId, mainBranch, direction, inspectedFile?.path, inspectedFile?.action],
  );

  const resetReviewState = () => {
    setExcludedPaths([]);
    setInspectedFile(null);
    setConfirmed(false);
    setProgress(null);
    setRunError(null);
    setOutcome(null);
  };

  const toggleFolder = (path: string) => {
    resetReviewState();
    setSaveError(null);
    if (selectedFolders.includes(path)) {
      setDraftFolders(selectedFolders.filter((item) => item !== path));
      setConsolidatedNotice(null);
      return;
    }
    const { next, replaced } = addFolderWithConsolidation(selectedFolders, path);
    setDraftFolders(next);
    if (replaced.length > 0) {
      setConsolidatedNotice(
        `Replaced ${replaced.length} nested folder${replaced.length === 1 ? '' : 's'} (${replaced.join(', ')}) covered by ${path}.`,
      );
    } else {
      setConsolidatedNotice(null);
    }
  };

  const removeFolder = (path: string) => {
    resetReviewState();
    setDraftFolders(selectedFolders.filter((item) => item !== path));
  };

  const saveScope = () => {
    if (saving || scopesEqual(selectedFolders, savedFolders)) return;
    setSaving(true);
    setSaveError(null);
    void api
      .saveReconcileScope(subsystemId, selectedFolders)
      .then((saved) => {
        setSavedOverride(saved);
        setDraftFolders(null);
        setSaving(false);
        app.pushNotice({
          tone: 'success',
          title: 'Reconcile folders saved',
          message:
            saved.length === 0
              ? `Cleared saved folder list for ${subsystemName}.`
              : `Saved ${saved.length} folder${saved.length === 1 ? '' : 's'} for ${subsystemName}.`,
        });
      })
      .catch((error: unknown) => {
        setSaveError(asApiError(error).message);
        setSaving(false);
      });
  };

  const resetScope = () => {
    resetReviewState();
    setConsolidatedNotice(null);
    setSaveError(null);
    setDraftFolders(null);
  };

  const clearScope = () => {
    resetReviewState();
    setConsolidatedNotice(null);
    setSaveError(null);
    setDraftFolders([]);
  };

  const startRun = () => {
    if (running || !planQuery.data || !confirmed) return;
    setRunError(null);
    setProgress(null);
    setRunning(true);

    let stopListening: (() => void) | null = null;
    let done = false;
    const finishListening = () => {
      done = true;
      stopListening?.();
      stopListening = null;
    };

    void listen<PromotionProgress>('reconcile-progress', (event) => {
      if (done) return;
      setProgress(event.payload);
    })
      .then((stop) => {
        if (done) stop();
        else stopListening = stop;
      })
      .catch(() => undefined)
      .then(() =>
        api.runReconcile({
          subsystemId,
          mainBranch,
          direction,
          folders: selectedFolders,
          excludedPaths,
          delivery: effectiveDelivery,
          message: message.trim() === '' ? null : message.trim(),
          confirmationAcknowledged: true,
        }),
      )
      .then((result) => {
        finishListening();
        setOutcome(result);
        setRunning(false);
        app.bumpDataVersion();
      })
      .catch((error: unknown) => {
        finishListening();
        setRunError(asApiError(error));
        setRunning(false);
      });
  };

  return (
    <div className="flex h-full min-h-0 flex-col">
      <header className="flex min-h-[48px] flex-wrap items-center justify-between gap-3 border-b border-line px-5 py-2.5">
        <div className="flex items-center gap-3">
          <h1 className="text-[14px] font-semibold text-fg">Reconcile</h1>
          <div className="flex items-center gap-1.5 text-[11px]">
            <button
              type="button"
              disabled={running || step === 'select'}
              onClick={() => {
                resetReviewState();
                setStep('select');
              }}
              className={cx(
                step === 'select'
                  ? 'font-semibold text-fg'
                  : 'text-muted hover:text-fg disabled:cursor-default',
              )}
            >
              1 Select folders
            </button>
            <span className="text-disabled" aria-hidden="true">
              ·
            </span>
            <span className={step === 'review' ? 'font-semibold text-fg' : 'text-muted'}>
              2 Review &amp; reconcile
            </span>
          </div>
        </div>

        <div className="flex flex-wrap items-center gap-2">
          <div
            role="group"
            aria-label="Reconcile direction"
            className="flex h-[25px] overflow-hidden rounded-md border border-line bg-surface"
          >
            {(
              [
                { id: 'subsystem_to_main', label: 'Subsystem → Main' },
                { id: 'main_to_subsystem', label: 'Main → Subsystem' },
              ] as const
            ).map((opt, index) => (
              <button
                key={opt.id}
                type="button"
                disabled={running}
                onClick={() => {
                  resetReviewState();
                  onDirectionChange(opt.id);
                }}
                className={cx(
                  'px-2.5 text-[11px] transition-colors disabled:opacity-60',
                  index > 0 && 'border-l border-line',
                  direction === opt.id
                    ? 'bg-raised font-semibold text-fg'
                    : 'text-muted hover:bg-hover hover:text-fg',
                )}
              >
                {opt.label}
              </button>
            ))}
          </div>

          <SubsystemSelector
            subsystems={subsystems}
            selectedId={subsystemId || null}
            onSelect={onSubsystemChange}
            disabled={running || subsystems.length === 0}
          />

          <div className="flex items-center gap-1.5">
            <span className="text-[11px] text-muted">Main</span>
            <Select
              ariaLabel="Main branch"
              value={mainBranch}
              onValueChange={(next) => {
                resetReviewState();
                onMainBranchChange(next);
              }}
              options={branchOptions}
              disabled={running || branchesLoading || branchOptions.length === 0}
              className="w-[170px]"
              title={branchesError?.message ?? 'Main repository branch'}
            />
          </div>

          {branchesError ? (
            <button
              type="button"
              onClick={onReloadBranches}
              className="text-[11px] text-status-blocked-ink underline"
            >
              Retry branches
            </button>
          ) : null}

          <Button
            variant="secondary"
            icon={ArrowDownToLine}
            loading={app.pullingCheckouts}
            disabled={running || app.pullingCheckouts}
            onClick={() => app.pullLatest(subsystemId || null)}
            title="Fetch the latest refs for this subsystem and main"
          >
            {app.pullingCheckouts ? 'Pulling…' : 'Pull latest'}
          </Button>

          <Button
            variant="ghost"
            size="sm"
            icon={RefreshCw}
            loading={treeQuery.loading || (step === 'review' && planQuery.loading)}
            disabled={running || !mainBranch || !subsystemId}
            onClick={() => {
              treeQuery.reload();
              if (step === 'review') planQuery.reload();
            }}
          >
            Refresh
          </Button>
        </div>
      </header>

      {step === 'select' ? (
        branchesLoading && !mainBranch ? (
          <LoadingState label="Loading branches" />
        ) : !mainBranch ? (
          <EmptyState title="No main branch available" />
        ) : treeQuery.loading && !treeQuery.data ? (
          <LoadingState label="Inspecting repository directories" />
        ) : treeQuery.error ? (
          <div className="p-5">
            <ErrorState error={treeQuery.error} onRetry={treeQuery.reload} />
          </div>
        ) : treeQuery.data ? (
          <DirectorySelector
            key={treeQuery.data.subsystemId}
            snapshot={treeQuery.data}
            loading={treeQuery.loading}
            selectedFolders={selectedFolders}
            savedFolders={savedFolders}
            saving={saving}
            saveError={saveError}
            locked={running}
            consolidatedNotice={consolidatedNotice}
            onDismissConsolidated={() => setConsolidatedNotice(null)}
            onToggleFolder={toggleFolder}
            onRemoveFolder={removeFolder}
            onSaveScope={saveScope}
            onResetScope={resetScope}
            onClearScope={clearScope}
            onProceed={() => {
              resetReviewState();
              setStep('review');
            }}
          />
        ) : null
      ) : (
        <ReconcileReview
          direction={direction}
          subsystemName={subsystemName}
          folders={selectedFolders}
          delivery={effectiveDelivery}
          onDeliveryChange={(next) => {
            setConfirmed(false);
            setRunError(null);
            setDelivery(next);
          }}
          plan={planQuery.data}
          planLoading={planQuery.loading}
          planError={planQuery.error}
          onReloadPlan={planQuery.reload}
          excludedPaths={excludedPaths}
          onTogglePath={(path, included) => {
            setConfirmed(false);
            setRunError(null);
            setExcludedPaths((current) =>
              included
                ? current.filter((p) => p !== path)
                : current.includes(path)
                  ? current
                  : [...current, path],
            );
          }}
          onSelectAll={() => {
            setConfirmed(false);
            setRunError(null);
            setExcludedPaths([]);
          }}
          onSelectNone={() => {
            if (!planQuery.data) return;
            setConfirmed(false);
            setRunError(null);
            setExcludedPaths(planQuery.data.files.map((f) => f.path));
          }}
          inspectedFile={inspectedFile}
          onInspectFile={setInspectedFile}
          fileDiff={diffQuery.data}
          diffLoading={Boolean(inspectedFile && diffQuery.loading)}
          diffError={inspectedFile ? diffQuery.error : null}
          onReloadDiff={diffQuery.reload}
          message={message}
          onMessageChange={setMessage}
          confirmed={confirmed}
          onConfirmedChange={setConfirmed}
          running={running}
          progress={progress}
          runError={runError}
          outcome={outcome}
          onBackToFolders={() => {
            resetReviewState();
            setStep('select');
          }}
          onRun={startRun}
          onOpenExternal={(url) => app.openExternal(url)}
          onDone={() => {
            resetReviewState();
            setStep('select');
            treeQuery.reload();
          }}
        />
      )}
    </div>
  );
}
