import type { RevertsNewerWork } from '../api/types';

/**
 * The "reverts newer work" warning names the newer commit so the operator can
 * verify it instead of taking it on trust. Factual tone: a statement about
 * what the write will do, not a scolding. Warning only — un-excluding the
 * path (or proceeding with it included) is how the operator does it on
 * purpose; there is no acknowledgement gate.
 */
export function revertsNewerText(warn: RevertsNewerWork): string {
  if (!warn.sha) {
    return "main's copy came from a later subsystem commit. Promoting this path would move it back.";
  }
  return `main's copy came from ${warn.sha.slice(0, 7)} "${warn.subject}", a later subsystem commit. Promoting this path would move it back.`;
}
