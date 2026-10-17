import { useMemo, type ReactNode } from 'react';
import {
  ArrowDownUp,
  ArrowUpDown,
  ChevronLeft,
  ChevronRight,
  GitPullRequestArrow,
  PanelRight,
  PanelRightClose,
  Search,
  SquareCheck,
  X,
} from 'lucide-react';

import type { ApiError } from '../api/invoke';
import type { CommitPage, CommitStatusKind, CommitSummary } from '../api/types';
import { CommitStatusBadge, StatusBadge } from './StatusBadge';
import { statusAppearance } from './statusAppearance';
import { cx } from './ui/cx';
import { formatDate } from './ui/format';
import {
  Button,
  Checkbox,
  EmptyState,
  ErrorState,
  IconButton,
  LoadingState,
  Select,
  Tooltip,
} from './ui/primitives';

const STATUS_FILTER_OPTIONS: Array<{ value: CommitStatusKind | 'all'; label: string }> = [
  { value: 'all', label: 'All statuses' },
  { value: 'eligible', label: 'Eligible' },
  { value: 'promotion_queued', label: 'Promotion queued' },
  { value: 'promotion_running', label: 'Promotion running' },
  { value: 'pr_open', label: 'PR open' },
  { value: 'in_review', label: 'In review' },
  { value: 'integrated', label: 'Integrated' },
  { value: 'applied', label: 'Already applied' },
  { value: 'divergent', label: 'Divergent' },
  { value: 'superseded', label: 'Superseded' },
  { value: 'reverted', label: 'Reverted' },
  { value: 'blocked', label: 'Blocked' },
  { value: 'failed', label: 'Failed' },
  { value: 'unknown', label: 'Unknown' },
];

export interface CommitTableProps {
  page: CommitPage | null;
  loading: boolean;
  error: ApiError | null;
  onRetry: () => void;
  targetBranch: string;
  search: string;
  onSearchChange: (value: string) => void;
  statusFilter: CommitStatusKind | 'all';
  onStatusFilterChange: (value: CommitStatusKind | 'all') => void;
  sort: 'newest' | 'oldest';
  onSortChange: (value: 'newest' | 'oldest') => void;
  pageNumber: number;
  onPageChange: (page: number) => void;
  pageSize: number;
  onPageSizeChange: (size: number) => void;
  selectedShas: string[];
  onToggleSha: (sha: string) => void;
  onSelectShas: (shas: string[]) => void;
  focusedSha: string | null;
  onFocusSha: (sha: string) => void;
  onPromoteSelected: () => void;
  onClearSelection: () => void;
  detailsVisible: boolean;
  onToggleDetails: () => void;
  toolbarExtras?: ReactNode;
  emptyMessage?: string;
}

/** Timeline dot: status colour lives on this 10px dot only — never on the row. */
function dotClassFor(commit: CommitSummary): string {
  const kind = commit.status.kind;
  switch (kind) {
    case 'integrated':
      return 'bg-status-integrated ring-status-integrated/30';
    case 'promotion_running':
    case 'promotion_queued':
      return 'bg-status-unknown ring-status-unknown/30';
    case 'pr_open':
      return 'bg-status-applied ring-status-applied/30';
    case 'in_review':
      return 'bg-status-eligible ring-status-eligible/30';
    case 'applied':
      return 'bg-status-applied ring-status-applied/30';
    case 'divergent':
      return 'bg-status-divergent ring-status-divergent/30';
    case 'superseded':
    case 'reverted':
    case 'unknown':
      return 'bg-status-superseded ring-status-superseded/30';
    case 'blocked':
    case 'failed':
      return 'bg-status-blocked ring-status-blocked/30';
    default:
      return 'bg-status-eligible ring-status-eligible/30';
  }
}

export function CommitTable({
  page,
  loading,
  error,
  onRetry,
  targetBranch,
  search,
  onSearchChange,
  statusFilter,
  onStatusFilterChange,
  sort,
  onSortChange,
  pageNumber,
  onPageChange,
  pageSize,
  onPageSizeChange,
  selectedShas,
  onToggleSha,
  onSelectShas,
  focusedSha,
  onFocusSha,
  onPromoteSelected,
  onClearSelection,
  detailsVisible,
  onToggleDetails,
  toolbarExtras,
  emptyMessage,
}: CommitTableProps) {
  const commits = useMemo(() => page?.commits ?? [], [page]);
  const total = page?.total ?? 0;
  const totalPages = Math.max(1, Math.ceil(total / Math.max(pageSize, 1)));
  const pageShas = useMemo(() => commits.map((commit) => commit.sha), [commits]);
  const allPageSelected =
    pageShas.length > 0 && pageShas.every((sha) => selectedShas.includes(sha));
  const somePageSelected = pageShas.some((sha) => selectedShas.includes(sha));

  const rangeStart = total === 0 ? 0 : (pageNumber - 1) * pageSize + 1;
  const rangeEnd = Math.min(pageNumber * pageSize, total);

  return (
    <div className="flex min-h-0 flex-1 flex-col">
      <div className="flex flex-wrap items-center gap-2 border-b border-line px-2.5 py-2">
        <div className="relative min-w-[180px] flex-1">
          <Search
            className="pointer-events-none absolute top-1/2 left-3 h-4 w-4 -translate-y-1/2 text-disabled"
            aria-hidden="true"
          />
          <input
            type="search"
            value={search}
            onChange={(event) => onSearchChange(event.target.value)}
            placeholder="Search commits…"
            aria-label="Search commits by message, SHA, author or tag"
            className="input pl-9"
          />
        </div>

        <Select
          ariaLabel="Filter commits by integration status"
          className="w-[190px]"
          value={statusFilter}
          onValueChange={(next) => onStatusFilterChange(next as CommitStatusKind | 'all')}
          options={STATUS_FILTER_OPTIONS.map((option) => ({
            value: option.value,
            label: option.label,
          }))}
        />

        <Button
          size="sm"
          variant="secondary"
          icon={sort === 'newest' ? ArrowDownUp : ArrowUpDown}
          onClick={() => onSortChange(sort === 'newest' ? 'oldest' : 'newest')}
          title={
            sort === 'newest'
              ? 'Sorted newest first — click for oldest first'
              : 'Sorted oldest first — click for newest first'
          }
        >
          {sort === 'newest' ? 'Newest' : 'Oldest'}
        </Button>

        {toolbarExtras}

        <div className="ml-auto flex items-center gap-2">
          {selectedShas.length > 0 ? (
            <>
              <span className="meta text-accent-ink tabular-nums">
                <SquareCheck className="h-2.5 w-2.5" aria-hidden="true" />
                {selectedShas.length} selected
              </span>
              <IconButton icon={X} label="Clear commit selection" onClick={onClearSelection} />
            </>
          ) : null}
          <Button
            size="sm"
            variant="primary"
            icon={GitPullRequestArrow}
            disabled={selectedShas.length === 0}
            onClick={onPromoteSelected}
            title={
              selectedShas.length === 0
                ? 'Tick one or more commits to promote'
                : `Promote ${selectedShas.length} selected commit(s) to a main-system branch`
            }
          >
            Promote Selected
          </Button>
          <IconButton
            icon={detailsVisible ? PanelRightClose : PanelRight}
            label={detailsVisible ? 'Hide commit details panel' : 'Show commit details panel'}
            onClick={onToggleDetails}
          />
        </div>
      </div>

      <div className="scroll-area min-h-0 flex-1">
        {loading && commits.length === 0 ? (
          <LoadingState label="Loading commits…" />
        ) : error ? (
          <ErrorState error={error} onRetry={onRetry} />
        ) : commits.length === 0 ? (
          <EmptyState
            icon={Search}
            title="No commits match the current filters"
            message={
              search || statusFilter !== 'all'
                ? 'Clear the search or the status filter to see the full history of the source branch.'
                : (emptyMessage ??
                  'No commits were returned for this source branch. Check the subsystem registry entry and the source branch name.')
            }
            action={
              search || statusFilter !== 'all' ? (
                <Button
                  size="sm"
                  onClick={() => {
                    onSearchChange('');
                    onStatusFilterChange('all');
                  }}
                >
                  Clear filters
                </Button>
              ) : undefined
            }
          />
        ) : (
          <table className="w-full border-separate border-spacing-0">
            <caption className="sr-only">
              Commits on the selected source branch with their integration status for target branch{' '}
              {targetBranch}
            </caption>
            <thead className="sticky top-0 z-10">
              <tr className="table-head">
                <th scope="col" className="w-10 py-2 pl-2.5 text-left">
                  <Checkbox
                    checked={allPageSelected}
                    inputRef={(element) => {
                      if (element) element.indeterminate = !allPageSelected && somePageSelected;
                    }}
                    onChange={(event) => {
                      if (event.target.checked) {
                        onSelectShas(Array.from(new Set([...selectedShas, ...pageShas])));
                      } else {
                        onSelectShas(selectedShas.filter((sha) => !pageShas.includes(sha)));
                      }
                    }}
                    label="Select all commits on this page"
                  />
                </th>
                <th scope="col" className="w-6 py-2" />
                <th scope="col" className="w-24 py-2 pr-2.5 text-right">
                  SHA
                </th>
                <th scope="col" className="py-2 pr-2.5 text-left">
                  Message
                </th>
                <th scope="col" className="hidden w-40 py-2 pr-2.5 text-left xl:table-cell">
                  Author
                </th>
                <th scope="col" className="hidden w-28 py-2 pr-2.5 text-right xl:table-cell">
                  Authored
                </th>
                <th scope="col" className="w-44 py-2 pr-2.5 text-right">
                  Status
                </th>
              </tr>
            </thead>
            <tbody>
              {commits.map((commit) => {
                const selected = selectedShas.includes(commit.sha);
                const focused = focusedSha === commit.sha;
                return (
                  <tr
                    key={commit.sha}
                    className={cx(
                      'row-hover h-[50px] border-b border-line',
                      (focused || selected) && 'bg-selection',
                    )}
                  >
                    <td className="py-2 pl-2.5 align-top">
                      <Checkbox
                        checked={selected}
                        onChange={() => {
                          onToggleSha(commit.sha);
                        }}
                        label={`Select commit ${commit.shortSha} – ${commit.subject}`}
                      />
                    </td>
                    <td className="relative w-6 align-top">
                      <span
                        aria-hidden="true"
                        className="absolute top-0 bottom-0 left-1/2 w-px -translate-x-1/2 bg-line"
                      />
                      <span
                        aria-hidden="true"
                        className={cx(
                          'absolute top-4 left-1/2 h-2.5 w-2.5 -translate-x-1/2 rounded-full ring-2',
                          dotClassFor(commit),
                        )}
                      />
                    </td>
                    <td className="py-2 pr-2.5 align-top">
                      <Tooltip
                        content={`${commit.sha} — ${statusAppearance(commit.status.kind).meaning}`}
                        side="top"
                        align="end"
                      >
                        <button
                          type="button"
                          onClick={() => onFocusSha(commit.sha)}
                          className="mono block w-full rounded-[3px] text-right text-accent-ink hover:underline"
                        >
                          {commit.shortSha}
                        </button>
                      </Tooltip>
                    </td>
                    <td className="max-w-[52ch] py-2 pr-2.5 align-top">
                      <Tooltip content={commit.subject} side="top" align="start">
                        <button
                          type="button"
                          onClick={() => onFocusSha(commit.sha)}
                          className="block w-full truncate text-left text-xs text-fg hover:text-fg"
                        >
                          {commit.subject}
                        </button>
                      </Tooltip>
                      <p className="mt-0.5 truncate text-[11px] text-muted">
                        {commit.authorLogin ?? commit.authorName} · {commit.relativeTime}
                        {commit.isMerge ? ' · merge commit' : ''}
                        {commit.touchesManifest ? ' · manifest change' : ''}
                      </p>
                    </td>
                    <td className="hidden max-w-40 py-2 pr-2.5 align-top text-xs text-muted xl:table-cell">
                      <span className="block truncate">{commit.authorName}</span>
                    </td>
                    <td className="mono hidden py-2 pr-2.5 text-right text-muted tabular-nums xl:table-cell">
                      {formatDate(commit.authoredAt)}
                    </td>
                    <td className="py-2 pr-2.5 align-top">
                      <div className="flex items-center justify-end gap-1.5">
                        {selected ? <StatusBadge kind="selected" /> : null}
                        <CommitStatusBadge status={commit.status} />
                      </div>
                    </td>
                  </tr>
                );
              })}
            </tbody>
          </table>
        )}
      </div>

      <div className="flex flex-wrap items-center justify-between gap-3 border-t border-line px-2.5 py-2">
        <p className="truncate text-[11px] text-muted">
          {total === 0
            ? 'No commits'
            : `Showing ${rangeStart}–${rangeEnd} of ${total} on ${page?.sourceBranch ?? 'source'}`}
          <span className="ml-1 text-disabled">· status for target {targetBranch}</span>
        </p>
        <div className="flex items-center gap-2">
          <label className="flex items-center gap-1.5 text-[11px] text-muted">
            Rows
            <Select
              className="w-[76px]"
              value={String(pageSize)}
              onValueChange={(next) => onPageSizeChange(Number(next))}
              ariaLabel="Rows per page"
              options={[10, 25, 50, 100].map((size) => ({
                value: String(size),
                label: String(size),
              }))}
            />
          </label>
          <IconButton
            icon={ChevronLeft}
            label="Previous page"
            disabled={pageNumber <= 1}
            onClick={() => onPageChange(Math.max(1, pageNumber - 1))}
          />
          <span className="mono text-right text-muted tabular-nums">
            {pageNumber} / {totalPages}
          </span>
          <IconButton
            icon={ChevronRight}
            label="Next page"
            disabled={pageNumber >= totalPages}
            onClick={() => onPageChange(Math.min(totalPages, pageNumber + 1))}
          />
        </div>
      </div>
    </div>
  );
}
