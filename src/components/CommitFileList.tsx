import { useRef } from 'react';
import type { PromotionPreflight } from '../api/types';
import { PathLabel } from './PathLabel';
import { Button, Checkbox } from './ui/primitives';

export interface CommitFileListProps {
  preflight: PromotionPreflight | null;
  preflightLoading: boolean;
  excludedPaths: string[];
  onSetExcludedPaths: (paths: string[]) => void;
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
  onSetExcludedPaths,
}: CommitFileListProps) {
  const anchorRef = useRef<number | null>(null);
  const files = preflight?.files ?? [];
  const deleted = new Set(preflight?.deleted ?? []);
  const unchanged = new Set(preflight?.unchanged ?? []);
  const replaced = new Set(preflight?.replaced ?? []);
  const excluded = new Set([...excludedPaths, ...(preflight?.excluded ?? [])]);

  const toggleable = files.filter((file) => file.outcome !== 'skipped');
  const includedCount = toggleable.filter((file) => !excluded.has(file.path)).length;
  const excludedInList = files.filter((file) => excluded.has(file.path)).length;
  const canExclude = toggleable.some((file) => !excluded.has(file.path));
  const canInclude = toggleable.some((file) => excludedPaths.includes(file.path));

  const excludeAll = () => {
    const paths = toggleable.filter((file) => !excluded.has(file.path)).map((file) => file.path);
    onSetExcludedPaths(Array.from(new Set([...excludedPaths, ...paths])));
  };

  const includeAll = () => {
    const restored = new Set(
      toggleable.filter((file) => excludedPaths.includes(file.path)).map((file) => file.path),
    );
    onSetExcludedPaths(excludedPaths.filter((path) => !restored.has(path)));
  };

  const toggleAt = (index: number, shiftKey: boolean) => {
    const file = files[index];
    if (!file) return;
    const include = excluded.has(file.path);
    const anchor = anchorRef.current;
    anchorRef.current = index;
    const [from, to] =
      shiftKey && anchor !== null
        ? [Math.min(anchor, index), Math.max(anchor, index)]
        : [index, index];
    const paths = files
      .slice(from, to + 1)
      .filter((candidate) => candidate.outcome !== 'skipped')
      .map((candidate) => candidate.path);
    if (paths.length === 0) return;
    onSetExcludedPaths(
      include
        ? excludedPaths.filter((path) => !paths.includes(path))
        : Array.from(new Set([...excludedPaths, ...paths])),
    );
  };

  return (
    <div className="space-y-1.5">
      <div className="flex h-[29px] items-center justify-between gap-2">
        <h3 className="text-[11px] font-semibold tracking-wide text-muted uppercase">
          Files to commit
        </h3>
        {files.length > 0 ? (
          <div className="flex items-center gap-2">
            {canExclude ? (
              <Button
                size="sm"
                variant="secondary"
                onClick={excludeAll}
                aria-label="Exclude every excludable file from this promotion"
              >
                Exclude all
              </Button>
            ) : null}
            {canInclude ? (
              <Button
                size="sm"
                variant="secondary"
                onClick={includeAll}
                aria-label="Include every excludable file in this promotion"
              >
                Include all
              </Button>
            ) : null}
            <span className="mono text-right text-muted tabular-nums">
              {includedCount} of {files.length} included
            </span>
          </div>
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
              ? 'All files included — uncheck any path to skip it, or shift-click to skip a range.'
              : 'Unchecked paths are skipped.'}
          </p>
          <ul className="scroll-area max-h-[348px] divide-y divide-line overflow-hidden rounded-md border border-line bg-surface">
            {files.map((file, index) => {
              const skipped = file.outcome === 'skipped';
              const isExcluded = excluded.has(file.path);
              const operatorExcluded = excludedPaths.includes(file.path);
              const included = !skipped && !isExcluded;
              const kind =
                skipped && isExcluded && !isLockfile(file.path)
                  ? 'excluded'
                  : changeKind(file.path, file.outcome, deleted, unchanged, replaced);
              return (
                <li key={file.path} className="flex h-[29px] items-center gap-2 px-2.5">
                  <Checkbox
                    checked={included}
                    disabled={skipped && !operatorExcluded}
                    onChange={(event) =>
                      toggleAt(index, (event.nativeEvent as MouseEvent).shiftKey)
                    }
                    label={
                      skipped && !operatorExcluded
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
