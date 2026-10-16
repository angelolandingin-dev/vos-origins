// GitHub access layer.
//
// The promotion engine needs exactly four privileged-ish capabilities:
//   1. push the integration branch
//   2. create / find integration pull requests
//   3. check whether a target branch exists remotely
//   4. (optional) confirm caller authorization
//
// Real API/token/push/PR operations CANNOT be exercised in local fixture tests, so they are
// isolated behind this interface with THREE clearly-labelled transports:
//
//   * StubGitHubClient  (mode: 'stub')  — records calls, no network. Used by `node --test`.
//                                         Optionally performs a REAL `git push` to a local
//                                         bare remote so push mechanics stay testable.
//   * LocalGitHubClient (mode: 'local') — no GitHub API; returns "not found" for remote
//                                         branch checks and performs real local pushes.
//   * RealGitHubClient  (mode: 'real')  — real REST calls + real push. NOT covered by local
//                                         tests; requires a token and live GitHub. Labeled so.
//
// Nothing in the engine calls `gh`/the network directly; everything goes through this module.

import { spawnSync } from 'node:child_process';

export class GitHubError extends Error {
  constructor(message, { status = null, body = null } = {}) {
    super(message);
    this.name = 'GitHubError';
    this.status = status;
    this.body = body;
  }
}

function sanitizeUrl(url) {
  return String(url).replace(/x-access-token:[^@]+@/g, 'x-access-token:***@').replace(/\/\/([^:@/]+):([^@/]+)@/g, '//$1:***@');
}

export function authenticatedRemoteUrl(repoSlug, token) {
  if (!token) throw new GitHubError('A token is required to build an authenticated remote URL');
  return `https://x-access-token:${token}@github.com/${repoSlug}.git`;
}

class BaseClient {
  constructor({ pushTarget = null } = {}) {
    this.pushTarget = pushTarget;
    this.calls = [];
  }

  _record(name, payload) {
    this.calls.push({ name, at: new Date().toISOString(), payload });
  }

  /** Perform a real local/remote git push. Returns { pushed, remote, refspec }. */
  pushBranch({ repoDir, branch, remoteUrl }) {
    const url = remoteUrl || this.pushTarget;
    if (!url) {
      this._record('pushBranch', { branch, remoteUrl: null, pushed: false, reason: 'no push target configured (stub)' });
      return { pushed: false, reason: 'no push target configured' };
    }
    const result = spawnSync('git', ['-C', repoDir, 'push', url, `${branch}:refs/heads/${branch}`], {
      encoding: 'utf8',
      windowsHide: true,
    });
    if (result.status !== 0) {
      throw new GitHubError(`git push failed: ${sanitizeUrl(result.stderr || '')}`);
    }
    this._record('pushBranch', { branch, remoteUrl: sanitizeUrl(url), pushed: true });
    return { pushed: true, remote: sanitizeUrl(url) };
  }
}

export class StubGitHubClient extends BaseClient {
  constructor(options = {}) {
    super(options);
    this.mode = 'stub';
    // Pre-seeded remote branch names, if a test wants getBranch to succeed.
    this.knownBranches = options.knownBranches || null;
    this.openPrs = options.openPrs || [];
    this.authorized = options.authorized ?? true;
    this.counter = 100;
  }

  getBranch(_repoSlug, branch) {
    this._record('getBranch', { branch });
    if (this.knownBranches) return { exists: this.knownBranches.includes(branch), branch };
    return { exists: null, branch, note: 'stub: remote existence unknown; caller should rely on local refs' };
  }

  listOpenPullRequests(repoSlug, { head, base }) {
    this._record('listOpenPullRequests', { repoSlug, head, base });
    return this.openPrs.filter((pr) => (!base || pr.base === base) && (!head || pr.head === head));
  }

  createPullRequest(repoSlug, opts) {
    this.counter += 1;
    const number = this.counter;
    const pr = {
      number,
      url: `https://github.com/${repoSlug}/pull/${number}`,
      html_url: `https://github.com/${repoSlug}/pull/${number}`,
      state: 'open',
      base: opts.base,
      head: opts.head,
      title: opts.title,
      body: opts.body,
      draft: !!opts.draft,
      stubbed: true,
    };
    this._record('createPullRequest', pr);
    return pr;
  }

  isAuthorizedUser(_repoSlug, login) {
    this._record('isAuthorizedUser', { login });
    return this.authorized;
  }

  snapshot() {
    return { mode: this.mode, calls: this.calls };
  }
}

export class LocalGitHubClient extends BaseClient {
  constructor(options = {}) {
    super(options);
    this.mode = 'local';
  }

  getBranch() {
    this._record('getBranch', { note: 'local mode: no GitHub API' });
    return { exists: null };
  }

  listOpenPullRequests() {
    this._record('listOpenPullRequests', { note: 'local mode: no GitHub API' });
    return [];
  }

  createPullRequest(repoSlug, opts) {
    const pr = {
      number: null,
      url: null,
      base: opts.base,
      head: opts.head,
      title: opts.title,
      note: 'local mode: PR not created; body captured for inspection',
      body: opts.body,
      local: true,
    };
    this._record('createPullRequest', pr);
    return pr;
  }

  isAuthorizedUser() {
    this._record('isAuthorizedUser', { note: 'local mode: authorization not enforced' });
    return true;
  }
}

export class RealGitHubClient extends BaseClient {
  // NOTE: This class is real but is NOT exercised by the local test-suite. It requires a
  // real GITHUB_TOKEN and live GitHub access. It is provided so the workflow's write path is
  // implemented rather than faked.
  constructor(options = {}) {
    super(options);
    this.mode = 'real';
    this.token = options.token;
    this.apiBase = options.apiBase || 'https://api.github.com';
    this.fetchImpl = options.fetchImpl || globalThis.fetch;
    if (!this.token) throw new GitHubError('RealGitHubClient requires a token');
  }

  async _request(method, pathname, body) {
    const res = await this.fetchImpl(`${this.apiBase}${pathname}`, {
      method,
      headers: {
        Authorization: `Bearer ${this.token}`,
        Accept: 'application/vnd.github+json',
        'X-GitHub-Api-Version': '2022-11-28',
        'User-Agent': 'vos-subsystem-integration-engine',
        ...(body ? { 'Content-Type': 'application/json' } : {}),
      },
      body: body ? JSON.stringify(body) : undefined,
    });
    const text = await res.text();
    let parsed = null;
    try {
      parsed = text ? JSON.parse(text) : null;
    } catch {
      parsed = text;
    }
    if (!res.ok) {
      throw new GitHubError(`GitHub API ${method} ${pathname} failed (${res.status})`, { status: res.status, body: parsed });
    }
    return parsed;
  }

  async getBranch(repoSlug, branch) {
    this._record('getBranch', { branch });
    try {
      const data = await this._request('GET', `/repos/${repoSlug}/branches/${encodeURIComponent(branch)}`);
      return { exists: true, branch, sha: data.commit && data.commit.sha };
    } catch (err) {
      if (err.status === 404) return { exists: false, branch };
      throw err;
    }
  }

  async listOpenPullRequests(repoSlug, { head, base, perPage = 100 } = {}) {
    this._record('listOpenPullRequests', { repoSlug, head, base, perPage });
    const owner = repoSlug.split('/')[0];
    const all = [];
    // GitHub paginates at 30 by default; ask for 100 per page and walk every page so a stale
    // first page can never hide a matching open PR (§12).
    for (let page = 1; page <= 100; page += 1) {
      const params = new URLSearchParams({ state: 'open', base: base || 'master', per_page: String(perPage), page: String(page) });
      if (head) params.set('head', `${owner}:${head}`);
      const batch = await this._request('GET', `/repos/${repoSlug}/pulls?${params.toString()}`);
      const items = Array.isArray(batch) ? batch : [];
      all.push(...items);
      if (items.length < perPage) break;
    }
    return all;
  }

  async createPullRequest(repoSlug, opts) {
    this._record('createPullRequest', { repoSlug, base: opts.base, head: opts.head });
    return this._request('POST', `/repos/${repoSlug}/pulls`, {
      title: opts.title,
      head: opts.head,
      base: opts.base,
      body: opts.body,
      draft: !!opts.draft,
    });
  }

  async isAuthorizedUser(repoSlug, login) {
    this._record('isAuthorizedUser', { login });
    try {
      await this._request('GET', `/repos/${repoSlug}/collaborators/${encodeURIComponent(login)}`);
      return true;
    } catch (err) {
      if (err.status === 404) return false;
      throw err;
    }
  }
}

/**
 * @param {{mode:'stub'|'local'|'real', token?:string, pushTarget?:string, ...}} options
 */
export function createGitHubClient(options = {}) {
  const mode = options.mode || 'stub';
  if (mode === 'real') return new RealGitHubClient(options);
  if (mode === 'local') return new LocalGitHubClient(options);
  return new StubGitHubClient(options);
}

export { sanitizeUrl };
