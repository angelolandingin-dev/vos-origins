import { cx } from './ui/cx';

/**
 * One repository path, GitHub Desktop style: the directory part is muted and
 * only the basename keeps the normal foreground colour. Rendered as a single
 * truncating line (the design system's truncate-never-wrap rule), with the
 * full path in `title`. A path with no `/` is all basename.
 *
 * Built once and used at every site that lists a repo path so the surfaces
 * cannot drift. The mono/tabular treatment is kept from the old call sites.
 */
export interface PathLabelProps {
  path: string;
  /** Layout utilities for the caller's flex context (e.g. `flex-1`). */
  className?: string;
  /** Overrides the hover title; defaults to the full path. */
  title?: string;
  /** Render the whole path in the muted token (excluded/skipped rows). */
  muted?: boolean;
}

export function PathLabel({ path, className, title, muted = false }: PathLabelProps) {
  const slash = path.lastIndexOf('/');
  const directory = slash >= 0 ? path.slice(0, slash + 1) : '';
  const basename = slash >= 0 ? path.slice(slash + 1) : path;

  return (
    <span
      className={cx(
        'mono block min-w-0 truncate tabular-nums',
        muted ? 'text-muted' : 'text-fg',
        className,
      )}
      title={title ?? path}
    >
      {directory ? <span className={muted ? undefined : 'text-muted'}>{directory}</span> : null}
      <span>{basename}</span>
    </span>
  );
}
