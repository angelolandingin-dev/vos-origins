import { useState } from 'react';
import { ArrowDownToLine, GitBranch, RefreshCw } from 'lucide-react';

import { useApp } from '../app/context';
import { CommitWorkspace } from '../components/CommitWorkspace';
import { SubsystemSelector } from '../components/SubsystemSelector';
import { cx } from '../components/ui/cx';
import { Button, EmptyState, LoadingState, Select } from '../components/ui/primitives';
import { SubsystemDiff } from './SubsystemDiff';

type PromoteView = 'commits' | 'file-drift';

/** Unified Subsystem → Main promotion workspace (by commits or by file drift). */
export function Promote() {
  const app = useApp();
  const [view, setView] = useState<PromoteView>('commits');
  const subsystems = app.subsystems.data ?? [];
  const branches = (app.targetBranches.data?.branches ?? []).filter((branch) => branch.allowed);

  return (
    <div className="flex h-full min-h-0 flex-col">
      <header className="flex min-h-[48px] flex-wrap items-center justify-between gap-3 border-b border-line px-5 py-2.5">
        <div className="flex min-w-0 items-center gap-3">
          <h1 className="text-[14px] font-semibold text-fg">Promote</h1>
          <div
            role="group"
            aria-label="Promote view mode"
            className="flex h-[25px] overflow-hidden rounded-md border border-line bg-surface"
          >
            {(
              [
                { id: 'commits', label: 'Commits' },
                { id: 'file-drift', label: 'File Drift' },
              ] as const
            ).map((opt, index) => (
              <button
                key={opt.id}
                type="button"
                onClick={() => setView(opt.id)}
                className={cx(
                  'px-2.5 text-[11px] transition-colors',
                  index > 0 && 'border-l border-line',
                  view === opt.id
                    ? 'bg-raised font-semibold text-fg'
                    : 'text-muted hover:bg-hover hover:text-fg',
                )}
              >
                {opt.label}
              </button>
            ))}
          </div>
        </div>

        <div className="flex flex-wrap items-center gap-2">
          {subsystems.length > 0 ? (
            <SubsystemSelector
              subsystems={subsystems}
              selectedId={app.selectedSubsystemId}
              onSelect={(id) => app.selectSubsystem(id)}
            />
          ) : null}
          <div className="flex items-center gap-1.5">
            <span className="text-[11px] text-muted">Target</span>
            <Select
              ariaLabel="Main-system target branch used for status computation"
              className="w-[180px]"
              value={app.targetBranch}
              onValueChange={(next) => app.setTargetBranch(next)}
              options={branches.map((branch) => ({ value: branch.name, label: branch.name }))}
            />
          </div>
          <Button
            variant="secondary"
            icon={ArrowDownToLine}
            loading={app.pullingCheckouts}
            disabled={app.pullingCheckouts}
            onClick={() => app.pullLatest(app.selectedSubsystemId)}
            title="Fetch the latest refs for the cached checkouts"
          >
            {app.pullingCheckouts ? 'Pulling…' : 'Pull latest'}
          </Button>
          <Button
            variant="ghost"
            size="sm"
            icon={RefreshCw}
            onClick={app.bumpDataVersion}
            title="Refetch registry, commits, file drift, and history"
          >
            Refresh
          </Button>
        </div>
      </header>

      <div className="flex min-h-0 flex-1 flex-col">
        {app.subsystems.loading && subsystems.length === 0 ? (
          <div className="flex min-h-0 flex-1 items-center justify-center p-6">
            <LoadingState label="Loading the subsystem registry…" />
          </div>
        ) : subsystems.length === 0 ? (
          <div className="flex min-h-0 flex-1 items-center justify-center p-6">
            <EmptyState
              icon={GitBranch}
              title="No registered subsystem"
              message="Register a subsystem in Settings → Registry, then refresh to browse and promote its changes."
            />
          </div>
        ) : view === 'commits' ? (
          <CommitWorkspace />
        ) : (
          <SubsystemDiff embedded />
        )}
      </div>
    </div>
  );
}
