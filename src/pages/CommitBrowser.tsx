import { GitBranch, Info, RefreshCw, ShieldAlert } from 'lucide-react';

import { useApp } from '../app/context';
import { CommitWorkspace } from '../components/CommitWorkspace';
import { SubsystemChip } from '../components/SubsystemCard';
import { Button, EmptyState, InlineAlert, LoadingState, Select } from '../components/ui/primitives';

/** Commit Browser (spec §3.3): commit-focused framing of the shared workspace. */
export function CommitBrowser() {
  const app = useApp();
  const subsystems = app.subsystems.data ?? [];
  const branches = (app.targetBranches.data?.branches ?? []).filter((branch) => branch.allowed);

  return (
    <div className="flex h-full min-h-0 flex-col">
      <header className="flex flex-wrap items-start justify-between gap-3 px-5 pb-3 pt-4">
        <div className="min-w-0">
          <h1 className="flex items-center gap-2 text-xl font-semibold tracking-tight text-white">
            <GitBranch className="h-5 w-5 text-brand-400" aria-hidden="true" />
            Commit Browser
          </h1>
          <p className="mt-0.5 text-xs text-slate-400">
            Browse a subsystem source branch and select exact commit SHAs for an integration PR
          </p>
        </div>

        <div className="flex flex-wrap items-center gap-2">
          <span className="flex items-center gap-2">
            <span className="text-2xs uppercase tracking-wide text-slate-500">
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

      <div className="px-3">
        <InlineAlert tone="info" title="Exact SHAs only">
          <span className="flex flex-wrap items-center gap-x-4 gap-y-1">
            <span className="inline-flex items-center gap-1.5">
              <ShieldAlert className="h-3.5 w-3.5" aria-hidden="true" />
              Abbreviated SHAs are display-only; the workflow receives full 40-character SHAs.
            </span>
            <span className="inline-flex items-center gap-1.5">
              <Info className="h-3.5 w-3.5" aria-hidden="true" />
              Only the first-parent delta of the ticked commits is applied — newer and unselected
              commits are never imported.
            </span>
          </span>
        </InlineAlert>
      </div>

      {subsystems.length > 0 ? (
        <div className="scroll-area flex shrink-0 gap-2 overflow-x-auto px-3 py-3">
          {subsystems.map((subsystem) => (
            <SubsystemChip
              key={subsystem.id}
              subsystem={subsystem}
              selected={subsystem.id === app.selectedSubsystemId}
              onSelect={() => app.selectSubsystem(subsystem.id)}
            />
          ))}
        </div>
      ) : null}

      <div className="flex min-h-0 flex-1 gap-3 px-3 pb-3">
        {app.subsystems.loading && subsystems.length === 0 ? (
          <LoadingState label="Loading the subsystem registry…" />
        ) : subsystems.length === 0 ? (
          <EmptyState
            icon={GitBranch}
            title="No registered subsystem"
            message="Register a subsystem through a registration PR, then refresh to browse its commit history."
          />
        ) : (
          <CommitWorkspace />
        )}
      </div>
    </div>
  );
}
