// Machine-readable change summary / diagnostics report.
//
// The report is ephemeral (stdout + optional artifact file + GitHub step summary). It is
// NEVER committed and is NOT a ledger — successful integration state lives only in Git
// commit trailers.

import fs from 'node:fs';

export function createReport(meta = {}) {
  return {
    engine: 'vos-subsystem-promotion',
    schemaVersion: 1,
    startedAt: new Date().toISOString(),
    ...meta,
    plan: [],
    findings: [],
    changes: {
      applied: [],
      merged: [],
      deleted: [],
      regenerated: [],
      mainOwnedSkipped: [],
      secretExcluded: [],
      explicitReview: [],
      rejected: [],
    },
    dependencies: null,
    provenance: null,
    result: null,
    blockers: [],
    warnings: [],
  };
}

export function addFinding(report, finding) {
  report.findings.push(finding);
  if (finding.severity === 'blocker') report.blockers.push(finding);
  else if (finding.severity === 'warning') report.warnings.push(finding);
}

export function blocker(report, code, message, details = {}) {
  addFinding(report, { severity: 'blocker', code, message, details });
}

export function warn(report, code, message, details = {}) {
  addFinding(report, { severity: 'warning', code, message, details });
}

export function finalize(report) {
  report.finishedAt = new Date().toISOString();
  report.ok = report.blockers.length === 0;
  if (report.result === null) {
    report.result = report.ok ? 'ready' : 'blocked';
  }
  return report;
}

export function writeReport(report, { filePath = null, stepSummaryPath = null } = {}) {
  const json = `${JSON.stringify(report, null, 2)}\n`;
  process.stdout.write(`\n${'='.repeat(78)}\nENGINE REPORT\n${'='.repeat(78)}\n${json}`);
  if (filePath) {
    fs.writeFileSync(filePath, json);
  }
  if (stepSummaryPath) {
    fs.appendFileSync(stepSummaryPath, renderMarkdown(report));
  }
}

export function renderMarkdown(report) {
  const lines = [];
  lines.push(`## Subsystem promotion: ${report.result || 'unknown'}`);
  lines.push('');
  if (report.request) {
    lines.push(`- Subsystem: \`${report.request.subsystem}\``);
    lines.push(`- Mode: \`${report.request.mode}\``);
    lines.push(`- Target branch: \`${report.request.targetBranch}\``);
    lines.push(`- Commits: ${(report.request.commits || []).map((c) => `\`${c.slice(0, 12)}\``).join(', ')}`);
  }
  lines.push('');
  if (report.blockers.length) {
    lines.push('### Blockers');
    for (const b of report.blockers) lines.push(`- **${b.code}**: ${b.message}`);
    lines.push('');
  }
  if (report.warnings.length) {
    lines.push('### Warnings');
    for (const w of report.warnings) lines.push(`- ${w.code}: ${w.message}`);
    lines.push('');
  }
  if (report.dependencies) {
    lines.push('### Dependencies');
    lines.push(`- added: ${report.dependencies.added.map((d) => d.name).join(', ') || '—'}`);
    lines.push(`- updated: ${report.dependencies.updated.map((d) => `${d.name} (${d.from} → ${d.to})`).join(', ') || '—'}`);
    lines.push(`- removed: ${report.dependencies.removed.map((d) => d.name).join(', ') || '—'}`);
    lines.push('');
  }
  if (report.provenance) {
    lines.push('### Provenance');
    lines.push('```');
    lines.push(report.provenance.message.trim());
    lines.push('```');
    lines.push('');
  }
  return `${lines.join('\n')}\n`;
}
