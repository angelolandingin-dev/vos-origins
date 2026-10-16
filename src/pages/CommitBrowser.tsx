import { GitBranch, RefreshCw } from 'lucide-react';

import { useApp } from '../app/context';
import { CommitWorkspace } from '../components/CommitWorkspace';
import { SubsystemFilterList } from '../components/SubsystemFilterList';
import { Button, EmptyState, LoadingState, Select } from '../components/ui/primitives';

/** Commit Browser (spec §3.3): commit-focused framing of the shared workspace. */
export function CommitBrowser() {
  const app = useApp();
  const subsystems = app.subsystems.data ?? [];
  const branches = (app.targetBranches.data?.branches ?? []).filter((branch) => branch.allowed);

  return (
    <div className="flex h-full min-h-0 flex-col">
      <header className="flex flex-wrap items-start justify-between gap-3 px-5 pt-4 pb-3">
        <div className="min-w-0">
          <h1 className="text-[14px] font-semibold text-fg">Commit Browser</h1>
        </div>

        <div className="flex flex-wrap items-center gap-2">
          <span className="flex items-center gap-2">
            <span className="text-[11px] tracking-wide text-muted uppercase">
              Status for target
            </span>
            <Select
              aria-label="Main-system target branch used for status computation"
              className="w-[190px]"
              value={app.targetBranch}
              onChange={(event) => app.setTargetBranch(event.target.value)}
              options={branches.map((branch) => ({ value: branch.name, label: branch.name }))}
            />
          </span>
          <Button
            variant="secondary"
            icon={RefreshCw}
            onClick={app.bumpDataVersion}
            title="Refetch registry, commits, runs and history"
          >
            Refresh
          </Button>
        </div>
      </header>

      <div className="flex min-h-0 flex-1 flex-col gap-2.5 px-3 pb-3 lg:flex-row">
        {subsystems.length > 0 ? (
          <aside
            aria-label="Subsystem list"
            className="flex shrink-0 flex-col overflow-hidden rounded-md border border-line bg-surface lg:w-[260px]"
          >
            <div className="max-h-[220px] min-h-0 flex-1 lg:max-h-none">
              <SubsystemFilterList
                subsystems={subsystems}
                selectedId={app.selectedSubsystemId}
                onSelect={(id) => app.selectSubsystem(id)}
              />
            </div>
          </aside>
        ) : null}

        <div className="flex min-h-0 flex-1">
          {app.subsystems.loading && subsystems.length === 0 ? (
            <LoadingState label="Loading the subsystem registry…" />
          ) : subsystems.length === 0 ? (
            <EmptyState
              icon={GitBranch}
              title="No registered subsystem"
              message="Register a subsystem on the Subsystems screen, then refresh to browse its commit history."
            />
          ) : (
            <CommitWorkspace />
          )}
        </div>
      </div>
    </div>
  );
}
