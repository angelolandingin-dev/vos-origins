import { useState } from 'react';
import { listen } from '@tauri-apps/api/event';
import { ExternalLink, GitPullRequestArrow, TriangleAlert } from 'lucide-react';

import { api, asApiError } from '../api/invoke';
import type { ApiError } from '../api/invoke';
import type {
  MigrateDependency,
  MigrateOutcome,
  MigratePath,
  MigratePlan,
  MigratePreview,
  MigrateRunResult,
  MigrationBlocker,
  PromotionProgress,
} from '../api/types';
import { useApp } from '../app/context';
import { PathLabel } from '../components/PathLabel';
import { cx } from '../components/ui/cx';
import {
  Button,
  Checkbox,
  EmptyState,
  ErrorState,
  Field,
  InlineAlert,
  KeyValue,
  LoadingState,
  Panel,
  Select,
} from '../components/ui/primitives';

interface PathChoice {
  excluded: string[];
  include: string[];
}

const EMPTY_CHOICE: PathChoice = { excluded: [], include: [] };

function candidatePaths(plan: MigratePlan): MigratePath[] {
  return plan.paths.filter((entry) => entry.state !== 'mainOnly');
}

function optInPaths(plan: MigratePlan): string[] {
  const paths = new Set(plan.mainOnly);
  for (const entry of plan.paths) {
    if (entry.state === 'mainOnly') paths.add(entry.path);
  }
  return Array.from(paths);
}

export function Migrate() {
  const app = useApp();
  const [branchChoice, setBranchChoice] = useState<string | null>(null);
  const [subsystemChoice, setSubsystemChoice] = useState<string[] | null>(null);
  const [message, setMessage] = useState('');
  const [preview, setPreview] = useState<MigratePreview | null>(null);
  const [previewLoading, setPreviewLoading] = useState(false);
  const [previewError, setPreviewError] = useState<ApiError | null>(null);
  const [choices, setChoices] = useState<Record<string, PathChoice>>({});
  const [running, setRunning] = useState(false);
  const [progress, setProgress] = useState<PromotionProgress[]>([]);
  const [runError, setRunError] = useState<ApiError | null>(null);
  const [result, setResult] = useState<MigrateRunResult | null>(null);

  const branches = app.targetBranches.data?.branches ?? [];
  const mainBranch =
    branchChoice ??
    app.targetBranches.data?.targetBranch ??
    branches.find((b) => b.allowed)?.name ??
    '';

  const enabledSubsystems = (app.subsystems.data ?? []).filter((subsystem) => subsystem.enabled);
  const enabledIds = enabledSubsystems.map((subsystem) => subsystem.id);
  const selectedIds = (subsystemChoice ?? enabledIds).filter((id) => enabledIds.includes(id));

  const plans = preview?.subsystems ?? [];
  const runnable = plans.filter((plan) => plan.blockers.length === 0);
  const hasWork = plans.some(
    (plan) =>
      candidatePaths(plan).length > 0 ||
      optInPaths(plan).length > 0 ||
      plan.dependencies.length > 0,
  );
  const canPreview =
    mainBranch !== '' && selectedIds.length > 0 && !previewLoading && !running && result === null;
  const canRun =
    preview !== null && result === null && runnable.length > 0 && !running && !previewLoading;
  const latestProgress = progress[progress.length - 1] ?? null;
  // Results are a report of the last run: changing the target would strand them, so reset explicitly.
  const selectionLocked = running || result !== null;

  const resetPreview = () => {
    setPreview(null);
    setPreviewError(null);
    setChoices({});
    setRunError(null);
    setProgress([]);
    setResult(null);
  };

  const changeMainBranch = (next: string) => {
    setBranchChoice(next);
    resetPreview();
  };

  const selectAllSubsystems = () => {
    setSubsystemChoice(null);
    resetPreview();
  };

  const clearSubsystems = () => {
    setSubsystemChoice([]);
    resetPreview();
  };

  const toggleSubsystem = (id: string) => {
    setSubsystemChoice(
      selectedIds.includes(id) ? selectedIds.filter((entry) => entry !== id) : [...selectedIds, id],
    );
    resetPreview();
  };

  const choiceFor = (plan: MigratePlan): PathChoice => choices[plan.subsystemId] ?? EMPTY_CHOICE;

  const setPlanChoice = (plan: MigratePlan, update: (current: PathChoice) => PathChoice) => {
    setChoices((current) => ({
      ...current,
      [plan.subsystemId]: update(current[plan.subsystemId] ?? EMPTY_CHOICE),
    }));
  };

  const toggleExcluded = (plan: MigratePlan, path: string, excluded: boolean) => {
    setPlanChoice(plan, (current) => ({
      ...current,
      excluded: excluded
        ? [...current.excluded, path]
        : current.excluded.filter((entry) => entry !== path),
    }));
  };

  const toggleInclude = (plan: MigratePlan, path: string, include: boolean) => {
    setPlanChoice(plan, (current) => ({
      ...current,
      include: include
        ? [...current.include, path]
        : current.include.filter((entry) => entry !== path),
    }));
  };

  const runPreview = () => {
    if (!canPreview) return;
    resetPreview();
    setPreviewLoading(true);
    api
      .previewMigration({ mainBranch, subsystemIds: selectedIds })
      .then((next) => {
        setPreview(next);
        setPreviewLoading(false);
      })
      .catch((error: unknown) => {
        setPreviewError(asApiError(error));
        setPreviewLoading(false);
      });
  };

  const run = () => {
    if (!canRun || !preview) return;
    const subsystems = preview.subsystems
      .filter((plan) => plan.blockers.length === 0)
      .map((plan) => {
        const choice = choiceFor(plan);
        return {
          subsystemId: plan.subsystemId,
          excludedPaths: candidatePaths(plan)
            .map((entry) => entry.path)
            .filter((path) => choice.excluded.includes(path)),
          includePaths: choice.include,
        };
      });
    const trimmedMessage = message.trim();
    const mainBranchForRun = preview.mainBranch;

    let stopListening: (() => void) | null = null;
    let done = false;
    const finishListening = () => {
      done = true;
      stopListening?.();
      stopListening = null;
    };

    setRunning(true);
    setRunError(null);
    setProgress([]);
    setResult(null);

    const startRun = () => {
      api
        .runMigration({
          mainBranch: mainBranchForRun,
          message: trimmedMessage === '' ? null : trimmedMessage,
          subsystems,
        })
        .then((next) => {
          finishListening();
          setRunning(false);
          setResult(next);
          app.bumpDataVersion();
        })
        .catch((error: unknown) => {
          finishListening();
          setRunning(false);
          setRunError(asApiError(error));
        });
    };

    // Register before invoking so the first per-subsystem progress event is not missed.
    void listen<PromotionProgress>('migration-progress', (event) => {
      if (done) return;
      setProgress((current) => [...current, event.payload]);
    })
      .then((stop) => {
        if (done) stop();
        else stopListening = stop;
      })
      .catch(() => {
        // Progress is best effort; the run result still reports every outcome.
      })
      .then(startRun);
  };

  const subsystemName = (id: string) =>
    preview?.subsystems.find((plan) => plan.subsystemId === id)?.name ??
    enabledSubsystems.find((subsystem) => subsystem.id === id)?.name ??
    id;

  const progressText = running
    ? latestProgress
      ? `${latestProgress.label}${
          latestProgress.index != null && latestProgress.total != null
            ? ` · ${latestProgress.index} / ${latestProgress.total}`
            : ''
        }`
      : 'Starting'
    : result
      ? 'Finished'
      : 'Not started';

  return (
    <div className="flex h-full min-h-0 flex-col">
      <header className="flex flex-wrap items-center justify-between gap-3 px-5 pb-3 pt-4">
        <h1 className="text-[14px] font-semibold text-fg">Migrate</h1>
        <div className="flex items-center gap-2">
          <span className="text-[11px] text-muted">Main branch</span>
          <Select
            ariaLabel="Main branch to migrate from"
            className="w-[220px]"
            value={mainBranch}
            onValueChange={changeMainBranch}
            disabled={selectionLocked || app.targetBranches.loading}
            options={branches.map((branch) => ({
              value: branch.name,
              label: branch.name,
              disabled: !branch.allowed,
              title: branch.allowed
                ? `tip ${branch.headSha.slice(0, 7)}`
                : (branch.disallowedReason ?? 'Not permitted by registry policy'),
            }))}
          />
        </div>
      </header>

      <div className="grid min-h-0 flex-1 gap-3 px-3 pb-3 xl:grid-cols-[minmax(0,1fr)_380px]">
        <div className="scroll-area min-h-0 space-y-3">
          <Panel
            title="Subsystems"
            subtitle={
              enabledIds.length > 0
                ? `${selectedIds.length} of ${enabledIds.length} selected`
                : 'None enabled'
            }
            actions={
              <>
                <Button
                  variant="ghost"
                  size="sm"
                  onClick={selectAllSubsystems}
                  disabled={selectionLocked || selectedIds.length === enabledIds.length}
                >
                  All
                </Button>
                <Button
                  variant="ghost"
                  size="sm"
                  onClick={clearSubsystems}
                  disabled={selectionLocked || selectedIds.length === 0}
                >
                  None
                </Button>
              </>
            }
            bodyClassName="p-3"
          >
            {app.subsystems.error ? (
              <ErrorState compact error={app.subsystems.error} onRetry={app.subsystems.reload} />
            ) : app.subsystems.loading && enabledSubsystems.length === 0 ? (
              <LoadingState label="Loading subsystems" />
            ) : enabledSubsystems.length === 0 ? (
              <p className="text-[11px] text-muted">No enabled subsystems in the registry.</p>
            ) : (
              <ul className="grid gap-x-3 sm:grid-cols-2">
                {enabledSubsystems.map((subsystem) => (
                  <li key={subsystem.id} className="flex h-[29px] items-center gap-2">
                    <label className="flex min-w-0 flex-1 cursor-pointer items-center gap-2 text-xs text-fg">
                      <Checkbox
                        checked={selectedIds.includes(subsystem.id)}
                        disabled={selectionLocked}
                        onChange={() => toggleSubsystem(subsystem.id)}
                        label={`Migrate into ${subsystem.name}`}
                      />
                      <span className="truncate" title={subsystem.repository}>
                        {subsystem.name}
                      </span>
                    </label>
                    <span className="mono shrink-0 text-[11px] text-muted">
                      {subsystem.sourceBranch}
                    </span>
                  </li>
                ))}
              </ul>
            )}
          </Panel>

          {result ? (
            <div className="space-y-3">
              <div className="flex items-center justify-between gap-2 px-1">
                <h2 className="text-xs font-semibold text-fg">Results</h2>
                <Button variant="secondary" size="sm" onClick={resetPreview} disabled={running}>
                  New migration
                </Button>
              </div>
              {result.outcomes.map((outcome) => (
                <OutcomeCard
                  key={outcome.subsystemId}
                  outcome={outcome}
                  name={subsystemName(outcome.subsystemId)}
                  onOpenExternal={app.openExternal}
                />
              ))}
            </div>
          ) : previewLoading ? (
            <Panel title="Preview" bodyClassName="p-3">
              <LoadingState label="Reading main-branch changes" />
            </Panel>
          ) : previewError ? (
            <Panel title="Preview" bodyClassName="p-3">
              <ErrorState error={previewError} onRetry={runPreview} />
            </Panel>
          ) : !preview ? (
            <Panel title="Preview" bodyClassName="p-3">
              <EmptyState
                icon={GitPullRequestArrow}
                title="No preview yet"
                message="Select the subsystems to migrate into, then preview the main-branch changes each one would receive."
              />
            </Panel>
          ) : plans.length === 0 ||
            (!hasWork && !plans.some((plan) => plan.blockers.length > 0)) ? (
            <Panel title="Preview" bodyClassName="p-3">
              <EmptyState
                icon={GitPullRequestArrow}
                title="Nothing to migrate"
                message={
                  plans.length === 0
                    ? 'The preview returned no subsystems.'
                    : `No main-branch changes reach the selected subsystems as of ${preview.mainSha.slice(0, 7)}.`
                }
              />
            </Panel>
          ) : (
            <div className="space-y-3">
              {plans.map((plan) => (
                <PlanCard
                  key={plan.subsystemId}
                  plan={plan}
                  choice={choiceFor(plan)}
                  disabled={running}
                  onToggleExcluded={(path, checked) => toggleExcluded(plan, path, checked)}
                  onToggleInclude={(path, checked) => toggleInclude(plan, path, checked)}
                />
              ))}
            </div>
          )}
        </div>

        <div className="scroll-area min-h-0 space-y-3">
          <Panel
            title="Migration"
            subtitle={mainBranch === '' ? 'No main branch selected' : `main: ${mainBranch}`}
            bodyClassName="space-y-3 p-3"
          >
            <dl className="divide-y divide-line border-y border-line px-1">
              <KeyValue label="Main commit">
                <span className="mono text-right" title={preview?.mainSha}>
                  {preview ? preview.mainSha.slice(0, 7) : '—'}
                </span>
              </KeyValue>
              <KeyValue label="Selected">
                <span className="mono text-right tabular-nums">{selectedIds.length}</span>
              </KeyValue>
              <KeyValue label="Ready to run">
                <span className="mono text-right tabular-nums">
                  {preview ? runnable.length : '—'}
                </span>
              </KeyValue>
            </dl>

            <Field
              label="Commit message"
              htmlFor="migrate-commit-message"
              hint="Optional. Used for every subsystem; blank lets the backend write the default subject."
            >
              <textarea
                id="migrate-commit-message"
                className="input min-h-[60px] py-2"
                rows={3}
                placeholder="Migrate main into subsystems"
                value={message}
                disabled={running}
                onChange={(event) => setMessage(event.target.value)}
              />
            </Field>

            <Button
              variant="secondary"
              className="w-full"
              loading={previewLoading}
              disabled={!canPreview}
              onClick={runPreview}
            >
              Preview migration
            </Button>
            <Button
              variant="primary"
              className="w-full"
              icon={GitPullRequestArrow}
              loading={running}
              disabled={!canRun}
              onClick={run}
            >
              Run migration
            </Button>

            <p
              role="status"
              aria-live="polite"
              className="mono min-h-[16px] truncate text-[11px] text-muted"
              title={latestProgress?.detail ?? undefined}
            >
              {progressText}
            </p>

            {runError ? (
              <InlineAlert
                tone="danger"
                title={runError.message}
                actions={
                  <Button variant="ghost" size="sm" onClick={run} disabled={!canRun}>
                    Retry
                  </Button>
                }
              >
                {runError.remedy ? <p>{runError.remedy}</p> : null}
                {runError.details ? (
                  <pre className="mono max-h-40 overflow-auto whitespace-pre-wrap text-[11px]">
                    {runError.details}
                  </pre>
                ) : null}
              </InlineAlert>
            ) : null}
          </Panel>
        </div>
      </div>
    </div>
  );
}

function PlanCard({
  plan,
  choice,
  disabled,
  onToggleExcluded,
  onToggleInclude,
}: {
  plan: MigratePlan;
  choice: PathChoice;
  disabled: boolean;
  onToggleExcluded: (path: string, checked: boolean) => void;
  onToggleInclude: (path: string, checked: boolean) => void;
}) {
  const candidates = candidatePaths(plan);
  const optIn = optInPaths(plan);
  const blocked = plan.blockers.length > 0;
  const included =
    candidates.filter((entry) => !choice.excluded.includes(entry.path)).length +
    choice.include.length;

  return (
    <Panel
      title={plan.name}
      subtitle={
        <span className="mono">
          {plan.sourceBranch} → {plan.branch}
        </span>
      }
      actions={
        <span className="flex shrink-0 items-center gap-1.5 text-[11px]">
          <span
            aria-hidden="true"
            className={cx(
              'h-1.5 w-1.5 rounded-full',
              blocked ? 'bg-status-blocked' : 'bg-status-eligible',
            )}
          />
          <span className={blocked ? 'text-status-blocked-ink' : 'text-muted'}>
            {blocked ? 'Blocked' : 'Ready'}
          </span>
          <span className="mono tabular-nums text-muted">{included} paths</span>
        </span>
      }
      bodyClassName="space-y-3 p-3"
    >
      <BlockerAlerts blockers={plan.blockers} />
      <WarningLines warnings={plan.warnings} />

      <section className="space-y-1.5">
        <h3 className="text-[11px] font-semibold tracking-wide text-muted uppercase">
          Paths from main
        </h3>
        {candidates.length === 0 ? (
          <p className="text-[11px] text-muted">No changed paths reach this subsystem.</p>
        ) : (
          <ul className="divide-y divide-line overflow-hidden rounded-md border border-line bg-surface">
            {candidates.map((entry) => (
              <li key={entry.path} className="flex h-[29px] items-center gap-2 px-2.5">
                <label className="flex min-w-0 flex-1 cursor-pointer items-center gap-2">
                  <Checkbox
                    checked={!choice.excluded.includes(entry.path)}
                    disabled={disabled}
                    onChange={(event) => onToggleExcluded(entry.path, event.target.checked)}
                    label={`Include ${entry.path} in the migration`}
                  />
                  <PathLabel path={entry.path} title={entry.path} className="flex-1 text-[11px]" />
                </label>
                {entry.state === 'diverged' ? (
                  <span
                    className="shrink-0 text-[11px] text-status-unknown-ink"
                    title="Changed in main and in the subsystem"
                  >
                    diverged
                  </span>
                ) : null}
              </li>
            ))}
          </ul>
        )}
      </section>

      {optIn.length > 0 ? (
        <section className="space-y-1.5">
          <h3 className="text-[11px] font-semibold tracking-wide text-muted uppercase">
            Main-only paths
          </h3>
          <ul className="divide-y divide-line overflow-hidden rounded-md border border-line bg-surface">
            {optIn.map((path) => (
              <li key={path} className="flex h-[29px] items-center gap-2 px-2.5">
                <label className="flex min-w-0 flex-1 cursor-pointer items-center gap-2">
                  <Checkbox
                    checked={choice.include.includes(path)}
                    disabled={disabled}
                    onChange={(event) => onToggleInclude(path, event.target.checked)}
                    label={`Add ${path} from main to the subsystem`}
                  />
                  <PathLabel path={path} title={path} muted className="flex-1 text-[11px]" />
                </label>
                <span className="shrink-0 text-[11px] text-muted">not in subsystem</span>
              </li>
            ))}
          </ul>
        </section>
      ) : null}

      <DependencyList dependencies={plan.dependencies} />

      {plan.excluded.length > 0 ? (
        <details className="overflow-hidden rounded-md border border-line bg-surface">
          <summary className="flex h-[29px] cursor-pointer items-center justify-between gap-2 px-2.5 text-[11px]">
            <span className="font-semibold tracking-wide text-muted uppercase">Not migratable</span>
            <span className="mono tabular-nums text-muted">{plan.excluded.length}</span>
          </summary>
          <ul className="scroll-area max-h-40 divide-y divide-line border-t border-line">
            {plan.excluded.map((path) => (
              <li key={path} className="flex h-[29px] items-center px-2.5">
                <PathLabel path={path} title={path} muted className="flex-1 text-[11px]" />
              </li>
            ))}
          </ul>
        </details>
      ) : null}
    </Panel>
  );
}

function OutcomeCard({
  outcome,
  name,
  onOpenExternal,
}: {
  outcome: MigrateOutcome;
  name: string;
  onOpenExternal: (url: string) => void;
}) {
  const pr = outcome.pullRequest;
  const prUrl = pr?.url ?? null;

  return (
    <Panel
      title={name}
      subtitle={<span className="mono">{outcome.branch}</span>}
      actions={
        <span className="flex shrink-0 items-center gap-1.5 text-[11px]">
          <span
            aria-hidden="true"
            className={cx(
              'h-1.5 w-1.5 rounded-full',
              outcome.ok ? 'bg-status-applied' : 'bg-status-blocked',
            )}
          />
          <span className={outcome.ok ? 'text-status-applied-ink' : 'text-status-blocked-ink'}>
            {outcome.ok ? 'Migrated' : 'Blocked'}
          </span>
        </span>
      }
      bodyClassName="space-y-3 p-3"
    >
      {outcome.ok ? (
        <InlineAlert
          tone="success"
          title={
            pr
              ? pr.number != null
                ? `Pull request #${pr.number} ${pr.state}`
                : `Pull request ${pr.state}`
              : 'Migration committed'
          }
          actions={
            prUrl ? (
              <Button
                variant="secondary"
                size="sm"
                icon={ExternalLink}
                onClick={() => onOpenExternal(prUrl)}
              >
                Open pull request
              </Button>
            ) : undefined
          }
        />
      ) : null}

      <dl className="divide-y divide-line border-y border-line px-1">
        <KeyValue label="Branch">
          <span className="mono text-right">{outcome.branch}</span>
        </KeyValue>
        <KeyValue label="Commit">
          <span className="mono text-right" title={outcome.commitSha ?? undefined}>
            {outcome.commitSha ? outcome.commitSha.slice(0, 7) : '—'}
          </span>
        </KeyValue>
        <KeyValue label="Applied">
          <span className="mono text-right tabular-nums">{outcome.applied.length} paths</span>
        </KeyValue>
        {pr ? (
          <KeyValue label="Pull request">
            <span className="mono text-right">
              {pr.head} → {pr.base} · {pr.state}
            </span>
          </KeyValue>
        ) : null}
      </dl>

      {outcome.message ? (
        <p className="text-[11px] leading-relaxed text-muted">{outcome.message}</p>
      ) : null}

      <BlockerAlerts blockers={outcome.blockers} />
      <WarningLines warnings={outcome.warnings} />
      <DependencyList dependencies={outcome.dependencies} />
    </Panel>
  );
}

function DependencyList({ dependencies }: { dependencies: MigrateDependency[] }) {
  if (dependencies.length === 0) return null;
  return (
    <section className="space-y-1.5">
      <h3 className="text-[11px] font-semibold tracking-wide text-muted uppercase">
        Dependency changes
      </h3>
      <ul className="divide-y divide-line overflow-hidden rounded-md border border-line bg-surface">
        {dependencies.map((dependency) => (
          <li
            key={`${dependency.action}-${dependency.section}-${dependency.name}`}
            className="flex h-[29px] items-center gap-2 px-2.5 text-[11px]"
          >
            <span className="w-[46px] shrink-0 tracking-wide text-muted uppercase">
              {dependency.action}
            </span>
            <span className="mono min-w-0 flex-1 truncate text-fg" title={dependency.name}>
              {dependency.name}
            </span>
            <span className="shrink-0 text-muted">
              {dependency.action === 'move' && dependency.fromSection
                ? `${dependency.fromSection} → `
                : ''}
              {dependency.section}
            </span>
            <span
              className="mono min-w-0 max-w-[45%] shrink-0 truncate text-right text-muted"
              title={`${dependency.fromSpec ?? '(absent)'} → ${dependency.spec}`}
            >
              {dependency.fromSpec ?? '(absent)'} → {dependency.spec}
            </span>
          </li>
        ))}
      </ul>
    </section>
  );
}

function BlockerAlerts({ blockers }: { blockers: MigrationBlocker[] }) {
  if (blockers.length === 0) return null;
  return (
    <ul className="space-y-2">
      {blockers.map((blocker, index) => (
        <li key={`${blocker.code}-${index}`}>
          <InlineAlert tone="danger" title={blocker.message}>
            {blocker.details ? (
              <pre className="mono max-h-40 overflow-auto whitespace-pre-wrap text-[11px]">
                {blocker.details}
              </pre>
            ) : null}
          </InlineAlert>
        </li>
      ))}
    </ul>
  );
}

function WarningLines({ warnings }: { warnings: string[] }) {
  if (warnings.length === 0) return null;
  return (
    <ul className="space-y-1">
      {warnings.map((warning) => (
        <li
          key={warning}
          className="flex items-start gap-1.5 text-[11px] leading-relaxed text-muted"
        >
          <TriangleAlert
            className="mt-0.5 h-3 w-3 shrink-0 text-status-unknown"
            aria-hidden="true"
          />
          <span>{warning}</span>
        </li>
      ))}
    </ul>
  );
}
