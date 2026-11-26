import { ChevronRight } from 'lucide-react';

import type { PromotionProgress } from '../api/types';
import type { ExecutionStage } from './executionProgress';
import { cx } from './ui/cx';

export function ExecutionProgressView({
  title = 'Progress',
  stages,
  steps,
  outcome,
}: {
  title?: string;
  stages: ReadonlyArray<ExecutionStage>;
  steps: PromotionProgress[];
  outcome: 'running' | 'success' | 'failed';
}) {
  const byStage = new Map(steps.map((step) => [step.stage, step]));

  let failedId: string | null = null;
  if (outcome === 'failed' && steps.length > 0) {
    const last = steps[steps.length - 1]!;
    failedId =
      last.stage === 'cleanup' && steps.length > 1 ? steps[steps.length - 2]!.stage : last.stage;
  }

  let currentIndex = -1;
  stages.forEach((stage, index) => {
    if (byStage.has(stage.id)) currentIndex = index;
  });
  if (failedId !== null) {
    const failedIndex = stages.findIndex((stage) => stage.id === failedId);
    if (failedIndex >= 0) currentIndex = failedIndex;
  }
  if (outcome === 'success') currentIndex = stages.length;

  const current =
    currentIndex >= 0 && currentIndex < stages.length
      ? byStage.get(stages[currentIndex]!.id)
      : undefined;
  const currentFraction =
    current?.index != null && current?.total != null && current.total > 0
      ? Math.min(current.index / current.total, 1)
      : 0;
  const done = currentIndex < 0 ? 0 : currentIndex + currentFraction;
  const percent = stages.length > 0 ? Math.min(100, Math.round((done / stages.length) * 100)) : 0;
  const detail =
    outcome === 'success'
      ? 'Completed.'
      : (current?.detail ?? current?.label ?? steps[steps.length - 1]?.label ?? 'Starting…');
  const counts =
    current?.index != null && current?.total != null ? `${current.index} / ${current.total}` : null;

  return (
    <section
      aria-label={title}
      role="status"
      aria-live="polite"
      className={cx(
        'space-y-2 rounded-md border border-line p-3',
        outcome === 'success'
          ? 'bg-status-applied-soft'
          : outcome === 'failed'
            ? 'bg-status-blocked-soft'
            : 'bg-surface',
      )}
    >
      <div className="flex h-[24px] items-center justify-between gap-2">
        <h3 className="text-[11px] font-semibold tracking-wide text-muted uppercase">{title}</h3>
        {counts ? (
          <span className="mono text-right text-[11px] text-muted tabular-nums">{counts}</span>
        ) : null}
      </div>
      <div className="h-[3px] overflow-hidden rounded-full bg-line" aria-hidden="true">
        <div
          className={cx(
            'h-full rounded-full transition-[width]',
            outcome === 'failed'
              ? 'bg-status-blocked'
              : outcome === 'success'
                ? 'bg-status-applied'
                : 'bg-accent',
          )}
          style={{ width: `${Math.max(outcome === 'running' ? 8 : 0, percent)}%` }}
        />
      </div>
      <ul className="divide-y divide-line overflow-hidden rounded-md border border-line bg-surface">
        {stages.map((stage, index) => {
          const seen = byStage.get(stage.id);
          const isFailed = stage.id === failedId;
          const state =
            isFailed || index === currentIndex ? 'current' : index < currentIndex ? 'done' : 'todo';
          return (
            <li
              key={stage.id}
              className={cx(
                'flex h-[22px] items-center gap-2 px-2.5 text-[11px]',
                isFailed
                  ? 'bg-status-blocked-soft'
                  : state === 'done'
                    ? 'bg-status-applied-soft/60'
                    : '',
              )}
            >
              <span
                aria-hidden="true"
                className={cx(
                  'h-1.5 w-1.5 shrink-0 rounded-full',
                  isFailed
                    ? 'bg-status-blocked'
                    : state === 'done'
                      ? 'bg-status-applied'
                      : state === 'current'
                        ? 'animate-pulse bg-accent'
                        : 'bg-line',
                )}
              />
              <span
                className={cx(
                  'min-w-0 flex-1 truncate',
                  isFailed
                    ? 'font-medium text-status-blocked-ink'
                    : state === 'todo'
                      ? 'text-muted'
                      : 'text-fg',
                )}
              >
                {seen?.label ?? stage.label}
              </span>
              {seen?.index != null && seen.total != null ? (
                <span className="mono shrink-0 text-[10px] text-muted tabular-nums">
                  {seen.index} / {seen.total}
                </span>
              ) : null}
            </li>
          );
        })}
      </ul>
      <p className="truncate text-[11px] text-muted">{detail}</p>
    </section>
  );
}

export function ExecutionLogDisclosure({
  title = 'Execution log',
  stages,
  steps,
  outcome,
}: {
  title?: string;
  stages: ReadonlyArray<ExecutionStage>;
  steps: PromotionProgress[];
  outcome: 'success' | 'failed';
}) {
  if (steps.length === 0) return null;
  const completedCount = outcome === 'success' ? stages.length : steps.length;
  return (
    <details className="group overflow-hidden rounded-md border border-line bg-surface">
      <summary className="flex h-[32px] cursor-pointer items-center justify-between gap-2 px-3 text-[11px] text-muted select-none hover:bg-hover hover:text-fg">
        <span className="flex items-center gap-1.5 font-medium">
          <ChevronRight
            className="h-3.5 w-3.5 text-disabled transition-transform group-open:rotate-90"
            aria-hidden="true"
          />
          {title}
        </span>
        <span
          className={cx(
            'mono flex items-center gap-1.5 tabular-nums',
            outcome === 'success' ? 'text-status-applied-ink' : 'text-status-blocked-ink',
          )}
        >
          <span
            className={cx(
              'h-1.5 w-1.5 rounded-full',
              outcome === 'success' ? 'bg-status-applied' : 'bg-status-blocked',
            )}
            aria-hidden="true"
          />
          {completedCount} / {stages.length} steps
        </span>
      </summary>
      <div className="border-t border-line p-2.5">
        <ExecutionProgressView title={title} stages={stages} steps={steps} outcome={outcome} />
      </div>
    </details>
  );
}
