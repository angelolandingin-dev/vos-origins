import { useState } from 'react';
import { Boxes, Plus, Search } from 'lucide-react';

import { api, asApiError, type ApiError } from '../api/invoke';
import type {
  RegistrationResult,
  SubsystemRegistrationRequest,
  SubsystemPolicy,
} from '../api/types';
import { useApp } from '../app/context';
import { CommitWorkspace } from '../components/CommitWorkspace';
import { SubsystemCard, SubsystemChip, SubsystemWarning } from '../components/SubsystemCard';
import { cx } from '../components/ui/cx';
import { useMediaQuery } from '../components/ui/useMediaQuery';
import {
  Button,
  EmptyState,
  ErrorState,
  Field,
  InlineAlert,
  LoadingState,
  Modal,
} from '../components/ui/primitives';

/** Subsystems screen (spec §§3.2/3.3). Registration goes through a registration PR, never a direct registry write. */

const DEFAULT_POLICY: SubsystemPolicy = {
  applicationFiles: 'selected-commit-delta',
  packageJson: 'merge-dependency-delta',
  lockfiles: 'regenerate-in-target',
  sharedConfiguration: 'review-explicitly',
  githubWorkflows: 'main-owned',
  secretFiles: 'never-import',
};

function RegistrationDialog({
  open,
  onClose,
  onRegistered,
}: {
  open: boolean;
  onClose: () => void;
  onRegistered: () => void;
}) {
  const [form, setForm] = useState<Omit<SubsystemRegistrationRequest, 'policy'>>({
    id: '',
    name: '',
    repository: 'organization/',
    sourceBranch: 'master',
    description: '',
  });
  const [submitting, setSubmitting] = useState(false);
  const [error, setError] = useState<ApiError | null>(null);
  const [result, setResult] = useState<RegistrationResult | null>(null);

  const update = (key: keyof typeof form, value: string) =>
    setForm((current) => ({ ...current, [key]: value }));

  const submit = () => {
    setSubmitting(true);
    setError(null);
    void api
      .submitSubsystemRegistration({ ...form, policy: DEFAULT_POLICY })
      .then((response) => {
        setResult(response);
        onRegistered();
      })
      .catch((caught: unknown) => setError(asApiError(caught)))
      .finally(() => setSubmitting(false));
  };

  return (
    <Modal
      open={open}
      onClose={onClose}
      title="Register Subsystem"
      description="Collects the registry entry that a reviewed registration PR adds to .integration/subsystems/<id>.json on the registry branch."
      footer={
        result ? (
          <Button variant="primary" onClick={onClose}>
            Close
          </Button>
        ) : (
          <>
            <Button variant="secondary" onClick={onClose} disabled={submitting}>
              Cancel
            </Button>
            <Button
              variant="primary"
              icon={Plus}
              loading={submitting}
              onClick={submit}
              disabled={!form.id || !form.name}
            >
              {submitting ? 'Validating…' : 'Prepare registration PR'}
            </Button>
          </>
        )
      }
    >
      {result ? (
        <div className="space-y-3">
          <InlineAlert
            tone="success"
            title={`Registration PR #${result.pullRequestNumber} prepared`}
          >
            <p>{result.message}</p>
            {result.branch ? <p className="mono mt-1 text-2xs">{result.branch}</p> : null}
          </InlineAlert>
          <ul className="space-y-2">
            {result.findings.map((finding) => (
              <li key={finding.code}>
                <InlineAlert
                  tone={
                    finding.level === 'error'
                      ? 'danger'
                      : finding.level === 'warning'
                        ? 'warning'
                        : 'info'
                  }
                  title={finding.message}
                >
                  {finding.remedy ? <p>{finding.remedy}</p> : null}
                </InlineAlert>
              </li>
            ))}
          </ul>
        </div>
      ) : (
        <div className="space-y-4">
          {error ? (
            <InlineAlert tone="danger" title={error.message}>
              {error.remedy ? <p>{error.remedy}</p> : null}
              {error.details ? (
                <pre className="mono mt-2 max-h-40 overflow-auto whitespace-pre-wrap rounded-md border border-rose-900/60 bg-rose-950/30 p-2 text-2xs">
                  {error.details}
                </pre>
              ) : null}
            </InlineAlert>
          ) : null}

          <div className="grid gap-4 sm:grid-cols-2">
            <Field
              label="Subsystem id"
              required
              hint="Lowercase slug. Becomes the registry file name and the stable identity in commit trailers."
            >
              <input
                className="input mono"
                value={form.id}
                onChange={(event) => update('id', event.target.value)}
                placeholder="hr-management"
                aria-label="Subsystem id"
                data-autofocus
              />
            </Field>
            <Field label="Display name" required>
              <input
                className="input"
                value={form.name}
                onChange={(event) => update('name', event.target.value)}
                placeholder="HR Management"
                aria-label="Display name"
              />
            </Field>
            <Field
              label="GitHub repository"
              required
              hint="OWNER/REPO. The registry is an allow-list; arbitrary remotes are refused."
            >
              <input
                className="input mono"
                value={form.repository}
                onChange={(event) => update('repository', event.target.value)}
                placeholder="organization/hr-management"
                aria-label="GitHub repository"
              />
            </Field>
            <Field
              label="Protected source branch"
              required
              hint="Approved commits land here. This is the source, never the main-system target branch."
            >
              <input
                className="input mono"
                value={form.sourceBranch}
                onChange={(event) => update('sourceBranch', event.target.value)}
                placeholder="master"
                aria-label="Protected source branch"
              />
            </Field>
          </div>

          <Field label="Description">
            <textarea
              className="input h-20 resize-none"
              value={form.description}
              onChange={(event) => update('description', event.target.value)}
              placeholder="What this subsystem owns inside the main system."
              aria-label="Description"
            />
          </Field>

          <div className="rounded-lg border border-ink-700/60 bg-ink-850/50 p-3">
            <p className="text-xs font-semibold text-slate-200">Integration policy defaults</p>
            <ul className="mt-1.5 space-y-1 text-2xs text-slate-400">
              <li>Application files: {DEFAULT_POLICY.applicationFiles}</li>
              <li>package.json: {DEFAULT_POLICY.packageJson}</li>
              <li>Lockfiles: {DEFAULT_POLICY.lockfiles}</li>
              <li>Shared configuration: {DEFAULT_POLICY.sharedConfiguration}</li>
              <li>GitHub workflows: {DEFAULT_POLICY.githubWorkflows}</li>
              <li>Secret files: {DEFAULT_POLICY.secretFiles}</li>
            </ul>
            <p className="mt-2 text-2xs text-slate-500">
              Overrides belong in the reviewed registration PR. Duplicate ids or repositories are
              rejected.
            </p>
          </div>
        </div>
      )}
    </Modal>
  );
}

export function Subsystems() {
  const app = useApp();
  const [search, setSearch] = useState('');
  const [registerOpen, setRegisterOpen] = useState(false);
  const narrow = useMediaQuery('(max-width: 1023px)');

  const subsystems = app.subsystems.data ?? [];
  const needle = search.trim().toLowerCase();
  const visible = needle
    ? subsystems.filter(
        (subsystem) =>
          subsystem.name.toLowerCase().includes(needle) ||
          subsystem.repository.toLowerCase().includes(needle),
      )
    : subsystems;

  return (
    <div className="flex h-full min-h-0 flex-col">
      <header className="flex flex-wrap items-start justify-between gap-3 px-5 pb-3 pt-4">
        <div className="min-w-0">
          <h1 className="text-xl font-semibold tracking-tight text-white">Subsystems</h1>
          <p className="mt-0.5 text-xs text-slate-400">
            Manage and integrate independent subsystems
          </p>
        </div>

        <div className="flex flex-wrap items-center gap-2">
          <div className="relative min-w-[220px] flex-1">
            <Search
              className="pointer-events-none absolute left-3 top-1/2 h-4 w-4 -translate-y-1/2 text-slate-500"
              aria-hidden="true"
            />
            <input
              type="search"
              value={search}
              onChange={(event) => setSearch(event.target.value)}
              placeholder="Search subsystems…"
              aria-label="Search registered subsystems"
              className="input pl-9"
            />
          </div>
          <Button variant="primary" icon={Plus} onClick={() => setRegisterOpen(true)}>
            Register Subsystem
          </Button>
        </div>
      </header>

      <div className="flex min-h-0 flex-1 gap-3 px-3 pb-3">
        {narrow ? (
          <div className="flex min-h-0 flex-1 flex-col gap-3">
            <div className="scroll-area flex shrink-0 gap-2 overflow-x-auto pb-1">
              {visible.map((subsystem) => (
                <SubsystemChip
                  key={subsystem.id}
                  subsystem={subsystem}
                  selected={subsystem.id === app.selectedSubsystemId}
                  onSelect={() => app.selectSubsystem(subsystem.id)}
                />
              ))}
            </div>
            <CommitWorkspace />
          </div>
        ) : (
          <>
            <aside className="flex w-[300px] shrink-0 flex-col gap-2">
              <div className="scroll-area min-h-0 flex-1 space-y-2">
                {app.subsystems.loading && subsystems.length === 0 ? (
                  <LoadingState label="Loading registry…" />
                ) : app.subsystems.error ? (
                  <ErrorState
                    error={app.subsystems.error}
                    onRetry={app.refreshSubsystems}
                    compact
                  />
                ) : visible.length === 0 ? (
                  <EmptyState
                    icon={Boxes}
                    title="No subsystem matches"
                    message="Clear the search, or register a new subsystem through a registration PR."
                    action={
                      <Button
                        size="sm"
                        variant="primary"
                        icon={Plus}
                        onClick={() => setRegisterOpen(true)}
                      >
                        Register Subsystem
                      </Button>
                    }
                  />
                ) : (
                  visible.map((subsystem) => (
                    <SubsystemCard
                      key={subsystem.id}
                      subsystem={subsystem}
                      selected={subsystem.id === app.selectedSubsystemId}
                      onSelect={() => app.selectSubsystem(subsystem.id)}
                    />
                  ))
                )}
              </div>

              {app.selectedSubsystem?.connectivityMessage ? (
                <SubsystemWarning message={app.selectedSubsystem.connectivityMessage} />
              ) : null}

              <p
                className={cx(
                  'rounded-lg border border-ink-700/60 bg-ink-870 p-2.5 text-2xs leading-relaxed text-slate-500',
                )}
              >
                The registry lives in the main repository on{' '}
                <span className="mono">{app.auth.data?.registryBranch ?? 'master'}</span>. It is
                read-only here: every change goes through a reviewed registration PR.
              </p>
            </aside>

            <CommitWorkspace />
          </>
        )}
      </div>

      <RegistrationDialog
        open={registerOpen}
        onClose={() => setRegisterOpen(false)}
        onRegistered={() => {
          app.refreshSubsystems();
          app.pushNotice({
            tone: 'success',
            title: 'Registration request prepared',
            message:
              'The subsystem becomes selectable after the registration PR is merged and the registry is refreshed.',
          });
        }}
      />
    </div>
  );
}
