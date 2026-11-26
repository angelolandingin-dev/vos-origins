import type { PromotionProgress } from '../api/types';

export interface ExecutionStage {
  id: string;
  label: string;
}

export function appendProgressStep(
  current: PromotionProgress[],
  next: PromotionProgress,
): PromotionProgress[] {
  const existing = current.findIndex((step) => step.stage === next.stage);
  if (existing >= 0) {
    const copy = current.slice();
    copy[existing] = next;
    return copy;
  }
  return [...current, next];
}
