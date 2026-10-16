import { useCallback, useEffect, useState } from 'react';

/** Theme mode: follow the OS until the user picks explicitly. Persisted locally. */

export type ThemeMode = 'system' | 'light' | 'dark';

const STORAGE_KEY = 'vos-origin.theme-mode';

function systemDark(): boolean {
  if (typeof window === 'undefined' || !window.matchMedia) return true;
  return window.matchMedia('(prefers-color-scheme: dark)').matches;
}

export function readThemeMode(): ThemeMode {
  try {
    const value = window.localStorage.getItem(STORAGE_KEY);
    if (value === 'light' || value === 'dark' || value === 'system') return value;
  } catch {
    /* Storage unavailable (private mode, permissions) — fall back to system. */
  }
  return 'system';
}

/** Apply the mode to the document root. `.dark`/`.light` win over the media query. */
export function applyTheme(mode: ThemeMode): void {
  const root = document.documentElement;
  const effective = mode === 'system' ? (systemDark() ? 'dark' : 'light') : mode;
  root.classList.toggle('dark', effective === 'dark');
  root.classList.toggle('light', effective === 'light');
  root.style.colorScheme = effective;
}

/** Call once before first render so the correct theme applies without a flash. */
export function initTheme(): ThemeMode {
  const mode = readThemeMode();
  applyTheme(mode);
  return mode;
}

export function useTheme(): {
  mode: ThemeMode;
  effective: 'light' | 'dark';
  setMode: (mode: ThemeMode) => void;
} {
  const [mode, setModeState] = useState<ThemeMode>(() => readThemeMode());
  const [followsSystem, setFollowsSystem] = useState<boolean>(() => systemDark());

  useEffect(() => {
    applyTheme(mode);
  }, [mode]);

  // While following the OS, re-apply when the OS theme changes.
  useEffect(() => {
    if (mode !== 'system') return;
    const query = window.matchMedia('(prefers-color-scheme: dark)');
    const onChange = (event: MediaQueryListEvent) => {
      setFollowsSystem(event.matches);
      applyTheme('system');
    };
    query.addEventListener('change', onChange);
    return () => query.removeEventListener('change', onChange);
  }, [mode]);

  const setMode = useCallback((next: ThemeMode) => {
    try {
      window.localStorage.setItem(STORAGE_KEY, next);
    } catch {
      /* Non-fatal: the theme still applies for this session. */
    }
    setModeState(next);
    applyTheme(next);
  }, []);

  return {
    mode,
    effective: mode === 'system' ? (followsSystem ? 'dark' : 'light') : mode,
    setMode,
  };
}
