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
