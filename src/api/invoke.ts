import { invoke } from '@tauri-apps/api/core';

import type {
  ApiErrorCode,
  ApiErrorPayload,
  AuthStatus,
  CacheStats,
  CheckoutClear,
  CheckoutRefresh,
  CommitDetails,
  CommitDiff,
  CommitPage,
  CommitQuery,
  IntegrationEventAudit,
  IntegrationHistory,
  IntegrationPr,
  IntegrationStatus,
  PromotionDispatchResult,
  PromotionPreflight,
  PromotionRequest,
  RegistrationResult,
  RegistryConfig,
  RegistrySnapshot,
  SubsystemDiff,
  SubsystemRegistrationRequest,
  SubsystemSummary,
  SubsystemSummaryCounts,
  TargetBranchList,
} from './types';

/** Typed entry point for the Rust backend (spec §9.2). Rust owns business rules; failures normalize to `ApiError` with a remedy. */

export class ApiError extends Error {
  readonly code: ApiErrorCode;
  readonly remedy: string | null;
  readonly details: string | null;
  readonly retryable: boolean;

  constructor(
    code: ApiErrorCode,
    message: string,
    remedy: string | null = null,
    details: string | null = null,
    retryable = false,
  ) {
    super(message);
    this.name = 'ApiError';
    this.code = code;
    this.remedy = remedy;
    this.details = details;
    this.retryable = retryable;
  }
}

export function isTauriRuntime(): boolean {
  if (typeof window === 'undefined') return false;
  return '__TAURI_INTERNALS__' in window;
}

const BACKEND_UNAVAILABLE = new ApiError(
  'backend_unavailable',
  'The Rust desktop backend is not reachable from this web context.',
  'Run the desktop app with `npm run tauri dev` (or install the packaged build). The UI deliberately shows no fabricated data when the backend is absent.',
);

function isApiErrorPayload(value: unknown): value is ApiErrorPayload {
  if (typeof value !== 'object' || value === null) return false;
  const candidate = value as Record<string, unknown>;
  return typeof candidate['code'] === 'string' && typeof candidate['message'] === 'string';
}

function normalizeError(raw: unknown): ApiError {
  if (raw instanceof ApiError) return raw;

  if (isApiErrorPayload(raw)) {
    return new ApiError(
      raw.code as ApiErrorCode,
      raw.message,
      raw.remedy,
      raw.details,
      raw.retryable,
    );
  }

  if (typeof raw === 'string') {
    // Tauri serializes command errors as JSON strings via the plain `invoke` error channel.
    try {
      const parsed: unknown = JSON.parse(raw);
      if (isApiErrorPayload(parsed)) return normalizeError(parsed);
    } catch {
      /* not JSON */
    }
    return new ApiError('internal', raw, null, null, false);
  }

  if (raw instanceof Error) {
    return new ApiError('internal', raw.message, null, null, false);
  }

  return new ApiError('internal', 'An unknown backend error occurred.', null, null, false);
}

export function asApiError(error: unknown): ApiError {
  return normalizeError(error);
}

async function call<T>(command: string, args?: Record<string, unknown>): Promise<T> {
  if (!isTauriRuntime()) {
    throw BACKEND_UNAVAILABLE;
  }
  try {
    return await invoke<T>(command, args);
  } catch (raw) {
    throw normalizeError(raw);
  }
}

/** Typed wrapper around every command in spec section 9.2. */
export const api = {
  getAuthStatus: () => call<AuthStatus>('get_auth_status'),

  listSubsystems: () => call<SubsystemSummary[]>('list_subsystems'),

  getSubsystemSummaries: () => call<SubsystemSummaryCounts[]>('get_subsystem_summaries'),

  listTargetBranches: () => call<TargetBranchList>('list_target_branches'),

  getSubsystemCommits: (query: CommitQuery) => call<CommitPage>('get_subsystem_commits', { query }),

  getCommitDetails: (subsystemId: string, sha: string, targetBranch: string) =>
    call<CommitDetails>('get_commit_details', { subsystemId, sha, targetBranch }),

  getCommitDiff: (
    subsystemId: string,
    sha: string,
    filePath: string | null,
    targetBranch: string,
  ) => call<CommitDiff>('get_commit_diff', { subsystemId, sha, filePath, targetBranch }),

  getIntegrationStatuses: (targetBranch: string, subsystemId: string | null, shas: string[]) =>
    call<IntegrationStatus[]>('get_integration_statuses', { targetBranch, subsystemId, shas }),

  getSubsystemDiff: (subsystemId: string, targetBranch: string) =>
    call<SubsystemDiff>('get_subsystem_diff', { subsystemId, targetBranch }),

  submitSubsystemRegistration: (request: SubsystemRegistrationRequest) =>
    call<RegistrationResult>('submit_subsystem_registration', { request }),

  dispatchPromotion: (request: PromotionRequest) =>
    call<PromotionDispatchResult>('dispatch_promotion', {
      request: { ...request, delivery: request.delivery ?? 'local_direct' },
    }),

  preflightPromotion: (request: PromotionRequest) =>
    call<PromotionPreflight>('preflight_promotion', { request }),

  listIntegrationPrs: (targetBranch: string, subsystemId: string | null) =>
    call<IntegrationPr[]>('list_integration_prs', { targetBranch, subsystemId }),

  listIntegrationHistory: (
    targetBranch: string | null,
    subsystemId: string | null,
    limit: number,
  ) => call<IntegrationHistory>('list_integration_history', { targetBranch, subsystemId, limit }),

  getIntegrationEvent: (args: {
    targetBranch: string;
    subsystemId: string;
    mainCommitSha: string;
  }) => call<IntegrationEventAudit>('get_integration_event', { ...args }),

  getRegistry: () => call<RegistrySnapshot>('get_registry'),

  saveRegistry: (config: RegistryConfig) => call<RegistrySnapshot>('save_registry', { config }),

  refreshCheckouts: (subsystemId: string | null) =>
    call<CheckoutRefresh>('refresh_checkouts', { subsystemId }),

  clearCheckouts: () => call<CheckoutClear>('clear_checkouts'),

  getCacheStats: () => call<CacheStats>('get_cache_stats'),

  clearCache: () => call<CacheStats>('clear_cache'),

  openExternalUrl: (url: string) => call<void>('open_external_url', { url }),
};

export type Api = typeof api;
