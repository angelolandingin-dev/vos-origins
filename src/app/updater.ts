import { getVersion } from '@tauri-apps/api/app';
import { relaunch } from '@tauri-apps/plugin-process';
import { check, type Update } from '@tauri-apps/plugin-updater';

import { isTauriRuntime } from '../api/invoke';

export interface UpdateInfo {
  update: Update;
  version: string;
  currentVersion: string;
  notes: string | null;
}

/** The running build's version, or null outside the desktop runtime. */
export async function currentVersion(): Promise<string | null> {
  if (!isTauriRuntime()) return null;
  try {
    return await getVersion();
  } catch {
    return null;
  }
}

/**
 * Asks the configured endpoint for a newer release. Returns null when the build is
 * current, when the app runs in a browser preview, or when the network is down —
 * an unreachable update server must never stop the tool from being used.
 */
export async function findUpdate(): Promise<UpdateInfo | null> {
  if (!isTauriRuntime()) return null;
  let update: Update | null;
  try {
    update = await check();
  } catch {
    return null;
  }
  if (!update) return null;
  return {
    update,
    version: update.version,
    currentVersion: update.currentVersion,
    notes: update.body ?? null,
  };
}

/**
 * Downloads and installs the release, then restarts into it. Windows runs the NSIS
 * installer in passive mode, so the user sees a progress window and no prompts.
 */
export async function installUpdate(
  update: Update,
  onProgress: (percent: number | null) => void,
): Promise<void> {
  let downloaded = 0;
  let total = 0;
  await update.downloadAndInstall((event) => {
    if (event.event === 'Started') {
      total = event.data.contentLength ?? 0;
      onProgress(0);
    } else if (event.event === 'Progress') {
      downloaded += event.data.chunkLength;
      onProgress(total > 0 ? Math.round((downloaded / total) * 100) : null);
    } else {
      onProgress(100);
    }
  });
  await relaunch();
}
