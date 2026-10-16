import { useEffect } from 'react';

import { AppShell } from '../components/AppShell';
import { PromotionDialog } from '../components/PromotionDialog';
import { CommitBrowser } from '../pages/CommitBrowser';
import { Dashboard } from '../pages/Dashboard';
import { IntegrationHistory } from '../pages/IntegrationHistory';
import { PromoteCommits } from '../pages/PromoteCommits';
import { Settings } from '../pages/Settings';
import { Subsystems } from '../pages/Subsystems';
import { WorkflowRuns } from '../pages/WorkflowRuns';
import { useApp } from './context';

/** Shell + active screen + the app-level promotion dialog shared by multiple screens. */
export default function App() {
  const app = useApp();
  const { route, pushNotice } = app;
  const authStatus = app.auth.data;

  // A fixture-mode banner must be visible exactly once per session start.
  useEffect(() => {
    if (authStatus?.dataSource !== 'fixture') return;
    pushNotice({
      tone: 'info',
      title: 'Fixture data mode',
      message:
        'Registry, commit, workflow and PR records are clearly-marked fictitious samples. Statuses are still computed by the real branch-scoped algorithm from provenance events, open PRs and runs. GitHub CLI detection is live.',
    });
  }, [authStatus?.dataSource, pushNotice]);

  const handleSwitchAccount = () => {
    app.pushNotice({
      tone: 'info',
      title: 'Switch GitHub account in your terminal',
      message:
        'The GitHub CLI owns credentials, so the desktop app never handles them. Run `gh auth logout` then `gh auth login` (or `gh auth switch`), then use Re-check GitHub CLI status.',
    });
  };

  return (
    <>
      <AppShell
        route={route}
        onNavigate={app.navigate}
        auth={app.auth}
        onRefreshAuth={app.refreshAuth}
        onSwitchAccount={handleSwitchAccount}
        targetBranch={app.targetBranch}
        selectedSubsystemName={app.selectedSubsystem?.name ?? null}
        notice={app.notice}
        onDismissNotice={app.dismissNotice}
      >
        {route === 'dashboard' ? <Dashboard /> : null}
        {route === 'subsystems' ? <Subsystems /> : null}
        {route === 'commit-browser' ? <CommitBrowser /> : null}
        {route === 'promote-commits' ? <PromoteCommits /> : null}
        {route === 'workflow-runs' ? <WorkflowRuns /> : null}
        {route === 'integration-history' ? <IntegrationHistory /> : null}
        {route === 'settings' ? <Settings /> : null}
      </AppShell>

      <PromotionDialog
        open={app.promotion.open}
        onClose={app.closePromotionDialog}
        subsystem={app.selectedSubsystem}
        sourceShas={app.promotion.shas}
        onRemoveSha={app.removePromotionSha}
        commits={app.promotion.commits}
        commitsLoading={app.promotion.commits.length === 0}
        targetBranches={app.targetBranches.data}
        targetBranch={app.targetBranch}
        onTargetBranchChange={app.setTargetBranch}
        mode={app.promotion.mode}
        onModeChange={app.setPromotionMode}
        baseSha={app.promotion.baseSha}
        onBaseShaChange={app.setPromotionBaseSha}
        dependencies={app.promotion.dependencies}
        dependenciesLoading={app.promotion.dependenciesLoading}
        dispatching={app.promotion.dispatching}
        error={app.promotion.error}
        result={app.promotion.result}
        onDispatch={app.dispatchPromotion}
        onOpenExternal={app.openExternal}
        onViewRuns={() => {
          app.closePromotionDialog();
          app.navigate('workflow-runs');
        }}
      />
    </>
  );
}
