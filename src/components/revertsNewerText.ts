import type { RevertsNewerWork } from '../api/types';

export function revertsNewerText(warn: RevertsNewerWork): string {
  if (!warn.sha) {
    return "main's copy came from a later subsystem commit. Promoting this path would move it back.";
  }
  return `main's copy came from ${warn.sha.slice(0, 7)} "${warn.subject}", a later subsystem commit. Promoting this path would move it back.`;
}
