import { useEffect, useState } from 'react';

function readMatches(query: string): boolean {
  if (typeof window === 'undefined' || typeof window.matchMedia !== 'function') return false;
  return window.matchMedia(query).matches;
}

export function useMediaQuery(query: string): boolean {
  const [snapshot, setSnapshot] = useState(() => ({ query, matches: readMatches(query) }));

  useEffect(() => {
    if (typeof window === 'undefined' || typeof window.matchMedia !== 'function') return;
    const list = window.matchMedia(query);
    const listener = (event: MediaQueryListEvent) => setSnapshot({ query, matches: event.matches });
    list.addEventListener('change', listener);
    return () => list.removeEventListener('change', listener);
  }, [query]);

  // If the query changed since the last snapshot, report the current value for
  // this render without writing state from an effect.
  return snapshot.query === query ? snapshot.matches : readMatches(query);
}
