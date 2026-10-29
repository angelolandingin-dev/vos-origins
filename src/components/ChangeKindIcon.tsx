import addedIcon from '../assets/added.svg';
import deletedIcon from '../assets/deleted.svg';
import modifiedIcon from '../assets/modified.svg';
import renamedIcon from '../assets/renamed.svg';
import type { FileChangeKind } from '../api/types';

/** GitHub Desktop file-change glyphs, keyed by the change each one describes. */
const ICON_SRC: Record<FileChangeKind, string> = {
  added: addedIcon,
  modified: modifiedIcon,
  deleted: deletedIcon,
  renamed: renamedIcon,
};

/**
 * A file's change kind as a glyph. The alt/title carry the status so the icon
 * stays readable without sight; the fixed 16px box never moves a 29px row.
 * `label` overrides the accessible name when the caller's word for the status
 * differs from the glyph's own change (the preflight's `changed` reads as the
 * modified glyph).
 */
export function ChangeKindIcon({
  change,
  label = change,
}: {
  change: FileChangeKind;
  label?: string;
}) {
  return <img src={ICON_SRC[change]} alt={label} title={label} className="h-4 w-4 shrink-0" />;
}
