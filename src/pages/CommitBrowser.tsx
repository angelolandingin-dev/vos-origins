import { useState } from 'react';
import { GitBranch, RefreshCw } from 'lucide-react';

import { api, asApiError } from '../api/invoke';
import { useApp } from '../app/context';
import { CommitWorkspace } from '../components/CommitWorkspace';
import { SubsystemSelector } from '../components/SubsystemSelector';
import { Button, EmptyState, LoadingState, Select } from '../components/ui/primitives';

/** Commit Browser (spec §3.3): commit-focused framing of the shared workspace. */
export function CommitBrowser() {
  const app = useApp();
  const [pulling, setPulling] = useState(false);
  const subsystems = app.subsystems.data ?? [];
  const branches = (app.targetBranches.data?.branches ?? []).filter((branch) => branch.allowed);

  const pullLatest = () => {
    setPulling(true);
    void api
      .refreshCheckouts(app.selectedSubsystemId)
      .then((result) => {
        app.pushNotice({
          tone: result.failed.length > 0 ? 'warning' : 'success',
          title: 'Repositories refreshed',
          message: result.message,
        });
        app.bumpDataVersion();
      })
      .catch((error: unknown) => {
        app.pushNotice({
          tone: 'danger',
          title: 'Refresh failed',
          message: asApiError(error).message,
        });
      })
      .finally(() => setPulling(false));
  };

  return (
    <div className="flex h-full min-h-0 flex-col">
      <header className="flex flex-wrap items-start justify-between gap-3 px-5 pt-4 pb-3">
        <div className="min-w-0">
          <h1 className="text-[14px] font-semibold text-fg">Commit Browser</h1>
        </div>

        <div className="flex flex-wrap items-center gap-2">
          {subsystems.length > 0 ? (
            <SubsystemSelector
              subsystems={subsystems}
              selectedId={app.selectedSubsystemId}
              onSelect={(id) => app.selectSubsystem(id)}
            />
          ) : null}
          <span className="flex items-center gap-2">
            <span className="text-[11px] tracking-wide text-muted uppercase">
              Status for target
            </span>
            <Select
              ariaLabel="Main-system target branch used for status computation"
              className="w-[190px]"
              value={app.targetBranch}
              onValueChange={(next) => app.setTargetBranch(next)}
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
          <Button
            variant="secondary"
            icon={RefreshCw}
            disabled={pulling}
            onClick={pullLatest}
            title="Fetch the cached checkouts"
          >
            {pulling ? 'Pulling…' : 'Pull latest'}
          </Button>
        </div>
      </header>

      <div className="flex min-h-0 flex-1 flex-col gap-2.5 px-3 pb-3">
        <div className="flex min-h-0 flex-1">
          {app.subsystems.loading && subsystems.length === 0 ? (
            <LoadingState label="Loading the subsystem registry…" />
          ) : subsystems.length === 0 ? (
            <EmptyState
              icon={GitBranch}
              title="No registered subsystem"
              message="Register a subsystem in Settings → Main system registry, then refresh to browse its commit history."
            />
          ) : (
            <CommitWorkspace />
          )}
        </div>
      </div>
    </div>
  );
}
