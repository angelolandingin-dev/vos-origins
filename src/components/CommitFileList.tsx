import type { PromotionPreflight } from '../api/types';
import { PathLabel } from './PathLabel';
import { Checkbox } from './ui/primitives';

export interface CommitFileListProps {
  preflight: PromotionPreflight | null;
  preflightLoading: boolean;
  excludedPaths: string[];
  onAddExcludedPath: (path: string) => void;
  onRemoveExcludedPath: (path: string) => void;
}

function isLockfile(path: string): boolean {
  const lower = path.split('/').pop()?.toLowerCase() ?? '';
  return (
    lower === 'package-lock.json' ||
    lower === 'npm-shrinkwrap.json' ||
    lower === 'yarn.lock' ||
    lower === 'pnpm-lock.yaml' ||
    lower === 'bun.lockb' ||
    lower === 'bun.lock'
  );
}

function changeKind(
  path: string,
  outcome: string,
  deleted: ReadonlySet<string>,
  unchanged: ReadonlySet<string>,
  replaced: ReadonlySet<string>,
): string {
  if (deleted.has(path)) return 'deleted';
  if (unchanged.has(path)) return 'unchanged';
  if (outcome === 'skipped') return isLockfile(path) ? 'deployment regenerates' : 'not copied';
  if (outcome === 'conflict') return 'conflict';
  if (replaced.has(path)) return 'overwritten';
  if (outcome === 'merged') return 'merged';
  return 'changed';
}

export function CommitFileList({
  preflight,
  preflightLoading,
  excludedPaths,
  onAddExcludedPath,
  onRemoveExcludedPath,
}: CommitFileListProps) {
  const files = preflight?.files ?? [];
  const deleted = new Set(preflight?.deleted ?? []);
  const unchanged = new Set(preflight?.unchanged ?? []);
  const replaced = new Set(preflight?.replaced ?? []);
  const excluded = new Set([...excludedPaths, ...(preflight?.excluded ?? [])]);

  const toggleable = files.filter((file) => file.outcome !== 'skipped');
  const includedCount = toggleable.filter((file) => !excluded.has(file.path)).length;
  const excludedInList = files.filter(
    (file) => file.outcome !== 'skipped' && excluded.has(file.path),
  ).length;

  return (
    <div className="space-y-1.5">
      <div className="flex h-[29px] items-center justify-between gap-2">
        <h3 className="text-[11px] font-semibold tracking-wide text-muted uppercase">
          Files to commit
        </h3>
        {files.length > 0 ? (
          <span className="mono text-right text-muted tabular-nums">
            {includedCount} of {files.length} included
          </span>
        ) : null}
      </div>

      {preflightLoading ? (
        <p className="text-[11px] leading-relaxed text-muted">
          Checking which files this promotion will write…
        </p>
      ) : files.length === 0 ? (
        <p className="text-[11px] leading-relaxed text-muted">
          The preflight reported no files for this selection.
        </p>
      ) : (
        <>
          <p className="text-[11px] leading-relaxed text-muted">
            {excludedInList === 0
              ? 'All files included — uncheck any path to skip it.'
              : 'Unchecked paths are skipped.'}
          </p>
          <ul className="scroll-area max-h-[348px] divide-y divide-line overflow-hidden rounded-md border border-line bg-surface">
            {files.map((file) => {
              const skipped = file.outcome === 'skipped';
              const isExcluded = excluded.has(file.path);
              const included = !skipped && !isExcluded;
              const kind = changeKind(file.path, file.outcome, deleted, unchanged, replaced);
              return (
                <li key={file.path} className="flex h-[29px] items-center gap-2 px-2.5">
                  <Checkbox
                    checked={included}
                    disabled={skipped}
                    onChange={() =>
                      isExcluded ? onRemoveExcludedPath(file.path) : onAddExcludedPath(file.path)
                    }
                    label={
                      skipped
                        ? isLockfile(file.path)
                          ? `${file.path} is not copied by the promotion - the deployment regenerates it`
                          : `${file.path} is not copied by the promotion - the engine handles this path in the target`
                        : included
                          ? `Exclude ${file.path} from this promotion`
                          : `Include ${file.path} in this promotion`
                    }
                  />
                  <PathLabel path={file.path} className="flex-1 text-[11px]" muted={!included} />
                  <span className="shrink-0 text-[11px] text-muted">{kind}</span>
                </li>
              );
            })}
          </ul>
        </>
      )}
    </div>
  );
}
