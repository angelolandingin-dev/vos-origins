import type { RevertsNewerWork } from '../api/types';

import { revertsNewerText } from './revertsNewerText';

export function RevertsNewerLine({ warn }: { warn: RevertsNewerWork }) {
  return (
    <p className="text-[11px] leading-relaxed text-muted" title={warn.sha || undefined}>
      {revertsNewerText(warn)}
    </p>
  );
}
