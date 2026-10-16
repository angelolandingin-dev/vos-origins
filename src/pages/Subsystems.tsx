import { useState } from 'react';
import { Boxes, Plus, RefreshCw, Search } from 'lucide-react';

import { api, asApiError, type ApiError } from '../api/invoke';
import type {
  RegistrationResult,
  SubsystemRegistrationRequest,
  SubsystemPolicy,
} from '../api/types';
import { useApp } from '../app/context';
import { CommitWorkspace } from '../components/CommitWorkspace';
import {
  SubsystemCard,
  SubsystemSegmentedControl,
  SubsystemWarning,
} from '../components/SubsystemCard';
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

/** Subsystems screen (spec §§3.2/3.3). The registry lives in the tool; a promotion is local-first. */

const DEFAULT_POLICY: SubsystemPolicy = {
  applicationFiles: 'selected-commit-delta',
  packageJson: 'merge-dependency-delta',
  lockfiles: 'regenerate-in-target',
  sharedConfiguration: 'review-explicitly',
  githubWorkflows: 'main-owned',
  secretFiles: 'never-import',
};

/** User-facing wording for the policy defaults below; the wire values above stay byte-identical. */
const POLICY_DISPLAY: Record<string, string> = {
  'selected-commit-delta': 'copies the selected files',
  'merge-dependency-delta': 'merges the manifest changes',
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
      description="Adds an entry to the tool registry (Settings → Main system registry)."
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
              {submitting ? 'Validating…' : 'Register subsystem'}
            </Button>
          </>
        )
      }
    >
      {result ? (
        <div className="space-y-3">
          <InlineAlert tone="success" title={`Registration #${result.pullRequestNumber} prepared`}>
            <p>{result.message}</p>
            {result.branch ? <p className="mono mt-1 text-[11px]">{result.branch}</p> : null}
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
                <pre className="mono mt-2 max-h-40 overflow-auto rounded-md border border-status-blocked/40 bg-status-blocked-soft p-2 text-[11px] whitespace-pre-wrap">
                  {error.details}
                </pre>
              ) : null}
            </InlineAlert>
          ) : null}

          <div className="grid gap-4 sm:grid-cols-2">
            <Field label="Subsystem id" required hint="Lowercase slug; used in commit trailers.">
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

          <div className="rounded-md border border-line bg-raised p-3">
            <p className="text-xs font-semibold text-fg">Integration policy defaults</p>
            <ul className="mt-1.5 space-y-1 text-[11px] text-muted">
              <li>
                Application files:{' '}
                {POLICY_DISPLAY[DEFAULT_POLICY.applicationFiles] ?? DEFAULT_POLICY.applicationFiles}
              </li>
              <li>
                package.json:{' '}
                {POLICY_DISPLAY[DEFAULT_POLICY.packageJson] ?? DEFAULT_POLICY.packageJson}
              </li>
              <li>Lockfiles: {DEFAULT_POLICY.lockfiles}</li>
              <li>Shared configuration: {DEFAULT_POLICY.sharedConfiguration}</li>
              <li>GitHub workflows: {DEFAULT_POLICY.githubWorkflows}</li>
              <li>Secret files: {DEFAULT_POLICY.secretFiles}</li>
            </ul>
            <p className="mt-2 text-[11px] text-disabled">
              Duplicate ids or repositories are rejected.
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
  const [pulling, setPulling] = useState(false);
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
      <header className="flex flex-wrap items-start justify-between gap-3 px-5 pt-4 pb-3">
        <div className="min-w-0">
          <h1 className="text-[14px] font-semibold text-fg">Subsystems</h1>
        </div>

        <div className="flex flex-wrap items-center gap-2">
          <div className="relative min-w-[220px] flex-1">
            <Search
              className="pointer-events-none absolute top-1/2 left-3 h-4 w-4 -translate-y-1/2 text-disabled"
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
          <Button
            variant="secondary"
            icon={RefreshCw}
            disabled={pulling}
            onClick={() => {
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
            }}
          >
            {pulling ? 'Pulling…' : 'Pull latest'}
          </Button>
          <Button variant="primary" icon={Plus} onClick={() => setRegisterOpen(true)}>
            Register Subsystem
          </Button>
        </div>
      </header>

      <div className="flex min-h-0 flex-1 gap-2.5 px-3 pb-3">
        {narrow ? (
          <div className="flex min-h-0 flex-1 flex-col gap-2.5">
            <div className="scroll-area flex shrink-0 overflow-x-auto pb-1">
              <SubsystemSegmentedControl
                subsystems={visible}
                selectedId={app.selectedSubsystemId}
                onSelect={(id) => app.selectSubsystem(id)}
              />
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
                    message="Clear the search, or register a new subsystem."
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

              <p className="rounded-md border border-line bg-surface p-2.5 text-[11px] leading-relaxed text-muted">
                The subsystem registry lives in the tool — manage it under{' '}
                <span className="mono">Settings → Main system registry</span>.
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
            message: 'The subsystem becomes selectable after the registry is refreshed.',
          });
        }}
      />
    </div>
  );
}
