import { cx } from './ui/cx';

export interface PathLabelProps {
  path: string;
  className?: string;
  /** Overrides the hover title; defaults to the full path. */
  title?: string;
  /** Render the whole path in the muted token (excluded/skipped rows). */
  muted?: boolean;
}

export function PathLabel({ path, className, title, muted = false }: PathLabelProps) {
  const slash = path.lastIndexOf('/');
  const directory = slash >= 0 ? path.slice(0, slash) : '';
  const basename = slash >= 0 ? path.slice(slash + 1) : path;

  return (
    <span
      className={cx('mono flex min-w-0 tabular-nums', muted ? 'text-muted' : 'text-fg', className)}
      title={title ?? path}
    >
      {directory ? (
        <span className={cx('min-w-0 truncate', muted ? undefined : 'text-muted')}>
          {directory}
        </span>
      ) : null}
      {/* The separator rides with the basename in a non-shrinking box, so the directory yields first and `/` is never cut. */}
      <span className="flex min-w-0 max-w-full shrink-0">
        {directory ? (
          <span className={cx('shrink-0', muted ? undefined : 'text-muted')}>/</span>
        ) : null}
        <span className="min-w-0 truncate">{basename}</span>
      </span>
    </span>
  );
}
